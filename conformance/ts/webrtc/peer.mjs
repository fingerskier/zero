// Shared RELAY 0.2 peer protocol over a DataChannel.
// Handshake roles come from `isHandshakeServer` (smaller PeerId issues
// CHALLENGE/WELCOME; the other sends HELLO/AUTH). AUTH still uses the
// same AuthTranscript / zerodb-relay-auth-v2 preimage as the relay
// (conformance/ts/models/relay.mjs ≡ zerodb-core handshake.rs).
// Do not invent a second AUTH domain. RTC offerer ≠ handshake server.

import { bytesToHex } from '../models/cbor.mjs'
import {
  DEFAULT_LIMITS,
  DOMAIN_RELAY_AUTH_V1,
  ERR_AUTH_FAILED,
  MSG_AUTH,
  MSG_CHALLENGE,
  MSG_ERROR,
  MSG_GOODBYE,
  MSG_HELLO,
  MSG_OP_ACK,
  MSG_OPS,
  MSG_WELCOME,
  RELAY_CAPS,
  authTranscript,
  authenticate,
  decodeEnvelope,
  encodeEnvelope,
  isHandshakeServer,
  negotiateWelcomeCaps,
  signAuth,
} from '../models/relay.mjs'
import { AUTH_WRONG_DATASTORE } from '../peer/store.mjs'
import { checkWelcomeProtocol, encodeRelayOp, frontierFromOps, splitOpsBatches, welcomeLimits } from '../peer/client.mjs'
import { concatBytes, signBytes } from '../peer/crypto.mjs'
import { ChannelTransport } from './channel.mjs'

export const ERR_VERSION_MISMATCH = 0x102
/** Session-level admission (populated A vs offered B). Named peer reject; not a second AUTH domain. */
export const ERR_AUTH_WRONG_DATASTORE = 0x203
export { AUTH_WRONG_DATASTORE, ERR_AUTH_FAILED }

/**
 * Bound/populated A vs offered B is AUTH_WRONG_DATASTORE before OPS.
 * Empty (`boundDs` falsy) may adopt. HELLO.datastore is not in AuthTranscript.
 */
export function admitDatastore(boundDs, offeredDs) {
  if (!boundDs || !offeredDs) return null
  if (String(boundDs).toLowerCase() !== String(offeredDs).toLowerCase()) return AUTH_WRONG_DATASTORE
  return null
}

function sendWrongDatastore(t, requestId) {
  t.send(
    encodeEnvelope(MSG_ERROR, requestId, {
      code: ERR_AUTH_WRONG_DATASTORE,
      message: AUTH_WRONG_DATASTORE,
      fatal: true,
    }),
  )
  return { phase: 'wrong-datastore', reason: AUTH_WRONG_DATASTORE, code: ERR_AUTH_WRONG_DATASTORE }
}

const RELAY_PROTOCOL_VERSION = 1
const DEFAULT_RELAY_LEVEL = 2

export function signAuthV1NonceOnly(seed, nonce) {
  const n = nonce instanceof Uint8Array ? nonce : hexTo32(nonce)
  return signBytes(seed, concatBytes(DOMAIN_RELAY_AUTH_V1, n))
}

function hexTo32(h) {
  const s = String(h)
  const out = new Uint8Array(32)
  for (let i = 0; i < 32; i++) out[i] = parseInt(s.slice(i * 2, i * 2 + 2), 16)
  return out
}

function asBytes32(v) {
  if (v instanceof Uint8Array) return v
  return hexTo32(v)
}

function expectType(env, want, name) {
  if (env.type === MSG_ERROR) {
    const code = env.payload && env.payload.code
    const msg = env.payload && env.payload.message
    const err = new Error(`${name} got ERROR ${code} ${msg}`)
    err.code = code
    err.relayMessage = msg
    throw err
  }
  if (env.type !== want) throw new Error(`expected ${name}, got type ${env.type}`)
  return env
}

function opsPayload(ds, wires) {
  return {
    datastore: ds,
    operations: wires.map((w) => ({
      op_id: w.id,
      author: w.author,
      physical_ms: w.ts.p,
      logical: w.ts.l,
      wire: JSON.stringify(w),
    })),
  }
}

function catchupWire(op) {
  if (!op || typeof op.wire !== 'string') return null
  try {
    return JSON.parse(op.wire)
  } catch {
    return null
  }
}

function intendedWelcome(helloCaps) {
  return {
    protocol_version: RELAY_PROTOCOL_VERSION,
    relay_level: DEFAULT_RELAY_LEVEL,
    capabilities: negotiateWelcomeCaps(helloCaps || []),
    limits: { ...DEFAULT_LIMITS },
  }
}

/**
 * Answerer (server role): CHALLENGE / transcript AUTH / WELCOME, then OPS ingest.
 * `nonce` is injectable so tests can pin the transcript.
 */
export async function serveDirect(store, channel, opts = {}) {
  const t = new ChannelTransport(channel)
  const nonce = opts.nonce instanceof Uint8Array ? opts.nonce : crypto.getRandomValues(new Uint8Array(32))
  const expectedDs = opts.expectedDs || null
  const welcomeOverride = opts.welcomeOverride || null

  const hello = decodeEnvelope(await t.recv())
  expectType(hello, MSG_HELLO, 'HELLO')
  const claimed = asBytes32(hello.payload.peer_id)
  const pk = asBytes32(hello.payload.public_key)
  const helloVersion = hello.payload.protocol_version
  const helloCaps = hello.payload.capabilities || []
  if (helloVersion !== RELAY_PROTOCOL_VERSION) {
    t.send(
      encodeEnvelope(MSG_ERROR, hello.request_id, {
        code: ERR_VERSION_MISMATCH,
        message: 'VERSION_MISMATCH',
        fatal: true,
      }),
    )
    return { phase: 'version-mismatch', code: ERR_VERSION_MISMATCH }
  }

  t.send(
    encodeEnvelope(MSG_CHALLENGE, hello.request_id, {
      nonce: bytesToHex(nonce),
    }),
  )

  const auth = decodeEnvelope(await t.recv())
  if (auth.type === MSG_ERROR) {
    return { phase: 'error', error: auth.payload }
  }
  expectType(auth, MSG_AUTH, 'AUTH')
  const sig = asBytes64(auth.payload.signature)
  const transcript = authTranscript(claimed, pk, helloVersion, helloCaps, nonce)
  const claimedHex = bytesToHex(claimed)
  const pkHex = bytesToHex(pk)
  const transcriptPeer = bytesToHex(transcript.peer_id)
  const transcriptPk = bytesToHex(transcript.public_key)
  if (transcriptPeer !== claimedHex || transcriptPk !== pkHex) {
    t.send(
      encodeEnvelope(MSG_ERROR, auth.request_id, {
        code: ERR_AUTH_FAILED,
        message: 'AUTH_FAILED',
        fatal: true,
      }),
    )
    return { phase: 'auth-failed', code: ERR_AUTH_FAILED }
  }
  const authErr = authenticate(claimed, pk, transcript, sig)
  if (authErr !== null) {
    t.send(
      encodeEnvelope(MSG_ERROR, auth.request_id, {
        code: ERR_AUTH_FAILED,
        message: 'AUTH_FAILED',
        fatal: true,
      }),
    )
    return { phase: 'auth-failed', code: authErr }
  }

  const offered = hello.payload && hello.payload.datastore
  const populated = store.ops.length > 0
  const bound = expectedDs || (populated ? store.dsHex : null)
  const admitErr = admitDatastore(bound, offered)
  if (admitErr) {
    return sendWrongDatastore(t, auth.request_id)
  }

  const welcome = welcomeOverride || intendedWelcome(helloCaps)
  t.send(encodeEnvelope(MSG_WELCOME, auth.request_id, welcome))
  if (opts.stopAfterWelcome) {
    return { phase: 'welcomed', welcome }
  }

  const adopting = !populated
  let joinDs = bound
  if (joinDs == null && offered) {
    joinDs = offered
    if (adopting) store.adoptDatastore(offered)
  }
  const outcomes = []
  let applied = 0
  let rejected = 0
  let batches = 0

  for (;;) {
    const opsFrame = decodeEnvelope(await t.recv())
    if (opsFrame.type === MSG_GOODBYE) break
    if (opsFrame.type !== MSG_OPS) {
      return { phase: batches ? 'ops' : 'welcomed', welcome, extra: opsFrame, applied, rejected, outcomes, datastore: joinDs }
    }
    batches += 1
    const ds = opsFrame.payload.datastore
    const opsAdmit = admitDatastore(joinDs, ds)
    if (opsAdmit) {
      const fail = sendWrongDatastore(t, opsFrame.request_id)
      return { ...fail, applied, rejected, outcomes, datastore: joinDs, batches }
    }
    if (joinDs == null) joinDs = ds
    const incoming = []
    for (const op of opsFrame.payload.operations || []) {
      const wire = catchupWire(op)
      if (wire) incoming.push(wire)
    }
    const batchOutcomes = []
    for (const wire of incoming) {
      const r = store.ingest(wire, { expectedDs: joinDs })
      if (r === 'applied') {
        applied += 1
        batchOutcomes.push({ op_id: wire.id, outcome: 'ACCEPT' })
      } else if (r === 'duplicate') {
        batchOutcomes.push({ op_id: wire.id, outcome: 'DUPLICATE' })
      } else {
        rejected += 1
        batchOutcomes.push({ op_id: wire.id, outcome: 'REJECT', reason: r })
      }
    }
    outcomes.push(...batchOutcomes)
    t.send(encodeEnvelope(MSG_OP_ACK, opsFrame.request_id, { outcomes: batchOutcomes }))
  }
  if (adopting && applied > 0 && joinDs && store.dsHex !== joinDs) store.adoptDatastore(joinDs)
  return {
    phase: 'ops',
    applied,
    rejected,
    outcomes,
    datastore: joinDs,
    batches,
    frontier: frontierFromOps(store.ops, joinDs || store.dsHex),
  }
}

function asBytes64(v) {
  if (v instanceof Uint8Array) return v
  const s = String(v)
  const out = new Uint8Array(s.length / 2)
  for (let i = 0; i < out.length; i++) out[i] = parseInt(s.slice(i * 2, i * 2 + 2), 16)
  return out
}

/**
 * Initiator (client role): HELLO / AUTH (v2 transcript by default) / WELCOME
 * reject / OPS. `signAuthFn(seed, transcript)` overrides the signature
 * (v1 nonce-only negative).
 */
export async function connectDirect(store, channel, opts = {}) {
  const t = new ChannelTransport(channel)
  const helloCaps = opts.capabilities || RELAY_CAPS.slice()
  const joinDs = opts.joinDs === undefined ? store.datastoreIdHex() : opts.joinDs
  const signFn = opts.signAuthFn || ((seed, transcript) => signAuth(seed, transcript))

  const helloPayload = {
    peer_id: store.authorHex,
    public_key: store.pkHex,
    protocol_version: opts.helloProtocolVersion == null ? 1 : opts.helloProtocolVersion,
    capabilities: helloCaps,
  }
  if (joinDs) helloPayload.datastore = joinDs
  if (opts.remoteCursor) helloPayload.cursor = opts.remoteCursor
  t.send(encodeEnvelope(MSG_HELLO, 1, helloPayload))
  const challenge = expectType(decodeEnvelope(await t.recv()), MSG_CHALLENGE, 'CHALLENGE')
  const nonce = asBytes32(challenge.payload.nonce)
  const transcript = authTranscript(store.author, store.pk, 1, helloCaps, nonce)
  const sig = signFn(store.seed, transcript)
  t.send(encodeEnvelope(MSG_AUTH, 2, { signature: bytesToHex(sig) }))

  const welcome = decodeEnvelope(await t.recv())
  if (welcome.type === MSG_ERROR) {
    const err = new Error(`AUTH ERROR ${welcome.payload && welcome.payload.code}`)
    err.code = welcome.payload && welcome.payload.code
    err.relayMessage = welcome.payload && welcome.payload.message
    throw err
  }
  expectType(welcome, MSG_WELCOME, 'WELCOME')
  checkWelcomeProtocol(welcome.payload)

  const toSend = store.exportOps(joinDs, { cursor: opts.remoteCursor })
  const limits = welcomeLimits(welcome.payload)
  const encoded = toSend.map(encodeRelayOp)
  const batches = toSend.length
    ? splitOpsBatches(
        joinDs,
        encoded,
        limits.max_batch_ops,
        limits.max_batch_bytes,
        limits.max_payload_bytes,
      )
    : []
  const outcomes = []
  let requestId = 3
  let offset = 0
  for (const batch of batches) {
    const wires = toSend.slice(offset, offset + batch.length)
    offset += batch.length
    t.send(encodeEnvelope(MSG_OPS, requestId, opsPayload(joinDs, wires)))
    requestId += 1
    const ack = expectType(decodeEnvelope(await t.recv()), MSG_OP_ACK, 'OP_ACK')
    outcomes.push(...(ack.payload.outcomes || []))
  }
  t.send(encodeEnvelope(MSG_GOODBYE, 0, { reason: 'done' }))
  return {
    welcome: welcome.payload,
    sent: toSend.length,
    sentIds: toSend.map((w) => w.id),
    batches: batches.length,
    outcomes,
    frontier: frontierFromOps(store.ops, joinDs || store.dsHex),
  }
}

/**
 * Pick CHALLENGE/WELCOME vs HELLO/AUTH from PeerId order.
 * Same `isHandshakeServer` / `AuthTranscript` helper — no second domain.
 */
export async function runNegotiated(store, remotePeerId, channel, opts = {}) {
  if (isHandshakeServer(store.author, remotePeerId)) {
    const result = await serveDirect(store, channel, opts)
    return { role: 'server', ...result }
  }
  const result = await connectDirect(store, channel, opts)
  return { role: 'client', ...result }
}
