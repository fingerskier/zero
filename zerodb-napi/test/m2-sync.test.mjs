/**
 * M2 sync: GunDB-style URL sync — serve a WebSocket listener, connectPeer to
 * converge two stores in one session (protocol v2, two-way).
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

test('serve + connectPeer converges two stores both ways', async () => {
  const pathA = tempDb('sync-a')
  const pathB = tempDb('sync-b')
  let dbA
  let dbB
  try {
    dbA = Database.init(pathA)
    const node = dbA.createNode('Todo')
    dbA.setLww(node, 'title', 'milk')

    dbB = Database.init(pathB)

    const eventsA = []
    const eventsB = []
    dbA.subscribe((e) => eventsA.push(e))
    dbB.subscribe((e) => eventsB.push(e))

    const port = dbA.serve(0)
    assert.equal(typeof port, 'number')
    assert.ok(port > 0)

    // First pull: B bootstraps from A (adopts A's datastore id).
    const first = dbB.connectPeer(`ws://127.0.0.1:${port}`)
    assert.ok(first.accepted >= 2)
    assert.equal(first.sent, 0)
    assert.equal(dbB.datastoreId(), dbA.datastoreId())
    dbB.replay()
    assert.equal(dbB.getLww(node, 'title'), 'milk')

    // B mutates; second session pushes B's ops back to A (two-way).
    dbB.setLww(node, 'title', 'oat milk')
    const second = dbB.connectPeer(`ws://127.0.0.1:${port}`)
    assert.equal(second.accepted, 0)
    assert.ok(second.sent >= 1)
    assert.ok(second.remoteAccepted >= 1)
    dbA.replay()
    assert.equal(dbA.getLww(node, 'title'), 'oat milk')
    assert.equal(dbA.opCount(), dbB.opCount())

    // Sync events observed on both sides.
    await sleep(50)
    const syncA = eventsA.filter((e) => e.kind === 'sync')
    const syncB = eventsB.filter((e) => e.kind === 'sync')
    assert.ok(syncA.length >= 2)
    assert.equal(syncA[0].role, 'serve')
    assert.equal(typeof syncA[0].peerAddr, 'string')
    assert.ok(syncB.length >= 2)
    assert.equal(syncB[0].role, 'connect')

    dbA.stopServe()
  } finally {
    cleanup(pathA, dbA)
    cleanup(pathB, dbB)
  }
})

test('stopServe stops the listener; close stops serving too', async () => {
  const pathA = tempDb('sync-stop')
  const pathB = tempDb('sync-stop-b')
  let dbA
  let dbB
  try {
    dbA = Database.init(pathA)
    dbA.createNode('Todo')
    dbB = Database.init(pathB)

    const port = dbA.serve(0)
    dbA.stopServe()
    assert.throws(() => dbB.connectPeer(`ws://127.0.0.1:${port}`))

    // Serving again works; close() also tears the listener down.
    const port2 = dbA.serve(0)
    const summary = dbB.connectPeer(`ws://127.0.0.1:${port2}`)
    assert.ok(summary.accepted >= 1)
    dbA.close()
    assert.throws(() => dbB.connectPeer(`ws://127.0.0.1:${port2}`))
  } finally {
    cleanup(pathA, dbA)
    cleanup(pathB, dbB)
  }
})

test('connectPeer rejects bad urls and unreachable peers', () => {
  const path = tempDb('sync-bad')
  let db
  try {
    db = Database.init(path)
    assert.throws(() => db.connectPeer('not-a-url'))
    assert.throws(() => db.connectPeer('ws://127.0.0.1:1')) // nothing listening
  } finally {
    cleanup(path, db)
  }
})
