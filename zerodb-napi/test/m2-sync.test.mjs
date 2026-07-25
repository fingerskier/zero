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

test('serve default binds loopback only', async () => {
  const path = tempDb('sync-loopback')
  let db
  try {
    db = Database.init(path)
    db.createNode('Todo')
    const port = db.serve(0)
    // Loopback connect works.
    const other = Database.init(tempDb('sync-loopback-b'))
    try {
      const summary = other.connectPeer(`ws://127.0.0.1:${port}`)
      assert.ok(summary.accepted >= 1)
    } finally {
      other.close()
    }
    // Default bind must not be reachable via a non-loopback local address.
    const { networkInterfaces } = await import('node:os')
    const lanAddr = Object.values(networkInterfaces())
      .flat()
      .find((i) => i && i.family === 'IPv4' && !i.internal)?.address
    if (lanAddr) {
      const net = await import('node:net')
      await assert.rejects(
        new Promise((resolve, reject) => {
          const sock = net.connect({ host: lanAddr, port, timeout: 2000 })
          sock.on('connect', () => {
            sock.destroy()
            resolve()
          })
          sock.on('timeout', () => {
            sock.destroy()
            reject(new Error('timeout'))
          })
          sock.on('error', reject)
        })
      )
    }
    db.stopServe()
  } finally {
    cleanup(path, db)
  }
})

test('serve with allowInsecureLan=true binds all interfaces', async () => {
  const path = tempDb('sync-lan')
  let db
  try {
    db = Database.init(path)
    db.createNode('Todo')
    const port = db.serve(0, true)
    assert.ok(port > 0)
    // Reachable via a non-loopback local address when one exists.
    const { networkInterfaces } = await import('node:os')
    const lanAddr = Object.values(networkInterfaces())
      .flat()
      .find((i) => i && i.family === 'IPv4' && !i.internal)?.address
    if (lanAddr) {
      const net = await import('node:net')
      await new Promise((resolve, reject) => {
        const sock = net.connect({ host: lanAddr, port, timeout: 5000 })
        sock.on('connect', () => {
          sock.destroy()
          resolve()
        })
        sock.on('timeout', () => {
          sock.destroy()
          reject(new Error('timeout'))
        })
        sock.on('error', reject)
      })
    }
    // Loopback still works too.
    const other = Database.init(tempDb('sync-lan-b'))
    try {
      const summary = other.connectPeer(`ws://127.0.0.1:${port}`)
      assert.ok(summary.accepted >= 1)
    } finally {
      other.close()
    }
    db.stopServe()
  } finally {
    cleanup(path, db)
  }
})

test('stalled raw connection (no WS handshake) does not block the server', async () => {
  const path = tempDb('sync-stall')
  const pathB = tempDb('sync-stall-b')
  let db
  let dbB
  let rawSock
  try {
    db = Database.init(path)
    const node = db.createNode('Todo')
    db.setLww(node, 'title', 'milk')
    const port = db.serve(0)

    // Open a raw TCP connection and never speak WebSocket.
    const net = await import('node:net')
    rawSock = await new Promise((resolve, reject) => {
      const s = net.connect({ host: '127.0.0.1', port })
      s.on('connect', () => resolve(s))
      s.on('error', reject)
    })

    // While the raw socket sits stalled, a normal sync session must still
    // succeed (serve holds no store lock until after the WS handshake, and
    // each connection is served on its own thread).
    dbB = Database.init(pathB)
    const summary = dbB.connectPeer(`ws://127.0.0.1:${port}`)
    assert.ok(summary.accepted >= 2)
    dbB.replay()
    assert.equal(dbB.getLww(node, 'title'), 'milk')

    db.stopServe()
  } finally {
    try {
      rawSock?.destroy()
    } catch {
      /* ignore */
    }
    cleanup(path, db)
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
