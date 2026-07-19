/**
 * M2 smoke: Node/NAPI Database mirrors LocalStore CRUD + restart + export/import.
 */
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdirSync, rmSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { createRequire } from 'node:module'

const require = createRequire(import.meta.url)
const { Database } = require('../index.js')

const root = join(fileURLToPath(new URL('.', import.meta.url)), '../../target/m2-napi-test')

function tempDb(name) {
  mkdirSync(root, { recursive: true })
  const path = join(root, `${name}-${process.pid}-${Date.now()}.sqlite`)
  return path
}

function cleanup(path, ...dbs) {
  for (const db of dbs) {
    try {
      db?.close?.()
    } catch {
      /* ignore */
    }
  }
  for (const suffix of ['', '-wal', '-shm']) {
    try {
      rmSync(path + suffix, { force: true })
    } catch {
      /* ignore */
    }
  }
}

test('init create set get restart', () => {
  const path = tempDb('basic')
  let db
  let db2
  try {
    db = Database.init(path)
    const node = db.createNode('Todo')
    db.setLww(node, 'title', 'milk')
    db.gcounterInc(node, 'views', 2)
    db.counterInc(node, 'score', 5)
    db.counterDec(node, 'score', 2)
    db.setAdd(node, 'tags', 'food')
    db.flagEnable(node, 'done')

    assert.equal(db.getLww(node, 'title'), 'milk')
    assert.equal(db.getProp(node, 'views'), 2)
    assert.equal(db.getProp(node, 'score'), 3)
    assert.deepEqual(db.getProp(node, 'tags'), ['food'])
    assert.equal(db.getProp(node, 'done'), true)
    assert.ok(db.opCount() >= 7)
    assert.equal(db.datastoreId().length, 64)

    const ds = db.datastoreId()
    db.close()
    db = null

    db2 = Database.open(path)
    assert.equal(db2.getLww(node, 'title'), 'milk')
    assert.equal(db2.getProp(node, 'score'), 3)
    assert.equal(db2.datastoreId(), ds)
  } finally {
    cleanup(path, db, db2)
  }
})

test('export import converges on empty peer', () => {
  const pathA = tempDb('export-a')
  const pathB = tempDb('export-b')
  let dbA
  let dbB
  try {
    dbA = Database.init(pathA)
    const node = dbA.createNode('Todo')
    dbA.setLww(node, 'title', 'oat')
    const bundle = dbA.exportJson()
    const ds = dbA.datastoreId()

    dbB = Database.init(pathB)
    const { accepted, skipped } = dbB.importJson(bundle)
    assert.ok(accepted >= 1)
    assert.equal(skipped, 0)
    assert.equal(dbB.getLww(node, 'title'), 'oat')
    assert.equal(dbB.datastoreId(), ds)
  } finally {
    cleanup(pathA, dbA)
    cleanup(pathB, dbB)
  }
})

test('listNodes and inspect', () => {
  const path = tempDb('inspect')
  let db
  try {
    db = Database.init(path)
    const node = db.createNode('Todo')
    db.setLww(node, 'title', 'x')
    const nodes = db.listNodes()
    assert.equal(nodes.length, 1)
    assert.equal(nodes[0].label, 'Todo')
    assert.equal(nodes[0].deleted, false)
    const report = db.inspect(path)
    assert.equal(report.ops, db.opCount())
    assert.ok(report.nodes[0].props.title === 'x')
  } finally {
    cleanup(path, db)
  }
})
