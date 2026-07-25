/**
 * M2 subscribe: live change notifications from local mutations, import, replay.
 */
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdirSync, rmSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { createRequire } from 'node:module'
import { setTimeout as sleep } from 'node:timers/promises'

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

test('subscribe receives local mutation events', async () => {
  const path = tempDb('sub-basic')
  let db
  try {
    db = Database.init(path)
    const events = []
    const sub = db.subscribe((e) => events.push(e))
    assert.equal(typeof sub, 'number')

    const node = db.createNode('Todo')
    db.setLww(node, 'title', 'milk')
    db.deleteNode(node)
    await sleep(20)

    assert.equal(events.length, 3)
    assert.equal(events[0].kind, 'op')
    assert.equal(events[0].method, 'createNode')
    assert.equal(events[0].node, node)
    assert.equal(events[1].method, 'setLww')
    assert.equal(events[1].key, 'title')
    assert.equal(typeof events[1].opId, 'string')
    assert.equal(events[2].method, 'deleteNode')
  } finally {
    cleanup(path, db)
  }
})

test('unsubscribe stops events; other subscribers continue', async () => {
  const path = tempDb('sub-unsub')
  let db
  try {
    db = Database.init(path)
    const a = []
    const b = []
    const subA = db.subscribe((e) => a.push(e))
    db.subscribe((e) => b.push(e))

    const node = db.createNode('Todo')
    await sleep(20)
    db.unsubscribe(subA)
    db.setLww(node, 'title', 'x')
    await sleep(20)

    assert.equal(a.length, 1)
    assert.equal(b.length, 2)
  } finally {
    cleanup(path, db)
  }
})

test('import and replay emit coarse events', async () => {
  const pathA = tempDb('sub-imp-a')
  const pathB = tempDb('sub-imp-b')
  let dbA
  let dbB
  try {
    dbA = Database.init(pathA)
    const node = dbA.createNode('Todo')
    dbA.setLww(node, 'title', 'oat')
    const bundle = dbA.exportJson()

    dbB = Database.init(pathB)
    const events = []
    dbB.subscribe((e) => events.push(e))
    dbB.importJson(bundle)
    dbB.replay()
    await sleep(20)

    assert.equal(events.length, 2)
    assert.equal(events[0].kind, 'import')
    assert.ok(events[0].accepted >= 2)
    assert.equal(events[0].skipped, 0)
    assert.equal(events[1].kind, 'replay')
  } finally {
    cleanup(pathA, dbA)
    cleanup(pathB, dbB)
  }
})
