// In-memory KERNEL peer store. Signs CreateNode / SetProperty LWW /
// SchemaEpoch (kind 5) and fail-closes on EPOCH_UNKNOWN.
// Pure TS/JS — MUST NOT import zerodb-napi.

import { encode, decode, bytesToHex, hexToBytes } from '../models/cbor.mjs'
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
export const AUTH_WRONG_DATASTORE = 'AUTH_WRONG_DATASTORE'
export const CLOCK_DRIFT = 'CLOCK_DRIFT'
export const APPLY_INVALID = 'APPLY_INVALID'

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

function isHex32(s) {
  return typeof s === 'string' && /^[0-9a-f]{64}$/i.test(s)
}

/** SCHEMA.md §3 body checks (same shape as Rust `validate_schema_epoch_body`). */
export function validateSchemaEpochBody(body) {
  if (!body || typeof body !== 'object' || Array.isArray(body)) return APPLY_INVALID
  const epoch = body.epoch
  if (typeof epoch !== 'number' || !Number.isInteger(epoch) || epoch <= 0) return APPLY_INVALID
  if (!isHex32(body.schema)) return APPLY_INVALID
  const irHex = body.ir
  if (typeof irHex !== 'string' || irHex.length === 0 || irHex.length % 2 !== 0 || !/^[0-9a-f]*$/i.test(irHex)) {
    return APPLY_INVALID
  }
  let irBytes
  try {
    irBytes = hexToBytes(irHex)
  } catch {
    return APPLY_INVALID
  }
  if (irBytes.length === 0) return APPLY_INVALID
  let tagged
  try {
    tagged = decode(irBytes)
  } catch {
    return APPLY_INVALID
  }
  if (validateIr(tagged) !== null) return APPLY_INVALID
  if (bytesToHex(schemaId(irBytes)) !== body.schema.toLowerCase()) return APPLY_INVALID
  const prev = body.prev
  if (prev == null) {
    if (epoch !== 1) return APPLY_INVALID
  } else if (typeof prev === 'string') {
    if (epoch === 1 || !isHex32(prev)) return APPLY_INVALID
  } else {
    return APPLY_INVALID
  }
  if (!Array.isArray(body.migration) || body.migration.length !== 0) return APPLY_INVALID
  return null
}

function belongsToDatastore(wire, expectedDs) {
  if (wire.kind === KIND_GENESIS) return wire.ds === ZERO_DS
  return wire.ds === expectedDs
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

  reject(wire, reason) {
    this.lastRejects.push({ op_id: wire.id, reason })
    return reason
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
    const invalid = validateSchemaEpochBody(body)
    if (invalid) throw new Error(invalid)
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

  ingest(wire, { expectedDs } = {}) {
    const bad = this.verifyWire(wire)
    if (bad) return this.reject(wire, bad)
    if (this.byId.has(wire.id)) return 'duplicate'
    const expected = expectedDs || this.dsHex
    if (!belongsToDatastore(wire, expected)) return this.reject(wire, AUTH_WRONG_DATASTORE)
    const wall = this.clock()
    if (wire.ts.p > wall + this.hlc.maxDriftMs) return this.reject(wire, CLOCK_DRIFT)
    const observed = this.hlc.recv(wall, { physical_ms: wire.ts.p, logical: wire.ts.l })
    if (observed.error) return this.reject(wire, CLOCK_DRIFT)
    if (wire.kind === KIND_SCHEMA_EPOCH) {
      const invalid = validateSchemaEpochBody(wire.body)
      if (invalid) return this.reject(wire, invalid)
    }
    if (wire.ep > this.schemaEpoch) return this.reject(wire, EPOCH_UNKNOWN)
    return this.#apply(wire)
  }

  importBundle(ops, { expectedDs } = {}) {
    let target = expectedDs
    if (!target) {
      if (this.ops.length === 0 && ops.length > 0) {
        try {
          target = bundleDatastoreId(ops)
        } catch {
          target = this.dsHex
        }
      } else {
        target = this.dsHex
      }
    }
    const adopting = this.ops.length === 0 && target !== this.dsHex
    const incoming = epochFirst(ops)
    let applied = 0
    let skipped = 0
    for (const w of incoming) {
      const r = this.ingest(w, { expectedDs: target })
      if (r === 'applied') applied += 1
      else skipped += 1
    }
    if (adopting && applied === 0) {
      throw new Error('cannot adopt datastore from a bundle with no accepted operations')
    }
    if (adopting) this.adoptDatastore(target)
    return { applied, skipped }
  }

  #apply(wire) {
    if (wire.kind === KIND_SCHEMA_EPOCH) {
      const invalid = validateSchemaEpochBody(wire.body)
      if (invalid) return invalid
    }
    this.ops.push(wire)
    this.byId.set(wire.id, wire)
    if (wire.kind === KIND_SCHEMA_EPOCH) {
      const epoch = wire.body.epoch
      if (this.schemaEpoch === 0 || epoch >= this.schemaEpoch) {
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
