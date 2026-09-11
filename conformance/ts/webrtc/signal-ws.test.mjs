/**
 * Live zerodb-relay WebSocket SIGNAL fanout. Two authenticated sockets;
 * forwarded {sender, payload} arrives on the other socket. Target gone
 * → 0x307. Not H6 closed. Not M4a complete.
 */
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawn, spawnSync } from 'node:child_process'
import { mkdirSync, rmSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

import { bytesToHex } from '../models/cbor.mjs'
import {
  MSG_AUTH,
  MSG_CHALLENGE,
  MSG_ERROR,
  MSG_HELLO,
  MSG_SIGNAL,
  MSG_WELCOME,
  RELAY_CAPS,
  authTranscript,
  decodeEnvelope,
  encodeEnvelope,
  signAuth,
} from '../models/relay.mjs'
import { PeerStore } from '../peer/store.mjs'
import { connectRelay } from '../peer/ws.mjs'
import { encodeSignal, ERR_TARGET_NOT_CONNECTED } from './signal.mjs'

const repo = join(fileURLToPath(new URL('.', import.meta.url)), '../../..')
const root = join(repo, 'target/h6-signal-ws-test')

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
  const built = spawnSync('cargo', ['build', '-p', 'zerodb-relay', '--locked'], {
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

async function handshake(store, t) {
  t.sendBinary(
    encodeEnvelope(MSG_HELLO, 1, {
      peer_id: store.authorHex,
      public_key: store.pkHex,
      protocol_version: 1,
      capabilities: RELAY_CAPS.slice(),
    }),
  )
  const challenge = decodeEnvelope(await t.readBinary())
  assert.equal(challenge.type, MSG_CHALLENGE)
  const transcript = authTranscript(store.author, store.pk, 1, RELAY_CAPS, challenge.payload.nonce)
  const sig = signAuth(store.seed, transcript)
  t.sendBinary(encodeEnvelope(MSG_AUTH, 2, { signature: bytesToHex(sig) }))
  const welcome = decodeEnvelope(await t.readBinary())
  assert.equal(welcome.type, MSG_WELCOME)
}

function seed(n) {
  return Uint8Array.from({ length: 32 }, () => n)
}

test('live WS SIGNAL arrives on the other socket with relay-asserted sender', async (t) => {
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
  const a = new PeerStore({ seed: seed(40) })
  const b = new PeerStore({ seed: seed(41) })
  const tA = await connectRelay(relay.url)
  const tB = await connectRelay(relay.url)
  t.after(() => {
    tA.close()
    tB.close()
  })
  await handshake(a, tA)
  await handshake(b, tB)

  const blob = new TextEncoder().encode('not-inspected-sdp-or-ice')
  tA.sendBinary(encodeSignal(9, { target: b.authorHex, payload: blob }))
  const env = decodeEnvelope(await tB.readBinary())
  assert.equal(env.type, MSG_SIGNAL)
  assert.equal(env.request_id, 9)
  assert.equal(env.payload.sender, a.authorHex)
  assert.equal(env.payload.target, undefined)
  assert.equal(env.payload.payload, bytesToHex(blob))
})

test('live WS SIGNAL to a disconnected peer is 0x307', async (t) => {
  ensureRelayBuilt()
  const relayDb = tempPath('gone')
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
  const a = new PeerStore({ seed: seed(42) })
  const b = new PeerStore({ seed: seed(43) })
  const tA = await connectRelay(relay.url)
  const tB = await connectRelay(relay.url)
  t.after(() => tA.close())
  await handshake(a, tA)
  await handshake(b, tB)
  tB.close()

  const deadline = Date.now() + 2000
  let last = null
  while (Date.now() < deadline) {
    tA.sendBinary(
      encodeSignal(7, {
        target: b.authorHex,
        payload: new TextEncoder().encode('opaque'),
      }),
    )
    const env = decodeEnvelope(await tA.readBinary())
    last = env
    if (
      env.type === MSG_ERROR &&
      env.payload.code === ERR_TARGET_NOT_CONNECTED &&
      env.payload.message === 'TARGET_NOT_CONNECTED'
    ) {
      assert.equal(env.request_id, 7)
      assert.equal(env.payload.fatal, false)
      return
    }
  }
  assert.fail(`expected 0x307 after target closed, last=${JSON.stringify(last && last.payload)}`)
})
