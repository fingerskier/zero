import { test } from 'node:test'
import assert from 'node:assert/strict'

import { encode, bytesToHex } from '../models/cbor.mjs'
import { envelopeToTagged, computeOpId } from '../models/op.mjs'
import { signOp } from './crypto.mjs'
import {
  PeerStore,
  EPOCH_UNKNOWN,
  AUTH_WRONG_DATASTORE,
  CLOCK_DRIFT,
  APPLY_INVALID,
  KIND_SCHEMA_EPOCH,
  KIND_SET_PROPERTY,
  encodeSchemaIr,
} from './store.mjs'
import { splitOpsBatches, encodeRelayOp, welcomeLimits } from './client.mjs'

const TODO_PIN = { nodes: { Todo: { props: { title: 'lww' } } } }

test('signed SchemaEpoch + CreateNode + SetProperty verify locally', () => {
  const store = new PeerStore()
  const epoch = store.applySchemaEpoch(TODO_PIN)
  assert.equal(epoch.kind, 5)
  assert.equal(epoch.ep, 0)
  assert.equal(epoch.body.epoch, 1)
  assert.equal(epoch.body.prev, null)
  assert.deepEqual(epoch.body.migration, [])
  const ir = encodeSchemaIr(TODO_PIN)
  assert.equal(epoch.body.schema, ir.idHex)
  assert.equal(store.schemaEpoch, 1)

  const { node, wire: create } = store.createNode('Todo')
  assert.equal(create.kind, 1)
  assert.equal(create.ep, 1)
  const set = store.setLww(node, 'title', 'milk')
  assert.equal(set.kind, 3)
  assert.equal(set.body.crdt, 'lww')
  assert.equal(store.getLww(node, 'title'), 'milk')

  for (const w of [epoch, create, set]) {
    const env = {
      v: w.v,
      ds: w.ds,
      ep: w.ep,
      author: w.author,
      ts: w.ts,
      deps: w.deps,
      grp: null,
      kind: w.kind,
      body: w.body,
    }
    assert.equal(bytesToHex(computeOpId(encode(envelopeToTagged(env)))), w.id)
    assert.equal(store.verifyWire(w), null)
  }
})

test('fail closed on EPOCH_UNKNOWN when ep is past applied kind-5 chain', () => {
  const a = new PeerStore()
  a.applySchemaEpoch(TODO_PIN)
  const { node } = a.createNode('Todo')
  a.setLww(node, 'title', 'milk')
  const future = a.commit(KIND_SET_PROPERTY, { node, path: 'title', crdt: 'lww', value: 'x' }, { ep: 2 })
  assert.equal(a.verifyWire(future), null)

  const b = new PeerStore()
  b.importBundle(a.ops.filter((w) => w.kind === 5 || w.kind === 1 || (w.kind === 3 && w.ep === 1)))
  assert.equal(b.schemaEpoch, 1)
  assert.equal(b.ingest(future), EPOCH_UNKNOWN)
  assert.equal(b.getLww(node, 'title'), 'milk')
  assert.equal(b.lastRejects.at(-1).reason, EPOCH_UNKNOWN)
})

test('import applies SchemaEpoch before epoch-bound data', () => {
  const a = new PeerStore()
  a.applySchemaEpoch(TODO_PIN)
  const { node } = a.createNode('Todo')
  a.setLww(node, 'title', 'milk')
  const shuffled = [a.ops[2], a.ops[0], a.ops[1]]
  const b = new PeerStore()
  const { applied } = b.importBundle(shuffled)
  assert.equal(applied, 3)
  assert.equal(b.schemaEpoch, 1)
  assert.equal(b.getLww(node, 'title'), 'milk')
})

test('splitOpsBatches honors advertised max_batch_ops and max_payload_bytes', () => {
  const store = new PeerStore()
  const wires = []
  for (let i = 0; i < 5; i++) {
    const { wire } = store.createNode('Todo')
    wires.push(wire)
  }
  const encoded = wires.map(encodeRelayOp)
  const batches = splitOpsBatches(store.dsHex, encoded, 2, 1_000_000, 1_000_000)
  assert.equal(batches.length, 3)
  assert.equal(batches[0].length, 2)
  assert.equal(batches[1].length, 2)
  assert.equal(batches[2].length, 1)

  assert.throws(
    () => splitOpsBatches(store.dsHex, encoded, 8, 1_000_000, 20),
    /max_payload_bytes/,
  )
})

function signRemote(store, { kind, body, ep, ds, ts }) {
  const stamp = ts || { p: store.clock(), l: 0 }
  const env = {
    v: 1,
    ds: ds || store.dsHex,
    ep: ep != null ? ep : store.schemaEpoch,
    author: store.authorHex,
    ts: stamp,
    deps: [],
    grp: null,
    kind,
    body,
  }
  const id = computeOpId(encode(envelopeToTagged(env)))
  return {
    id: bytesToHex(id),
    v: 1,
    ds: env.ds,
    ep: env.ep,
    author: store.authorHex,
    author_pk: store.pkHex,
    ts: stamp,
    deps: [],
    kind,
    body,
    sig: bytesToHex(signOp(store.seed, id)),
  }
}

test('join A rejects a well-signed op whose ds is B (AUTH_WRONG_DATASTORE)', () => {
  const a = new PeerStore()
  a.applySchemaEpoch(TODO_PIN)
  const { node } = a.createNode('Todo')
  a.setLww(node, 'title', 'milk')

  const foreign = new PeerStore()
  foreign.applySchemaEpoch(TODO_PIN)
  const { node: otherNode } = foreign.createNode('Todo')
  const foreignOp = foreign.setLww(otherNode, 'title', 'poison')
  assert.notEqual(foreign.dsHex, a.dsHex)
  assert.equal(a.verifyWire(foreignOp), null)

  const empty = new PeerStore()
  const emptyDs = empty.dsHex
  assert.equal(empty.ingest(foreignOp, { expectedDs: a.dsHex }), AUTH_WRONG_DATASTORE)
  assert.equal(empty.dsHex, emptyDs)
  assert.notEqual(empty.dsHex, foreign.dsHex)
  assert.equal(empty.ops.length, 0)
  assert.equal(empty.getLww(otherNode, 'title'), null)

  assert.throws(
    () => empty.importBundle([foreignOp], { expectedDs: a.dsHex }),
    /cannot adopt datastore from a bundle with no accepted operations/,
  )
  assert.equal(empty.dsHex, emptyDs)
  assert.equal(empty.ops.length, 0)

  empty.adoptDatastore(a.dsHex)
  assert.equal(empty.ingest(foreignOp), AUTH_WRONG_DATASTORE)
  assert.equal(empty.dsHex, a.dsHex)
  assert.equal(empty.ops.length, 0)
})

test('import far-future LWW is CLOCK_DRIFT; in-window remote stamp is observed so local write wins', () => {
  const wall = 1_000_000
  const author = new PeerStore({ clock: () => wall })
  author.applySchemaEpoch(TODO_PIN)
  const { node } = author.createNode('Todo')
  const far = signRemote(author, {
    kind: KIND_SET_PROPERTY,
    body: { node, path: 'title', crdt: 'lww', value: 'future' },
    ep: 1,
    ts: { p: wall + 120_000, l: 0 },
  })
  assert.equal(author.verifyWire(far), null)

  const peer = new PeerStore({ clock: () => wall })
  peer.importBundle(author.ops.filter((w) => w.kind === 5 || w.kind === 1))
  assert.equal(peer.ingest(far), CLOCK_DRIFT)
  assert.equal(peer.getLww(node, 'title'), null)
  peer.setLww(node, 'title', 'local')
  assert.equal(peer.getLww(node, 'title'), 'local')

  const near = signRemote(author, {
    kind: KIND_SET_PROPERTY,
    body: { node, path: 'title', crdt: 'lww', value: 'near' },
    ep: 1,
    ts: { p: wall + 10_000, l: 7 },
  })
  const observed = new PeerStore({ clock: () => wall })
  observed.importBundle(author.ops.filter((w) => w.kind === 5 || w.kind === 1))
  assert.equal(observed.ingest(near), 'applied')
  assert.equal(observed.getLww(node, 'title'), 'near')
  const local = observed.setLww(node, 'title', 'after')
  assert.ok(local.ts.p > near.ts.p || (local.ts.p === near.ts.p && local.ts.l > near.ts.l))
  assert.equal(observed.getLww(node, 'title'), 'after')
})

test('invalid SchemaEpoch body is rejected and does not bump schemaEpoch', () => {
  const author = new PeerStore()
  const ir = encodeSchemaIr(TODO_PIN)
  const goodShape = {
    epoch: 1,
    schema: ir.idHex,
    ir: ir.hex,
    prev: null,
    migration: [],
  }

  const cases = [
    { ...goodShape, ir: 'ff' },
    { ...goodShape, schema: '11'.repeat(32) },
    { ...goodShape, epoch: 0 },
  ]
  for (const body of cases) {
    const peer = new PeerStore()
    const wire = signRemote(author, { kind: KIND_SCHEMA_EPOCH, body, ep: 0, ds: peer.dsHex })
    assert.equal(peer.verifyWire(wire), null)
    assert.equal(peer.ingest(wire), APPLY_INVALID)
    assert.equal(peer.schemaEpoch, 0)
    assert.equal(peer.ops.length, 0)
    const data = signRemote(author, {
      kind: KIND_SET_PROPERTY,
      body: { node: 'aa'.repeat(16), path: 'title', crdt: 'lww', value: 'x' },
      ep: 1,
      ds: peer.dsHex,
    })
    assert.equal(peer.ingest(data), EPOCH_UNKNOWN)
    assert.equal(peer.schemaEpoch, 0)
  }
})

test('welcomeLimits reads advertised values', () => {
  const limits = welcomeLimits({
    limits: {
      max_batch_ops: 16,
      max_batch_bytes: 1024,
      max_payload_bytes: 256,
      ops_per_second: 100,
      bytes_per_second: 4096,
    },
  })
  assert.equal(limits.max_batch_ops, 16)
  assert.equal(limits.max_payload_bytes, 256)
  assert.equal(limits.ops_per_second, 100)
  assert.equal(limits.bytes_per_second, 4096)
})
