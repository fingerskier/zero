// Persist/reopen proof for the M4a-a IndexedDB + OPFS adapters.
//
// Requires the wasm pkg:
//   cd zerodb-wasm && wasm-pack build --target web
//   (or: bash zerodb-wasm/scripts/build.sh)
//
// Run: node --test --test-concurrency=1 zerodb-wasm/test/persist-reopen.test.mjs
// One wasm heap — keep this file a single test so cases cannot interleave.

import test from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

import {
  ADAPTER_INDEXEDDB,
  ADAPTER_OPFS,
  IndexedDbJournal,
  OpfsJournal,
  detectAdapter,
  openDurable,
  openJournal,
} from '../js/storage.mjs'
import { fakeIndexedDB, memoryOpfsRoot } from './doubles.mjs'

const here = path.dirname(fileURLToPath(import.meta.url))
const pkgDir = path.join(here, '..', 'pkg')

assert.ok(
  fs.existsSync(path.join(pkgDir, 'zerodb_wasm.js')),
  'wasm pkg missing — run: cd zerodb-wasm && wasm-pack build --target web',
)

const wasm = await import(
  new URL(`file://${path.join(pkgDir, 'zerodb_wasm.js').replaceAll('\\', '/')}`)
)
await wasm.default({
  module_or_path: fs.readFileSync(path.join(pkgDir, 'zerodb_wasm_bg.wasm')),
})
const { ZeroDb } = wasm

function mutate(db) {
  const node = db.createNode('Todo')
  db.setLww(node, 'title', 'persisted')
  db.flagEnable(node, 'done')
  return node
}

function snapshot(db, node) {
  return {
    peer: db.peerId(),
    ds: db.datastoreId(),
    ops: db.opCount(),
    title: db.getLww(node, 'title'),
    done: db.getProp(node, 'done'),
  }
}

function assertRestored(expected, reopened, node) {
  assert.equal(reopened.peerId(), expected.peer)
  assert.equal(reopened.datastoreId(), expected.ds)
  assert.equal(reopened.opCount(), expected.ops)
  assert.equal(reopened.getLww(node, 'title'), expected.title)
  assert.equal(reopened.getProp(node, 'done'), expected.done)
}

function drop(db) {
  if (db && typeof db.free === 'function') db.free()
}

test('IDB and OPFS persist then reopen restore signed ops + identity', async () => {
  assert.equal(detectAdapter({ adapter: 'indexeddb' }), ADAPTER_INDEXEDDB)
  assert.equal(detectAdapter({ adapter: 'opfs' }), ADAPTER_OPFS)
  assert.equal(detectAdapter({ opfsRoot: memoryOpfsRoot() }), ADAPTER_OPFS)
  assert.equal(detectAdapter({ indexedDB: fakeIndexedDB() }), ADAPTER_INDEXEDDB)
  assert.throws(() => detectAdapter({}), /no durable browser storage/)

  assert.equal(
    (await openJournal('auto-store', { opfsRoot: memoryOpfsRoot() })).kind,
    ADAPTER_OPFS,
  )

  {
    const indexedDB = fakeIndexedDB()
    const first = await openDurable(ZeroDb, {
      name: 'idb-reopen',
      adapter: 'indexeddb',
      indexedDB,
    })
    assert.equal(first.restored, false)
    assert.equal(first.adapter, ADAPTER_INDEXEDDB)
    const node = mutate(first.db)
    await first.journal.persist(first.db)
    const expected = snapshot(first.db, node)
    drop(first.db)

    const second = await openDurable(ZeroDb, {
      name: 'idb-reopen',
      adapter: 'indexeddb',
      indexedDB,
    })
    assert.equal(second.restored, true)
    assertRestored(expected, second.db, node)
    drop(second.db)
  }

  {
    const opfsRoot = memoryOpfsRoot()
    const first = await openDurable(ZeroDb, {
      name: 'opfs-reopen',
      adapter: 'opfs',
      opfsRoot,
    })
    assert.equal(first.restored, false)
    assert.equal(first.adapter, ADAPTER_OPFS)
    const node = mutate(first.db)
    await first.journal.persist(first.db)
    const expected = snapshot(first.db, node)
    drop(first.db)

    const second = await openDurable(ZeroDb, {
      name: 'opfs-reopen',
      adapter: 'opfs',
      opfsRoot,
    })
    assert.equal(second.restored, true)
    assertRestored(expected, second.db, node)
    drop(second.db)
  }

  {
    const indexedDB = fakeIndexedDB()
    const j = await IndexedDbJournal.open('crash', { indexedDB })
    const { db } = await j.restore(ZeroDb)
    const node = db.createNode('Todo')
    db.setLww(node, 'title', 'kept')
    await j.persist(db)
    db.setLww(node, 'title', 'lost')
    drop(db)

    const j2 = await IndexedDbJournal.open('crash', { indexedDB })
    const again = await j2.restore(ZeroDb)
    assert.equal(again.db.getLww(node, 'title'), 'kept')
    assert.equal(again.opCount, 2)
    drop(again.db)
  }

  {
    const live = new ZeroDb()
    const node = mutate(live)
    const seed = live.seedHex()
    const ds = live.datastoreId()
    const bundle = JSON.parse(live.exportJson())
    const expected = snapshot(live, node)
    drop(live)

    const indexedDB = fakeIndexedDB()
    const idb = await IndexedDbJournal.open('pair-idb', { indexedDB })
    const idbDb = ZeroDb.fromSeed(seed, ds)
    idbDb.importJson(JSON.stringify(bundle))
    idbDb.replay()
    await idb.persist(idbDb)
    drop(idbDb)

    const opfsRoot = memoryOpfsRoot()
    const opfs = await OpfsJournal.open('pair-opfs', { opfsRoot })
    const opfsDb = ZeroDb.fromSeed(seed, ds)
    opfsDb.importJson(JSON.stringify(bundle))
    opfsDb.replay()
    await opfs.persist(opfsDb)
    drop(opfsDb)

    const idbAgain = await (await IndexedDbJournal.open('pair-idb', { indexedDB })).restore(ZeroDb)
    const opfsAgain = await (await OpfsJournal.open('pair-opfs', { opfsRoot })).restore(ZeroDb)
    assertRestored(expected, idbAgain.db, node)
    assertRestored(expected, opfsAgain.db, node)
    drop(idbAgain.db)
    drop(opfsAgain.db)
  }

  {
    const indexedDB = fakeIndexedDB()
    const first = await openDurable(ZeroDb, { name: 'reset-me', indexedDB, adapter: 'indexeddb' })
    const peer = first.db.peerId()
    mutate(first.db)
    await first.journal.persist(first.db)
    drop(first.db)
    await first.journal.reset()

    const second = await openDurable(ZeroDb, { name: 'reset-me', indexedDB, adapter: 'indexeddb' })
    assert.equal(second.restored, false)
    assert.notEqual(second.db.peerId(), peer)
    assert.equal(second.db.opCount(), 0)
    drop(second.db)
  }
})
