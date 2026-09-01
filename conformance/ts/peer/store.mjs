// In-memory KERNEL peer store. Signs CreateNode / SetProperty LWW /
// SchemaEpoch (kind 5) and fail-closes on EPOCH_UNKNOWN.
// Pure TS/JS — MUST NOT import zerodb-napi.

import { encode, bytesToHex, hexToBytes } from '../models/cbor.mjs'
import { blake3 } from '../models/blake3.mjs'
import { HlcModel } from '../models/hlc.mjs'
import { validateIr } from '../models/schemavalidate.mjs'
import { envelopeToTagged, computeOpId, sigPreimage, verifySignature } from '../models/op.mjs'
import { generateIdentity, deriveLocalDatastore, signOp } from './crypto.mjs'

export const KIND_GENESIS = 0
export const KIND_CREATE_NODE = 1
export const KIND_SET_PROPERTY = 3
export const KIND_SCHEMA_EPOCH = 5
export const EPOCH_UNKNOWN = 'EPOCH_UNKNOWN'
export const AUTH_SIG_INVALID = 'AUTH_SIG_INVALID'

const DOMAIN_SCHEMA_IR = new TextEncoder().encode('zerodb-schema-ir-v1')
const ZERO_DS = '00'.repeat(32)

const CRDT_TAG = { lww: 0, gcounter: 1, pncounter: 2, orset: 3, flag: 4 }

function concatBytes(...parts) {
  let n = 0
  for (const p of parts) n += p.length
  const out = new Uint8Array(n)
  let o = 0
  for (const p of parts) {
    out.set(p, o)
    o += p.length
  }
  return out
}

function schemaId(irBytes) {
  return blake3(concatBytes(DOMAIN_SCHEMA_IR, irBytes))
}

function cmpWire(a, b) {
  if (a.ts.p !== b.ts.p) return a.ts.p < b.ts.p ? -1 : 1
  if (a.ts.l !== b.ts.l) return a.ts.l < b.ts.l ? -1 : 1
  if (a.author !== b.author) return a.author < b.author ? -1 : 1
  if (a.id !== b.id) return a.id < b.id ? -1 : 1
  return 0
}

function batchApplyRank(kind) {
  if (kind === KIND_GENESIS) return 0
  if (kind === KIND_SCHEMA_EPOCH) return 1
  return 2
}

export function epochFirst(ops) {
  return ops
    .map((op, i) => ({ op, i }))
    .sort((a, b) => {
      const ra = batchApplyRank(a.op.kind)
      const rb = batchApplyRank(b.op.kind)
      if (ra !== rb) return ra - rb
      return a.i - b.i
    })
    .map((x) => x.op)
}

/** M1 pin JSON → tagged SCHEMA §2 IR (nullable true, default value types). */
export function pinToIrTagged(pin) {
  const nodesIn = pin.nodes || {}
  const nodes = {}
  for (const [label, entity] of Object.entries(nodesIn)) {
    const propsIn = (entity && entity.props) || {}
    const props = {}
    for (const [path, crdt] of Object.entries(propsIn)) {
      let name
      let encrypted = false
      if (typeof crdt === 'string') {
        name = crdt
      } else {
        name = crdt.crdt
        encrypted = Boolean(crdt.encrypted)
      }
      const tag = CRDT_TAG[name]
      if (tag == null) throw new Error(`unknown crdt ${name}`)
      const valueType = tag === 1 || tag === 2 ? 2 : tag === 4 ? 1 : 4
      props[path] = {
        t: 'map',
        v: {
          crdt: { t: 'uint', v: tag },
          encrypted: { t: 'bool', v: encrypted },
          nullable: { t: 'bool', v: true },
          type: { t: 'uint', v: valueType },
        },
      }
    }
    nodes[label] = { t: 'map', v: { props: { t: 'map', v: props } } }
  }
  const top = {
    v: { t: 'uint', v: 1 },
    nodes: { t: 'map', v: nodes },
    edges: { t: 'map', v: {} },
  }
  if (pin.name) top.name = { t: 'text', v: pin.name }
  const ir = { t: 'map', v: top }
  const invalid = validateIr(ir)
  if (invalid !== null) throw new Error(invalid)
  return ir
}

export function encodeSchemaIr(pinOrTagged) {
  const tagged = pinOrTagged && pinOrTagged.t === 'map' ? pinOrTagged : pinToIrTagged(pinOrTagged)
  const bytes = encode(tagged)
  return { bytes, hex: bytesToHex(bytes), id: schemaId(bytes), idHex: bytesToHex(schemaId(bytes)) }
}

function randomNodeHex() {
  const b = new Uint8Array(16)
  crypto.getRandomValues(b)
  return bytesToHex(b)
}

export class PeerStore {
  constructor(opts = {}) {
    const id = generateIdentity(opts.seed)
    const local = deriveLocalDatastore(id.author, opts.salt)
    this.seed = id.seed
    this.pk = id.pk
    this.author = id.author
    this.authorHex = id.authorHex
    this.pkHex = id.pkHex
    this.ds = local.ds
    this.dsHex = local.dsHex
    this.salt = local.salt
    this.hlc = new HlcModel()
    this.clock = opts.clock || (() => Date.now())
    this.ops = []
    this.byId = new Map()
    this.schemaEpoch = 0
    this.schemaIdHex = null
    this.schemaIrHex = null
    this.nodes = new Map()
    this.props = new Map()
    this.lastRejects = []
  }

  datastoreIdHex() {
    return this.dsHex
  }

  adoptDatastore(dsHex) {
    this.dsHex = dsHex
    this.ds = hexToBytes(dsHex)
  }

  nextTs() {
    return this.hlc.local(this.clock())
  }

  commit(kind, body, { ep } = {}) {
    const stamp = ep != null ? ep : this.schemaEpoch
    const ts = this.nextTs()
    const env = {
      v: 1,
      ds: this.dsHex,
      ep: stamp,
      author: this.authorHex,
      ts: { p: ts.physical_ms, l: ts.logical },
      deps: [],
      grp: null,
      kind,
      body,
    }
    const canonical = encode(envelopeToTagged(env))
    const id = computeOpId(canonical)
    const idHex = bytesToHex(id)
    const sig = signOp(this.seed, id)
    const wire = {
      id: idHex,
      v: 1,
      ds: this.dsHex,
      ep: stamp,
      author: this.authorHex,
      author_pk: this.pkHex,
      ts: { p: ts.physical_ms, l: ts.logical },
      deps: [],
      kind,
      body,
      sig: bytesToHex(sig),
    }
    const applied = this.#apply(wire)
    if (applied !== 'applied') {
      throw new Error(applied)
    }
    return wire
  }

  applySchemaEpoch(pinOrTagged) {
    const ir = encodeSchemaIr(pinOrTagged)
    const body = {
      epoch: 1,
      schema: ir.idHex,
      ir: ir.hex,
      prev: null,
      migration: [],
    }
    return this.commit(KIND_SCHEMA_EPOCH, body, { ep: 0 })
  }

  createNode(label, nodeHex) {
    const node = nodeHex || randomNodeHex()
    const wire = this.commit(KIND_CREATE_NODE, { label, node })
    return { node, wire }
  }

  setLww(node, path, value) {
    return this.commit(KIND_SET_PROPERTY, { node, path, crdt: 'lww', value })
  }

  exportOps(dsHex) {
    const ds = dsHex || this.dsHex
    return this.ops.filter((w) => w.ds === ds || (w.kind === KIND_GENESIS && w.ds === ZERO_DS))
  }

  getLww(node, path) {
    const key = `${node}\0${path}`
    const v = this.props.get(key)
    return v == null ? null : v
  }

  verifyWire(wire) {
    if (!wire || !wire.id || !wire.author_pk || !wire.sig) return AUTH_SIG_INVALID
    const pk = hexToBytes(wire.author_pk)
    if (bytesToHex(blake3(pk)) !== wire.author) return AUTH_SIG_INVALID
    const env = {
      v: wire.v,
      ds: wire.ds,
      ep: wire.ep,
      author: wire.author,
      ts: wire.ts,
      deps: wire.deps || [],
      grp: wire.grp ?? null,
      kind: wire.kind,
      body: wire.body,
    }
    const id = computeOpId(encode(envelopeToTagged(env)))
    if (bytesToHex(id) !== wire.id) return AUTH_SIG_INVALID
    if (!verifySignature(pk, sigPreimage(id), hexToBytes(wire.sig))) return AUTH_SIG_INVALID
    return null
  }

  ingest(wire) {
    const bad = this.verifyWire(wire)
    if (bad) {
      this.lastRejects.push({ op_id: wire.id, reason: bad })
      return bad
    }
    if (this.byId.has(wire.id)) return 'duplicate'
    if (wire.ep > this.schemaEpoch) {
      this.lastRejects.push({ op_id: wire.id, reason: EPOCH_UNKNOWN })
      return EPOCH_UNKNOWN
    }
    return this.#apply(wire)
  }

  importBundle(ops) {
    const incoming = epochFirst(ops)
    let applied = 0
    let skipped = 0
    for (const w of incoming) {
      const r = this.ingest(w)
      if (r === 'applied') applied += 1
      else skipped += 1
    }
    return { applied, skipped }
  }

  #apply(wire) {
    this.ops.push(wire)
    this.byId.set(wire.id, wire)
    if (wire.kind === KIND_SCHEMA_EPOCH) {
      const epoch = wire.body && wire.body.epoch
      if (typeof epoch === 'number' && (this.schemaEpoch === 0 || epoch >= this.schemaEpoch)) {
        this.schemaEpoch = epoch
        this.schemaIdHex = wire.body.schema
        this.schemaIrHex = wire.body.ir
      }
    }
    if (wire.kind === KIND_CREATE_NODE) {
      this.nodes.set(wire.body.node, wire.body.label)
    }
    if (wire.kind === KIND_SET_PROPERTY && wire.body && wire.body.crdt === 'lww') {
      const key = `${wire.body.node}\0${wire.body.path}`
      const prev = this.props.get(`${key}\0wire`)
      if (!prev || cmpWire(prev, wire) < 0) {
        this.props.set(key, wire.body.value)
        this.props.set(`${key}\0wire`, wire)
      }
    }
    return 'applied'
  }
}

export function bundleDatastoreId(ops) {
  for (const w of ops) {
    if (w.kind !== KIND_GENESIS && w.ds && w.ds !== ZERO_DS) return w.ds
  }
  throw new Error('bundle has no datastore id')
}
