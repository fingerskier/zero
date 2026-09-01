import { test } from 'node:test'
import assert from 'node:assert/strict'

import { encode, bytesToHex } from '../models/cbor.mjs'
import { envelopeToTagged, computeOpId } from '../models/op.mjs'
import { PeerStore, EPOCH_UNKNOWN, KIND_SET_PROPERTY, encodeSchemaIr } from './store.mjs'
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
