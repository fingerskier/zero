/**
 * M2 push sessions (protocol v2 capability): autoConnect upgrades to a
 * persistent push session when the server acks push, so remote mutations
 * arrive without waiting for the poll interval; a push-disabled (old-style)
 * server still converges via the poll fallback.
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

async function waitFor(fn, timeoutMs = 5000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    if (fn()) return true
    await sleep(25)
  }
  return fn()
}

test('autoConnect push-mode converges without waiting for the interval', async () => {
  const pathA = tempDb('push-a')
  const pathB = tempDb('push-b')
  let dbA
  let dbB
  try {
    dbA = Database.init(pathA)
    const node = dbA.createNode('Todo')
    dbA.setLww(node, 'title', 'milk')

    dbB = Database.init(pathB)
    const port = dbA.serve(0)
    const eventsB = []
    dbB.subscribe((e) => eventsB.push(e))

    // Interval is enormous: any convergence after bootstrap must be push.
    const intervalMs = 60_000
    const conn = dbB.autoConnect(`ws://127.0.0.1:${port}`, intervalMs)
    assert.ok(await waitFor(() => dbB.getLww(node, 'title') === 'milk'))
    // Initial exchange emits `connect` asynchronously (ThreadsafeFunction).
    // Wait until the persistent session is acknowledged before writing, or
    // the next op can land in that first exchange and never produce a tick.
    assert.ok(
      await waitFor(() =>
        eventsB.some((e) => e.kind === 'sync' && e.role === 'connect' && e.push === true),
      ),
      'push session must be negotiated before the post-bootstrap write',
    )

    // Server-side mutation arrives at B far faster than the interval.
    const start = Date.now()
    dbA.setLww(node, 'title', 'pushed')
    assert.ok(await waitFor(() => dbB.getLww(node, 'title') === 'pushed'))
    assert.ok(
      await waitFor(() => eventsB.some((e) => e.kind === 'sync' && e.role === 'connect-push')),
      'push-session tick events should be observed',
    )
    assert.ok(Date.now() - start < intervalMs / 10, 'latency must be well under intervalMs')

    // Client-side mutation streams back over the same session.
    dbB.setLww(node, 'done', 'yes')
    assert.ok(await waitFor(() => dbA.getLww(node, 'done') === 'yes'))

    dbB.disconnect(conn)
    dbA.stopServe()
  } finally {
    cleanup(pathA, dbA)
    cleanup(pathB, dbB)
  }
})

test('new client against a push-disabled server still converges via poll', async () => {
  const pathA = tempDb('poll-a')
  const pathB = tempDb('poll-b')
  let dbA
  let dbB
  try {
    dbA = Database.init(pathA)
    const node = dbA.createNode('Todo')
    dbA.setLww(node, 'title', 'old-server')

    dbB = Database.init(pathB)
    const port = dbA.serve(0, false, false) // push capability off (old-style)
    const eventsB = []
    dbB.subscribe((e) => eventsB.push(e))

    const conn = dbB.autoConnect(`ws://127.0.0.1:${port}`, 200)
    assert.ok(await waitFor(() => dbB.getLww(node, 'title') === 'old-server'))
    assert.ok(
      await waitFor(() => eventsB.some((e) => e.kind === 'sync' && e.push === false)),
      'session should report push declined'
    )

    // Both directions still converge on the poll/dirty path.
    dbA.setLww(node, 'title', 'poll-1')
    assert.ok(await waitFor(() => dbB.getLww(node, 'title') === 'poll-1'))
    dbB.setLww(node, 'done', 'yes')
    assert.ok(await waitFor(() => dbA.getLww(node, 'done') === 'yes'))

    dbB.disconnect(conn)
    dbA.stopServe()
  } finally {
    cleanup(pathA, dbA)
    cleanup(pathB, dbB)
  }
})
