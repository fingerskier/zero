// End-to-end proof: a wasm ZeroDb (browser peer store) syncs two-way with a
// Node NAPI Database over the WebSocket sync protocol v2 via zero-sync.mjs.
//
// Requires the wasm pkg to be built first:
//   cd zerodb-wasm && npx wasm-pack build --target web
//
// Run: node --test examples/browser-peer/test/

import test from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { createRequire } from 'node:module'

import { syncOnce, connectPush, FrameReader, encodeFrame } from '../zero-sync.mjs'

const here = path.dirname(fileURLToPath(import.meta.url))
const pkgDir = path.join(here, '..', '..', '..', 'zerodb-wasm', 'pkg')
const require = createRequire(import.meta.url)
const { Database } = require('../../../zerodb-napi/index.js')

assert.ok(
  fs.existsSync(path.join(pkgDir, 'zerodb_wasm.js')),
  'wasm pkg missing — run: cd zerodb-wasm && npx wasm-pack build --target web',
)
const wasm = await import(
  new URL(`file://${path.join(pkgDir, 'zerodb_wasm.js').replaceAll('\\', '/')}`)
)
await wasm.default({
  module_or_path: fs.readFileSync(path.join(pkgDir, 'zerodb_wasm_bg.wasm')),
})
const { ZeroDb } = wasm

function tmpDb(name) {
  return path.join(
    fs.mkdtempSync(path.join(os.tmpdir(), 'zerodb-browser-peer-')),
    `${name}.sqlite`,
  )
}

test('frame reader handles fragmented and coalesced frames', () => {
  const reader = new FrameReader()
  const a = encodeFrame({ n: 1 })
  const b = encodeFrame({ n: 2 })
  // Coalesce both frames plus a split tail of a third.
  const c = encodeFrame({ n: 3 })
  const joined = new Uint8Array(a.length + b.length + 2)
  joined.set(a, 0)
  joined.set(b, a.length)
  joined.set(c.subarray(0, 2), a.length + b.length)
  const first = reader.push(joined)
  assert.deepEqual(first.map(f => f.n), [1, 2])
  const rest = reader.push(c.subarray(2))
  assert.deepEqual(rest.map(f => f.n), [3])
})

test('wasm peer converges two-way with a NAPI serve peer', async () => {
  const db = Database.init(tmpDb('node-peer'))
  const port = db.serve(0)
  const url = `ws://127.0.0.1:${port}`
  try {
    // Node peer seeds the datastore.
    const node = db.createNode('Todo')
    db.setLww(node, 'title', 'from-node')

    // Fresh wasm peer adopts the Node datastore on first sync.
    const web = new ZeroDb()
    assert.notEqual(web.datastoreId(), db.datastoreId())
    const first = await syncOnce(web, url)
    assert.equal(first.accepted, 2)
    assert.equal(first.sent, 0)
    assert.equal(web.datastoreId(), db.datastoreId())
    assert.equal(web.getLww(node, 'title'), 'from-node')

    // Both sides mutate; one session converges both.
    web.setLww(node, 'web_only', 'from-web')
    const webNode = web.createNode('WebTodo')
    db.setLww(node, 'node_only', 'x')
    const second = await syncOnce(web, url)
    assert.equal(second.accepted, 1) // node_only
    assert.equal(second.sent, 2) // web_only + WebTodo create
    assert.equal(second.remoteAccepted, 2)
    assert.equal(web.getLww(node, 'node_only'), 'x')
    assert.equal(db.getLww(node, 'web_only'), 'from-web')
    const nodeSide = db.listNodes().map(n => n.id)
    assert.ok(nodeSide.includes(webNode), 'node peer sees wasm-created node')
    assert.equal(db.opCount(), web.opCount())

    // Third sync is a no-op both ways.
    const third = await syncOnce(web, url)
    assert.equal(third.accepted, 0)
    assert.equal(third.sent, 0)
  } finally {
    db.close()
  }
})

test('wasm peer receives a pushed op from a NAPI server without re-syncing', async () => {
  const db = Database.init(tmpDb('push-server'))
  const port = db.serve(0)
  const url = `ws://127.0.0.1:${port}`
  let handle
  try {
    const node = db.createNode('Todo')
    db.setLww(node, 'title', 'initial')

    const web = new ZeroDb()
    handle = await connectPush(web, url)
    assert.equal(handle.push, true, 'NAPI server should ack push')
    assert.equal(handle.summary.accepted, 2)
    assert.equal(web.getLww(node, 'title'), 'initial')

    // Server mutates; the wasm peer converges with no new session.
    db.setLww(node, 'title', 'pushed')
    const deadline = Date.now() + 5000
    while (web.getLww(node, 'title') !== 'pushed') {
      assert.ok(Date.now() < deadline, 'timeout waiting for pushed op')
      await new Promise(r => setTimeout(r, 25))
    }

    // Local wasm mutation streams back over the same session (onChange push).
    web.setLww(node, 'from_web', 'yes')
    const back = Date.now() + 5000
    while (db.getLww(node, 'from_web') !== 'yes') {
      assert.ok(Date.now() < back, 'timeout waiting for client push')
      await new Promise(r => setTimeout(r, 25))
    }
    assert.equal(db.opCount(), web.opCount())
  } finally {
    handle?.close()
    db.close()
  }
})

test('onChange fires for local ops and importJson, offChange stops it', () => {
  const a = new ZeroDb()
  const events = []
  const sub = a.onChange(e => events.push(e))
  const node = a.createNode('Todo')
  a.setLww(node, 'title', 'x')
  assert.equal(events.length, 2)
  assert.equal(events[0].kind, 'op')
  assert.equal(events[0].method, 'createNode')
  assert.equal(events[0].node, node)
  assert.ok(events[0].opId)
  assert.equal(events[1].method, 'setLww')
  assert.equal(events[1].key, 'title')

  const b = new ZeroDb()
  const bEvents = []
  b.onChange(e => bEvents.push(e))
  const res = b.importJson(a.exportJson())
  assert.equal(res.accepted, 2)
  b.replay()
  assert.deepEqual(bEvents.map(e => e.kind), ['import', 'replay'])
  assert.equal(bEvents[0].accepted, 2)

  a.offChange(sub)
  a.setLww(node, 'title', 'y')
  assert.equal(events.length, 2, 'offChange stops delivery')
})

test('identity round-trip: seed + bundle restore the same peer', async () => {
  const a = new ZeroDb()
  const node = a.createNode('Todo')
  a.setLww(node, 'title', 'persisted')
  const seed = a.seedHex()
  const ds = a.datastoreId()
  const bundle = a.exportJson()

  const b = ZeroDb.fromSeed(seed, ds)
  assert.equal(b.peerId(), a.peerId())
  const res = b.importJson(bundle)
  assert.equal(res.accepted, 2)
  b.replay()
  assert.equal(b.getLww(node, 'title'), 'persisted')
})
