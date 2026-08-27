/**
 * M3a client E2E: two NAPI stores + live `zerodb-relay` process.
 */
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawn, spawnSync } from 'node:child_process'
import { mkdirSync, rmSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { createRequire } from 'node:module'

const require = createRequire(import.meta.url)
const { Database } = require('../index.js')

const repo = join(fileURLToPath(new URL('.', import.meta.url)), '../..')
const root = join(repo, 'target/m3a-napi-test')

function tempPath(name) {
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

function relayBin() {
  const name = process.platform === 'win32' ? 'zerodb-relay.exe' : 'zerodb-relay'
  return join(repo, 'target', 'debug', name)
}

function ensureRelayBuilt() {
  const built = spawnSync('cargo', ['build', '-p', 'zerodb-relay'], {
    cwd: repo,
    encoding: 'utf8',
  })
  if (built.status !== 0) {
    throw new Error(`cargo build -p zerodb-relay failed:\n${built.stderr || built.stdout}`)
  }
}

function startRelay(dbPath) {
  const proc = spawn(relayBin(), ['--path', dbPath, '--bind', '127.0.0.1:0'], {
    cwd: repo,
  })
  return new Promise((resolve, reject) => {
    let ready = false
    const timer = setTimeout(() => {
      if (!ready) {
        proc.kill()
        reject(new Error('zerodb-relay did not print a listen address'))
      }
    }, 15000)
    let buf = ''
    const onData = (chunk) => {
      buf += chunk.toString()
      // Require the port digits so a Windows-chunked `ws://127.0.0.1:` prefix
      // is not treated as a complete listen line.
      const m = buf.match(/listening on ws:\/\/\S+:(\d+)/)
      if (m) {
        ready = true
        clearTimeout(timer)
        proc.stderr.off('data', onData)
        proc.stdout.off('data', onData)
        // Never feed the printed host to tungstenite (Windows IPv6 / mapped Display).
        resolve({ proc, url: `ws://127.0.0.1:${m[1]}` })
      }
    }
    proc.stderr.on('data', onData)
    proc.stdout.on('data', onData)
    proc.once('error', (e) => {
      if (!ready) {
        clearTimeout(timer)
        reject(e)
      }
    })
    proc.once('exit', (code) => {
      if (!ready) {
        clearTimeout(timer)
        reject(new Error(`zerodb-relay exited ${code} before listen: ${buf}`))
      }
    })
  })
}

test('two NAPI peers converge through zerodb-relay', async (t) => {
  ensureRelayBuilt()
  const relayDb = tempPath('relay')
  const pathA = tempPath('a')
  const pathB = tempPath('b')
  let relay
  let dbA
  let dbB
  t.after(() => {
    try {
      relay?.proc.kill()
    } catch {
      /* ignore */
    }
    cleanup(pathA, dbA)
    cleanup(pathB, dbB)
    cleanup(relayDb)
  })

  relay = await startRelay(relayDb)
  dbA = Database.init(pathA)
  const node = dbA.createNode('Todo')
  dbA.setLww(node, 'title', 'milk')

  const pushed = dbA.connectRelay(relay.url)
  assert.ok(pushed.sent >= 2, `A sent ${pushed.sent}`)
  assert.equal(pushed.ackAccepted, pushed.sent)

  dbB = Database.init(pathB)
  const caught = dbB.connectRelay(relay.url, dbA.datastoreId())
  assert.ok(caught.received >= 2, `B received ${caught.received}`)
  assert.ok(caught.merkleLeaves >= 1, `B walked ${caught.merkleLeaves} leaves`)
  assert.equal(dbB.datastoreId(), dbA.datastoreId())
  dbB.replay()
  assert.equal(dbB.getLww(node, 'title'), 'milk')
})

function sortedTags(db, node) {
  return [...(db.getProp(node, 'tags') ?? [])].sort()
}

function opIds(db) {
  const bundle = JSON.parse(db.exportJson())
  return [...bundle.ops.map((o) => o.id)].sort()
}

test('concurrent CRDTs converge through zerodb-relay', async (t) => {
  ensureRelayBuilt()
  const relayDb = tempPath('relay-e2')
  const pathA = tempPath('e2-a')
  const pathB = tempPath('e2-b')
  let relay
  let dbA
  let dbB
  t.after(() => {
    try {
      relay?.proc.kill()
    } catch {
      /* ignore */
    }
    cleanup(pathA, dbA)
    cleanup(pathB, dbB)
    cleanup(relayDb)
  })

  relay = await startRelay(relayDb)
  dbA = Database.init(pathA)
  const node = dbA.createNode('Todo')
  dbA.setLww(node, 'title', 'seed')
  dbA.setAdd(node, 'tags', 'food')
  dbA.setAdd(node, 'tags', 'urgent')
  dbA.flagEnable(node, 'done')
  dbA.counterInc(node, 'voteScore', 2)
  dbA.connectRelay(relay.url)

  dbB = Database.init(pathB)
  const joined = dbB.connectRelay(relay.url, dbA.datastoreId())
  assert.ok(joined.received >= 2, `B join received ${joined.received}`)
  dbB.replay()

  dbA.setLww(node, 'title', 'race-a')
  dbB.setLww(node, 'title', 'race-b')
  dbA.setAdd(node, 'tags', 'food')
  dbB.setRemove(node, 'tags', 'food')
  dbA.flagEnable(node, 'done')
  dbB.flagDisable(node, 'done')
  dbA.counterInc(node, 'voteScore', 4)
  dbB.counterDec(node, 'voteScore', 1)
  dbB.counterInc(node, 'voteScore', 3)

  dbA.connectRelay(relay.url)
  dbB.connectRelay(relay.url)
  dbA.connectRelay(relay.url)
  dbA.replay()
  dbB.replay()

  const title = dbA.getLww(node, 'title')
  assert.equal(title, dbB.getLww(node, 'title'))
  assert.ok(title === 'race-a' || title === 'race-b', `LWW winner ${title}`)
  assert.deepEqual(sortedTags(dbA, node), ['food', 'urgent'])
  assert.deepEqual(sortedTags(dbB, node), ['food', 'urgent'])
  assert.equal(dbA.getProp(node, 'done'), true)
  assert.equal(dbB.getProp(node, 'done'), true)
  assert.equal(dbA.getProp(node, 'voteScore'), 8)
  assert.equal(dbB.getProp(node, 'voteScore'), 8)

  const rematch = dbA.connectRelay(relay.url)
  assert.equal(rematch.received, 0)
  assert.deepEqual(opIds(dbA), opIds(dbB))
})

test('three NAPI peers: C catch-up after B close/reopen', async (t) => {
  ensureRelayBuilt()
  const relayDb = tempPath('relay-e3')
  const pathA = tempPath('e3-a')
  const pathB = tempPath('e3-b')
  const pathC = tempPath('e3-c')
  let relay
  let dbA
  let dbB
  let dbC
  t.after(() => {
    try {
      relay?.proc.kill()
    } catch {
      /* ignore */
    }
    cleanup(pathA, dbA)
    cleanup(pathB, dbB)
    cleanup(pathC, dbC)
    cleanup(relayDb)
  })

  relay = await startRelay(relayDb)
  dbA = Database.init(pathA)
  const node = dbA.createNode('Todo')
  dbA.setLww(node, 'title', 'seed')
  for (let i = 0; i < 8; i++) {
    dbA.setLww(node, 'note', `a-seed-${i}`)
  }
  dbA.connectRelay(relay.url)
  const ds = dbA.datastoreId()

  dbB = Database.init(pathB)
  dbB.connectRelay(relay.url, ds)
  dbB.replay()

  dbC = Database.init(pathC)
  const joined = dbC.connectRelay(relay.url, ds)
  assert.ok(joined.received >= 10, `C join received ${joined.received}`)
  dbC.replay()
  assert.equal(dbC.datastoreId(), ds)

  for (let i = 0; i < 10; i++) {
    dbA.setLww(node, 'note', `a-live-${i}`)
  }
  for (let i = 0; i < 8; i++) {
    dbB.setAdd(node, 'tags', `b${i}`)
  }
  dbA.connectRelay(relay.url)

  dbB.counterInc(node, 'voteScore', 5)
  dbB.setLww(node, 'pending', 'unsynced')
  dbB.close()
  dbB = Database.open(pathB)
  assert.equal(dbB.datastoreId(), ds)
  dbB.replay()
  assert.equal(dbB.getLww(node, 'pending'), 'unsynced')
  const pushed = dbB.connectRelay(relay.url)
  assert.ok(pushed.sent >= 2, `reopened B sent ${pushed.sent}`)
  assert.equal(pushed.ackAccepted + pushed.ackDuplicate, pushed.sent)

  const caught = dbC.connectRelay(relay.url, ds)
  assert.ok(caught.received >= 20, `C catch-up received ${caught.received}`)
  dbC.replay()
  assert.equal(dbC.getLww(node, 'pending'), 'unsynced')
  assert.equal(dbC.getProp(node, 'voteScore'), 5)
  assert.deepEqual(opIds(dbB), opIds(dbC))

  const aCaught = dbA.connectRelay(relay.url)
  assert.ok(aCaught.received >= 10, `A catch-up received ${aCaught.received}`)
  dbA.replay()
  assert.equal(dbA.getLww(node, 'pending'), 'unsynced')
  assert.deepEqual(opIds(dbA), opIds(dbB))
  assert.deepEqual(opIds(dbA), opIds(dbC))

  const resume = dbC.connectRelay(relay.url, ds)
  assert.equal(resume.received, 0)
})
