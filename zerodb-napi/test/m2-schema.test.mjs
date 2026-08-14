/**
 * M2-schema + edges + query params + promise facade.
 */
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdirSync, rmSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { createRequire } from 'node:module'
import { ZeroDB } from '../zerodb.mjs'

const require = createRequire(import.meta.url)
const { Database } = require('../index.js')

const root = join(fileURLToPath(new URL('.', import.meta.url)), '../../target/m2-napi-test')

function tempDb(name) {
  mkdirSync(root, { recursive: true })
  return join(root, `${name}-${process.pid}-${Date.now()}.sqlite`)
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

test('applySchema persists schemaId and stamps epoch', () => {
  const path = tempDb('schema')
  let db
  try {
    db = Database.init(path)
    const out = db.applySchema(
      JSON.stringify({ nodes: { Todo: { props: { title: 'lww' } } } }),
    )
    assert.equal(out.epoch, 1)
    assert.equal(out.schemaId.length, 64)
    assert.equal(db.schemaId(), out.schemaId)
    const n = db.createNode('Todo')
    db.setLww(n, 'title', 'milk')
    const bundle = JSON.parse(db.exportJson())
    assert.ok(bundle.ops.every((op) => op.ep === 1))
  } finally {
    cleanup(path, db)
  }
})

test('listNodes includes props; edges round-trip', () => {
  const path = tempDb('edges')
  let db
  try {
    db = Database.init(path)
    const a = db.createNode('Todo')
    db.setLww(a, 'title', 'milk')
    const b = db.createNode('Note')
    const e = db.createEdge('NOTATES', b, a)
    const nodes = db.listNodes()
    const todo = nodes.find((n) => n.id === a)
    assert.equal(todo.props.title, 'milk')
    const edges = db.listEdges()
    assert.equal(edges.length, 1)
    assert.equal(edges[0].id, e)
    db.deleteEdge(e)
    assert.equal(db.listEdges().length, 0)
  } finally {
    cleanup(path, db)
  }
})

test('query binds $params', () => {
  const path = tempDb('qparam')
  let db
  try {
    db = Database.init(path)
    const a = db.createNode('Todo')
    db.setLww(a, 'title', 'milk')
    const rows = db.query('MATCH (t:Todo) WHERE t.title = $want RETURN t.title', {
      want: 'milk',
    })
    assert.equal(rows.length, 1)
    assert.equal(rows[0]['t.title'], 'milk')
  } finally {
    cleanup(path, db)
  }
})

test('ZeroDB facade open/create/query', async () => {
  const path = tempDb('facade')
  let z
  try {
    z = await ZeroDB.open({ path, create: true })
    const id = await z.create('Todo', { title: 'oat' })
    const rows = await z.query('MATCH (t:Todo) RETURN t.title')
    assert.equal(rows.length, 1)
    assert.equal(rows[0]['t.title'], 'oat')
    assert.ok(id)
    await z.close()
    z = null
  } finally {
    cleanup(path, z)
  }
})
