// RELAY 0.2.2-draft client: HELLO / transcript AUTH / WELCOME, OPS push
// honoring advertised limits, merkle-walk catch-up. Same wire as
// zerodb_storage::relay_client. MUST NOT import zerodb-napi.

import { encode, bytesToHex, hexToBytes } from '../models/cbor.mjs'
import {
  RELAY_CAPS,
  DEFAULT_LIMITS,
  MSG_HELLO,
  MSG_CHALLENGE,
  MSG_AUTH,
  MSG_WELCOME,
  MSG_SYNC_REQUEST,
  MSG_SYNC_RESPONSE,
  MSG_DELTA_REQUEST,
  MSG_DELTA_BATCH,
  MSG_MERKLE_NODE_REQUEST,
  MSG_MERKLE_NODE_RESPONSE,
  MSG_MERKLE_LEAF_REQUEST,
  MSG_MERKLE_LEAF_RESPONSE,
  MSG_OPS,
  MSG_OP_ACK,
  MSG_ERROR,
  encodeEnvelope,
  decodeEnvelope,
  authTranscript,
  signAuth,
  jsonToTagged,
} from '../models/relay.mjs'
import { merkleRootOnce, buildTreeAligned, emptyLeaf } from '../models/merkle.mjs'

const MERKLE_FORMAT_VERSION = 1
const BUCKET_WIDTH_MS = 60_000
const RATE_WINDOW_MS = 1100

export function welcomeLimits(welcome) {
  const limits = (welcome && welcome.limits) || {}
  const num = (v, fallback) => (typeof v === 'number' && v > 0 ? v : fallback)
  return {
    max_payload_bytes: num(limits.max_payload_bytes, DEFAULT_LIMITS.max_payload_bytes),
    max_batch_ops: num(limits.max_batch_ops, DEFAULT_LIMITS.max_batch_ops),
    max_batch_bytes: num(limits.max_batch_bytes, DEFAULT_LIMITS.max_batch_bytes),
    ops_per_second: num(limits.ops_per_second, DEFAULT_LIMITS.ops_per_second),
    bytes_per_second: num(limits.bytes_per_second, DEFAULT_LIMITS.bytes_per_second),
  }
}

function cborArrayHeaderLen(n) {
  if (n <= 23) return 1
  if (n <= 255) return 2
  if (n <= 65535) return 3
  return 5
}

export function encodeRelayOp(wire) {
  return encode(
    jsonToTagged({
      op_id: wire.id,
      author: wire.author,
      physical_ms: wire.ts.p,
      logical: wire.ts.l,
      wire: JSON.stringify(wire),
    }),
  )
}

export function splitOpsBatches(ds, ops, maxOps, maxBytes, maxPayload) {
  const capOps = Math.max(1, maxOps)
  const emptyLen = encodeEnvelope(MSG_OPS, 0, { datastore: ds, operations: [] }).length
  const frameLen = (n, bytes) => emptyLen - 1 + cborArrayHeaderLen(n) + bytes
  const sized = []
  for (const op of ops) {
    const n = op.length
    if (n > maxPayload) throw new Error('single op exceeds max_payload_bytes')
    sized.push({ op, n })
  }
  const out = []
  let cur = []
  let curBytes = 0
  for (const { op, n } of sized) {
    const nextN = cur.length + 1
    const nextBytes = curBytes + n
    const over = nextN > capOps || frameLen(nextN, nextBytes) > maxBytes
    if (over && cur.length === 0) throw new Error('single op exceeds WELCOME batch limits')
    if (over) {
      out.push(cur)
      cur = []
      curBytes = 0
      if (frameLen(1, n) > maxBytes) throw new Error('single op exceeds max_batch_bytes')
    }
    cur.push(op)
    curBytes += n
  }
  if (cur.length) out.push(cur)
  return out
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

async function paceOpsWindow(window, nextOps, nextBytes, maxOps, maxBytes) {
  const now = Date.now()
  if (now - window.start >= RATE_WINDOW_MS) {
    window.ops = 0
    window.bytes = 0
    window.start = now
  }
  if (window.ops + nextOps > maxOps || window.bytes + nextBytes > maxBytes) {
    const remain = RATE_WINDOW_MS - (Date.now() - window.start)
    if (remain > 0) await sleep(remain)
    window.ops = 0
    window.bytes = 0
    window.start = Date.now()
  }
}

function firstReply(replies) {
  if (!replies || replies.length === 0) throw new Error('empty relay reply')
  return replies[0]
}

function expectType(frame, want, name) {
  const env = decodeEnvelope(frame)
  if (env.type === MSG_ERROR) {
    throw new Error(`${name} got ERROR: ${JSON.stringify(env.payload)}`)
  }
  if (env.type !== want) {
    throw new Error(`expected ${name}, got type ${env.type}`)
  }
  return env
}

function merkleOpsOf(wires) {
  return wires.map((w) => ({
    op_id: hexToBytes(w.id),
    author: hexToBytes(w.author),
    physical_ms: w.ts.p,
    logical: w.ts.l,
  }))
}

function acceptedRoot(wires) {
  if (wires.length === 0) return emptyLeaf()
  return merkleRootOnce(merkleOpsOf(wires))
}

function frontierFromOps(ops, ds) {
  const tips = new Map()
  for (const w of ops) {
    if (w.ds !== ds) continue
    const tip = { p: w.ts.p, l: w.ts.l, id: w.id }
    const prev = tips.get(w.author)
    if (!prev || prev.p < tip.p || (prev.p === tip.p && prev.l < tip.l) || (prev.p === tip.p && prev.l === tip.l && prev.id < tip.id)) {
      tips.set(w.author, tip)
    }
  }
  const frontier = {}
  for (const [author, tip] of tips) {
    frontier[author] = { op_id: tip.id, physical_ms: tip.p, logical: tip.l }
  }
  return { frontier, epoch: 0 }
}

function catchupWireOp(op) {
  if (!op || typeof op.wire !== 'string') return null
  try {
    return JSON.parse(op.wire)
  } catch {
    return null
  }
}

function collectWireOps(payload, incoming) {
  let skipped = 0
  const ops = (payload && payload.operations) || []
  for (const op of ops) {
    const wire = catchupWireOp(op)
    if (wire) incoming.push(wire)
    else skipped += 1
  }
  return skipped
}

function parseBucketIndices(payload) {
  if (payload.merkle_format_version == null) return null
  if (payload.merkle_format_version !== MERKLE_FORMAT_VERSION) {
    throw new Error('unsupported merkle_format_version')
  }
  if (payload.bucket_width_ms !== BUCKET_WIDTH_MS) {
    throw new Error('unsupported merkle bucket_width_ms')
  }
  const indices = payload.bucket_indices
  if (!Array.isArray(indices)) throw new Error('missing bucket_indices')
  if (indices.length > 65536) throw new Error('merkle bucket manifest exceeds limit')
  for (let i = 1; i < indices.length; i++) {
    if (indices[i - 1] >= indices[i]) throw new Error('merkle bucket manifest must be strictly sorted')
  }
  return indices
}

function eqHex(a, b) {
  return String(a).toLowerCase() === String(b).toLowerCase()
}

async function walkRemote(handle, ds, local, level, index, state) {
  if (level === 0) {
    const frame = encodeEnvelope(MSG_MERKLE_LEAF_REQUEST, state.requestId, {
      datastore: ds,
      leaf_index: index,
    })
    state.requestId += 1
    const env = expectType(firstReply(await handle(frame)), MSG_MERKLE_LEAF_RESPONSE, 'MERKLE_LEAF_RESPONSE')
    const localIds = new Set(
      (local.leaves[index] ? local.leaves[index].op_ids : []).map((id) => bytesToHex(id)),
    )
    for (const id of env.payload.op_ids || []) {
      if (!localIds.has(id)) state.missing.add(id)
    }
    state.summary.merkle_leaves += 1
    return
  }

  const frame = encodeEnvelope(MSG_MERKLE_NODE_REQUEST, state.requestId, {
    datastore: ds,
    level,
    index,
  })
  state.requestId += 1
  const env = expectType(firstReply(await handle(frame)), MSG_MERKLE_NODE_RESPONSE, 'MERKLE_NODE_RESPONSE')
  const remoteLeft = env.payload.left
  const remoteRight = env.payload.right
  const childLevel = local.levels[level - 1]
  const localLeft = childLevel ? bytesToHex(childLevel[index * 2]) : bytesToHex(emptyLeaf())
  const localRight = childLevel ? bytesToHex(childLevel[index * 2 + 1]) : bytesToHex(emptyLeaf())
  state.summary.merkle_nodes += 1
  if (!eqHex(remoteLeft, localLeft)) {
    await walkRemote(handle, ds, local, level - 1, index * 2, state)
  }
  if (!eqHex(remoteRight, localRight)) {
    await walkRemote(handle, ds, local, level - 1, index * 2 + 1, state)
  }
}

async function handshake(store, handle) {
  const hello = encodeEnvelope(MSG_HELLO, 1, {
    peer_id: store.authorHex,
    public_key: store.pkHex,
    protocol_version: 1,
    capabilities: RELAY_CAPS.slice(),
  })
  const challenge = expectType(firstReply(await handle(hello)), MSG_CHALLENGE, 'CHALLENGE')
  const nonce = challenge.payload.nonce
  const transcript = authTranscript(store.author, store.pk, 1, RELAY_CAPS, hexToBytes(nonce))
  const sig = signAuth(store.seed, transcript)
  const auth = encodeEnvelope(MSG_AUTH, 2, { signature: bytesToHex(sig) })
  const welcome = expectType(firstReply(await handle(auth)), MSG_WELCOME, 'WELCOME')
  return welcomeLimits(welcome.payload)
}

/**
 * Push local ops and merkle-walk catch-up. `handle(frame) => Promise<Uint8Array[]>`.
 */
export async function sync(store, joinDs, handle) {
  const summary = {
    sent: 0,
    ack_accepted: 0,
    ack_duplicate: 0,
    ack_rejected: 0,
    received: 0,
    applied: 0,
    skipped: 0,
    merkle_nodes: 0,
    merkle_leaves: 0,
  }
  const limits = await handshake(store, handle)
  const ds = joinDs || store.datastoreIdHex()
  const toSend = store.exportOps(ds)
  let requestId = 3

  if (toSend.length > 0) {
    summary.sent = toSend.length
    const encoded = toSend.map(encodeRelayOp)
    const batches = splitOpsBatches(
      ds,
      encoded,
      limits.max_batch_ops,
      limits.max_batch_bytes,
      limits.max_payload_bytes,
    )
    let offset = 0
    const window = { ops: 0, bytes: 0, start: Date.now() }
    for (const batch of batches) {
      const wires = toSend.slice(offset, offset + batch.length)
      offset += batch.length
      const batchOps = batch.length
      const batchBytes = batch.reduce((n, op) => n + op.length, 0)
      await paceOpsWindow(window, batchOps, batchBytes, limits.ops_per_second, limits.bytes_per_second)
      const frame = encodeEnvelope(MSG_OPS, requestId, {
        datastore: ds,
        operations: wires.map((w) => ({
          op_id: w.id,
          author: w.author,
          physical_ms: w.ts.p,
          logical: w.ts.l,
          wire: JSON.stringify(w),
        })),
      })
      requestId += 1
      const ack = expectType(firstReply(await handle(frame)), MSG_OP_ACK, 'OP_ACK')
      for (const o of ack.payload.outcomes || []) {
        if (o.outcome === 'ACCEPT') summary.ack_accepted += 1
        else if (o.outcome === 'DUPLICATE') summary.ack_duplicate += 1
        else if (o.outcome === 'REJECT') summary.ack_rejected += 1
      }
      window.ops += batchOps
      window.bytes += batchBytes
    }
  }

  const syncReq = encodeEnvelope(MSG_SYNC_REQUEST, requestId, {
    datastore: ds,
    accepted_root: bytesToHex(acceptedRoot(toSend)),
    cursor: frontierFromOps(store.ops, ds),
  })
  const replies = await handle(syncReq)
  const incoming = []
  let catchupSkipped = 0
  let syncPayload = null
  for (const frame of replies) {
    const env = decodeEnvelope(frame)
    if (env.type === MSG_SYNC_RESPONSE) syncPayload = env.payload
    else if (env.type === MSG_OPS) catchupSkipped += collectWireOps(env.payload, incoming)
    else if (env.type === MSG_ERROR) throw new Error(`SYNC error: ${JSON.stringify(env.payload)}`)
  }
  if (!syncPayload) throw new Error('expected SYNC_RESPONSE')

  const bucketIndices = parseBucketIndices(syncPayload)
  if (bucketIndices) {
    const remoteRoot = syncPayload.validated_root
    const localTree = buildTreeAligned(merkleOpsOf(toSend), bucketIndices)
    if (!eqHex(bytesToHex(localTree.levels[localTree.levels.length - 1][0]), remoteRoot)) {
      const state = { requestId, missing: new Set(), summary }
      const rootLevel = localTree.levels.length - 1
      await walkRemote(handle, ds, localTree, rootLevel, 0, state)
      requestId = state.requestId
      if (state.missing.size > 0) {
        const delta = encodeEnvelope(MSG_DELTA_REQUEST, requestId, {
          datastore: ds,
          op_ids: [...state.missing],
        })
        for (const frame of await handle(delta)) {
          const env = decodeEnvelope(frame)
          if (env.type === MSG_DELTA_BATCH) catchupSkipped += collectWireOps(env.payload, incoming)
          else if (env.type === MSG_ERROR) throw new Error(`DELTA error: ${JSON.stringify(env.payload)}`)
          else throw new Error('expected DELTA_BATCH')
        }
      }
    }
  }

  summary.received = incoming.length + catchupSkipped
  if (incoming.length === 0) {
    summary.skipped = catchupSkipped
    if (joinDs && store.ops.length === 0) store.adoptDatastore(joinDs)
    return summary
  }

  const expectedDs = joinDs || store.datastoreIdHex()
  const { applied, skipped } = store.importBundle(incoming, { expectedDs })
  summary.applied = applied
  summary.skipped = catchupSkipped + skipped
  return summary
}
