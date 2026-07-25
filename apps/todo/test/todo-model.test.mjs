// Node smoke test for the todo app's data layer (todo-model.mjs) on a wasm
// ZeroDb store, plus sync against a NAPI Database.serve peer via the
// zero-sync.mjs driver.
//
// Requires the wasm pkg to be built first:
//   cd zerodb-wasm && npx wasm-pack build --target web --out-dir ../apps/todo/pkg
//
// Run: node --test apps/todo/test/todo-model.test.mjs

import test from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { createRequire } from 'node:module'

import * as model from '../todo-model.mjs'
import { syncOnce } from '../zero-sync.mjs'

const here = path.dirname(fileURLToPath(import.meta.url))
const require = createRequire(import.meta.url)
const { Database } = require('../../../zerodb-napi/index.js')

// Load the wasm pkg from apps/todo/pkg, falling back to zerodb-wasm/pkg.
const candidates = [
  path.join(here, '..', 'pkg'),
  path.join(here, '..', '..', '..', 'zerodb-wasm', 'pkg'),
]
const pkgDir = candidates.find(d => fs.existsSync(path.join(d, 'zerodb_wasm.js')))
assert.ok(pkgDir, 'wasm pkg missing — run: cd zerodb-wasm && npx wasm-pack build --target web --out-dir ../apps/todo/pkg')
const wasm = await import(
  new URL(`file://${path.join(pkgDir, 'zerodb_wasm.js').replaceAll('\\', '/')}`)
)
await wasm.default({
  module_or_path: fs.readFileSync(path.join(pkgDir, 'zerodb_wasm_bg.wasm')),
})
const { ZeroDb } = wasm

function tmpDb(name) {
  return path.join(
    fs.mkdtempSync(path.join(os.tmpdir(), 'zerodb-todo-')),
    `${name}.sqlite`,
  )
}

test('create/edit/toggle/tag/list/filter/delete on a wasm store', () => {
  const db = new ZeroDb()
  const a = model.createTodo(db, 'buy milk')
  const b = model.createTodo(db, 'walk dog')

  assert.deepEqual(
    model.listTodos(db).map(t => t.title).sort(),
    ['buy milk', 'walk dog'],
  )

  model.setTitle(db, a, 'buy oat milk')
  assert.equal(model.listTodos(db).find(t => t.id === a).title, 'buy oat milk')

  // Flag toggle round-trip.
  assert.equal(model.isDone(db, a), false)
  assert.equal(model.toggleDone(db, a), true)
  assert.equal(model.isDone(db, a), true)
  assert.deepEqual(model.listTodos(db, 'done').map(t => t.id), [a])
  assert.deepEqual(model.listTodos(db, 'active').map(t => t.id), [b])
  assert.equal(model.toggleDone(db, a), false)
  assert.equal(model.listTodos(db, 'done').length, 0)

  // ORSet tags.
  model.addTag(db, a, 'errand')
  model.addTag(db, a, 'food')
  model.removeTag(db, a, 'errand')
  assert.deepEqual(model.listTodos(db).find(t => t.id === a).tags, ['food'])

  // Tombstone delete hides the todo.
  model.deleteTodo(db, b)
  assert.deepEqual(model.listTodos(db).map(t => t.id), [a])
})

test('concurrent tag add+remove resolves add-wins across two wasm replicas', () => {
  const x = new ZeroDb()
  const id = model.createTodo(x, 'shared')
  model.addTag(x, id, 't')
  const y = new ZeroDb()
  y.importJson(x.exportJson())
  y.replay()

  // Concurrently: x removes 't', y re-adds 't' (fresh add unseen by x).
  model.removeTag(x, id, 't')
  model.addTag(y, id, 't')
  x.importJson(y.exportJson())
  x.replay()
  y.importJson(x.exportJson())
  y.replay()
  assert.deepEqual(model.listTodos(x).find(t => t.id === id).tags, ['t'])
  assert.deepEqual(model.listTodos(y)[0].tags, ['t'])
})

test('todo model converges with a NAPI serve peer over the sync driver', async () => {
  const node = Database.init(tmpDb('todo-peer'))
  const port = node.serve(0)
  const url = `ws://127.0.0.1:${port}`
  try {
    const web = new ZeroDb()
    const id = model.createTodo(web, 'sync me')
    model.toggleDone(web, id)
    model.addTag(web, id, 'remote')

    const first = await syncOnce(web, url)
    assert.equal(first.remoteAccepted, 4) // create + title + flag + tag
    assert.equal(web.datastoreId(), node.datastoreId())
    assert.equal(node.getLww(id, 'title'), 'sync me')
    assert.equal(node.getProp(id, 'done'), true)

    // Peer-side edits flow back into the model view.
    node.setLww(id, 'title', 'synced back')
    node.flagDisable(id, 'done')
    const second = await syncOnce(web, url)
    assert.equal(second.accepted, 2)
    const t = model.listTodos(web).find(x => x.id === id)
    assert.equal(t.title, 'synced back')
    assert.equal(t.done, false)
    assert.deepEqual(t.tags, ['remote'])
    assert.equal(node.opCount(), web.opCount())
  } finally {
    node.close()
  }
})
