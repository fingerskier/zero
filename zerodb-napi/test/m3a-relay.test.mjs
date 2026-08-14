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
      const m = buf.match(/listening on (ws:\/\/\S+)/)
      if (m) {
        ready = true
        clearTimeout(timer)
        proc.stderr.off('data', onData)
        proc.stdout.off('data', onData)
        resolve({ proc, url: m[1].replace(/\/$/, '') })
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
  assert.equal(dbB.datastoreId(), dbA.datastoreId())
  dbB.replay()
  assert.equal(dbB.getLww(node, 'title'), 'milk')
})
