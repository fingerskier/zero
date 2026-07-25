/**
 * M2 query: O3 minimal MATCH/WHERE/RETURN via NAPI.
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

test('query MATCH WHERE RETURN', () => {
  const path = tempDb('query')
  let db
  try {
    db = Database.init(path)
    const a = db.createNode('Todo')
    db.setLww(a, 'title', 'milk')
    db.flagEnable(a, 'done')
    const b = db.createNode('Todo')
    db.setLww(b, 'title', 'bread')

    const rows = db.query('MATCH (t:Todo) WHERE t.done = true RETURN t.title')
    assert.equal(rows.length, 1)
    assert.equal(rows[0]['t.title'], 'milk')

    const all = db.query('MATCH (t:Todo) RETURN t.title')
    assert.equal(all.length, 2)
  } finally {
    cleanup(path, db)
  }
})

test('query parse error rejects', () => {
  const path = tempDb('query-err')
  let db
  try {
    db = Database.init(path)
    assert.throws(() => db.query('SELECT * FROM nope'), /parse/)
  } finally {
    cleanup(path, db)
  }
})
