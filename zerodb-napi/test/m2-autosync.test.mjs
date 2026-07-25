/**
 * M2 auto-sync: autoConnect keeps two stores converged (dirty-flag push +
 * interval poll) without manual connectPeer calls.
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

async function waitFor(fn, timeoutMs = 3000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    if (fn()) return true
    await sleep(50)
  }
  return fn()
}

test('autoConnect converges both directions without manual sync', async () => {
  const pathA = tempDb('auto-a')
  const pathB = tempDb('auto-b')
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

    const conn = dbB.autoConnect(`ws://127.0.0.1:${port}`, 200)
    assert.equal(typeof conn, 'number')

    // Initial session bootstraps B from A.
    assert.ok(await waitFor(() => dbB.getLww(node, 'title') === 'milk'))
    assert.equal(dbB.datastoreId(), dbA.datastoreId())

    // Mutate B: dirty flag triggers a re-sync; A converges without manual sync.
    dbB.setLww(node, 'title', 'oat milk')
    assert.ok(await waitFor(() => dbA.getLww(node, 'title') === 'oat milk'))

    // Mutate A: B converges within the poll interval.
    dbA.setLww(node, 'done', 'yes')
    assert.ok(await waitFor(() => dbB.getLww(node, 'done') === 'yes'))

    // Sync events observed on B for each session.
    assert.ok(eventsB.some((e) => e.kind === 'sync' && e.role === 'connect'))

    // disconnect stops resyncing.
    dbB.disconnect(conn)
    dbA.setLww(node, 'done', 'no')
    await sleep(600)
    assert.equal(dbB.getLww(node, 'done'), 'yes')

    dbA.stopServe()
  } finally {
    cleanup(pathA, dbA)
    cleanup(pathB, dbB)
  }
})

test('autoConnect retries with sync-error events; close stops all', async () => {
  const path = tempDb('auto-err')
  let db
  try {
    db = Database.init(path)
    db.createNode('Todo')
    const events = []
    db.subscribe((e) => events.push(e))

    db.autoConnect('ws://127.0.0.1:1', 100) // nothing listening
    assert.ok(
      await waitFor(() =>
        events.some((e) => e.kind === 'sync-error' && typeof e.retryInMs === 'number')
      )
    )

    // close() stops the background thread; the process must exit cleanly.
    db.close()
  } finally {
    cleanup(path, db)
  }
})
