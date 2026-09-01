/**
 * Live RELAY 0.2 smoke: TS peer writes SchemaEpoch + data; a second TS
 * session catch-up-sees them through zerodb-relay. Not the H9 harness.
 */
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawn, spawnSync } from 'node:child_process'
import { mkdirSync, rmSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

import { PeerStore } from './store.mjs'
import { sync } from './client.mjs'
import { connectRelay } from './ws.mjs'

const repo = join(fileURLToPath(new URL('.', import.meta.url)), '../../..')
const root = join(repo, 'target/m3c-ts-peer-test')

function tempPath(name) {
  mkdirSync(root, { recursive: true })
  return join(root, `${name}-${process.pid}-${Date.now()}.sqlite`)
}

function cleanup(path) {
  for (const suffix of ['', '-wal', '-shm']) {
    try {
      rmSync(path + suffix, { force: true })
    } catch {
      /* ignore */
    }
  }
}

function relayBin() {
  return join(repo, 'target', 'debug', 'zerodb-relay')
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
      const m = buf.match(/listening on ws:\/\/\S+:(\d+)/)
      if (m) {
        ready = true
        clearTimeout(timer)
        proc.stderr.off('data', onData)
        proc.stdout.off('data', onData)
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

async function session(url, store, joinDs) {
  const transport = await connectRelay(url)
  try {
    return await sync(store, joinDs, (frame) => transport.request(frame))
  } finally {
    transport.close()
  }
}

test('TS peer writes SchemaEpoch + data; second session catch-up-sees them', async (t) => {
  ensureRelayBuilt()
  const relayDb = tempPath('relay')
  let relay
  t.after(() => {
    try {
      relay?.proc.kill()
    } catch {
      /* ignore */
    }
    cleanup(relayDb)
  })

  relay = await startRelay(relayDb)
  const a = new PeerStore()
  a.applySchemaEpoch({ nodes: { Todo: { props: { title: 'lww' } } } })
  const { node } = a.createNode('Todo')
  a.setLww(node, 'title', 'milk')

  const pushed = await session(relay.url, a, null)
  assert.ok(pushed.sent >= 3, `A must submit local ops, sent=${pushed.sent}`)
  assert.equal(pushed.ack_rejected, 0, JSON.stringify(pushed))
  assert.equal(pushed.ack_accepted + pushed.ack_duplicate, pushed.sent, JSON.stringify(pushed))

  const b = new PeerStore()
  assert.notEqual(b.datastoreIdHex(), a.datastoreIdHex())
  const caught = await session(relay.url, b, a.datastoreIdHex())
  assert.ok(caught.received >= 3, `B must receive A's ops, received=${caught.received}`)
  assert.ok(caught.applied >= 3, `B must apply A's ops, applied=${caught.applied}`)
  assert.ok(caught.merkle_leaves >= 1, 'catch-up must walk a Merkle leaf')
  assert.equal(b.datastoreIdHex(), a.datastoreIdHex())
  assert.equal(b.schemaEpoch, 1)
  assert.equal(b.getLww(node, 'title'), 'milk')
})
