/**
 * H6 first-cut evidence: SIGNAL → fake DataChannel → v2 transcript AUTH →
 * WELCOME → OPS convergence. Not H6 closed. Not M4a complete.
 *
 * The DataChannel is an in-process ordered/reliable double (not wrtc).
 */
import { test } from 'node:test'
import assert from 'node:assert/strict'

import { bytesToHex } from '../models/cbor.mjs'
import { authTranscriptPreimage, decodeEnvelope, signAuth } from '../models/relay.mjs'
import { PeerStore } from '../peer/store.mjs'
import { AUTH_WRONG_DATASTORE } from '../peer/store.mjs'
import {
  ERR_AUTH_FAILED,
  ERR_TARGET_NOT_CONNECTED,
  FakeDataChannel,
  SignalRelay,
  connectDirect,
  encodeSignal,
  negotiateViaSignal,
  pairDataChannels,
  serveDirect,
  signAuthV1NonceOnly,
} from './index.mjs'

const DOMAIN = new TextEncoder().encode('zerodb-relay-auth-v2')

function schemaPin() {
  return { name: 'Todo', nodes: { Todo: { props: { title: 'lww' } } } }
}

function seed(n) {
  return Uint8Array.from({ length: 32 }, () => n)
}

test('AuthTranscript preimage is zerodb-relay-auth-v2 (no second domain)', () => {
  const store = new PeerStore({ seed: seed(1) })
  const t = {
    peer_id: store.author,
    public_key: store.pk,
    hello_protocol_version: 1,
    hello_capabilities: ['dual-root'],
    nonce: new Uint8Array(32).fill(7),
    welcome_protocol_version: 1,
    relay_level: 2,
    welcome_capabilities: ['dual-root'],
    limits: {
      max_payload_bytes: 1048576,
      max_batch_ops: 64,
      max_batch_bytes: 16777216,
      max_subscriptions: 64,
      ops_per_second: 100,
      bytes_per_second: 10485760,
    },
  }
  const pre = authTranscriptPreimage(t)
  assert.deepEqual(pre.subarray(0, DOMAIN.length), DOMAIN)
  assert.equal(new TextDecoder().decode(pre.subarray(0, DOMAIN.length)), 'zerodb-relay-auth-v2')
  assert.notEqual(new TextDecoder().decode(pre.subarray(0, 20)), 'zerodb-relay-auth-v1')
})

test('SIGNAL → DataChannel → v2 AUTH → WELCOME → OPS converges', async () => {
  const a = new PeerStore({ seed: seed(2) })
  const b = new PeerStore({ seed: seed(3) })
  a.applySchemaEpoch(schemaPin())
  const { node } = a.createNode('Todo')
  a.setLww(node, 'title', 'milk')

  const relay = new SignalRelay()
  const neg = await negotiateViaSignal(relay, a.authorHex, b.authorHex)
  assert.equal(neg.error, undefined)
  assert.equal(neg.initiatorChannel.label, 'zerodb-relay')
  assert.equal(neg.initiatorChannel.ordered, true)
  assert.equal(neg.answererChannel.label, 'zerodb-relay')

  const served = serveDirect(b, neg.answererChannel, { expectedDs: a.dsHex })
  const client = connectDirect(a, neg.initiatorChannel, { joinDs: a.dsHex })
  const [answer, init] = await Promise.all([served, client])

  assert.equal(answer.phase, 'ops')
  assert.ok(answer.applied >= 2)
  assert.equal(answer.rejected, 0)
  assert.equal(init.sent, a.exportOps(a.dsHex).length)
  assert.equal(b.getLww(node, 'title'), 'milk')
  assert.equal(b.dsHex, a.dsHex)
})

test('v1 nonce-only AUTH fails closed (no WELCOME, no OPS)', async () => {
  const a = new PeerStore({ seed: seed(4) })
  const b = new PeerStore({ seed: seed(5) })
  a.applySchemaEpoch(schemaPin())
  const { node } = a.createNode('Todo')
  a.setLww(node, 'title', 'secret')

  const left = new FakeDataChannel()
  const right = new FakeDataChannel()
  pairDataChannels(left, right)

  const served = serveDirect(b, right)
  let thrown = null
  const client = connectDirect(a, left, {
    signAuthFn: (seedBytes, transcript) => signAuthV1NonceOnly(seedBytes, transcript.nonce),
  }).catch((e) => {
    thrown = e
    return null
  })
  const [answer] = await Promise.all([served, client])
  assert.equal(answer.phase, 'auth-failed')
  assert.equal(answer.code, ERR_AUTH_FAILED)
  assert.equal(thrown && thrown.code, ERR_AUTH_FAILED)
  assert.equal(b.getLww(node, 'title'), null)
  assert.notEqual(b.dsHex, a.dsHex)
})

test('flipped intended WELCOME limits fail closed on AUTH', async () => {
  const a = new PeerStore({ seed: seed(6) })
  const b = new PeerStore({ seed: seed(7) })
  const left = new FakeDataChannel()
  const right = new FakeDataChannel()
  pairDataChannels(left, right)

  const served = serveDirect(b, right)
  const client = connectDirect(a, left, {
    signAuthFn: (seedBytes, transcript) => {
      const bad = {
        ...transcript,
        limits: { ...transcript.limits, ops_per_second: transcript.limits.ops_per_second ^ 1 },
      }
      return signAuth(seedBytes, bad)
    },
  }).catch((e) => e)

  const [answer, err] = await Promise.all([served, client])
  assert.equal(answer.phase, 'auth-failed')
  assert.equal(err && err.code, ERR_AUTH_FAILED)
})

test('client WELCOME protocol_version reject still applies on DataChannel', async () => {
  const a = new PeerStore({ seed: seed(8) })
  const b = new PeerStore({ seed: seed(9) })
  const left = new FakeDataChannel()
  const right = new FakeDataChannel()
  pairDataChannels(left, right)

  const served = serveDirect(b, right, {
    stopAfterWelcome: true,
    welcomeOverride: {
      protocol_version: 2,
      relay_level: 2,
      capabilities: [],
      limits: {
        max_payload_bytes: 1048576,
        max_batch_ops: 64,
        max_batch_bytes: 16777216,
        max_subscriptions: 64,
        ops_per_second: 100,
        bytes_per_second: 10485760,
      },
    },
  })
  const client = connectDirect(a, left).catch((e) => e)
  const err = await client
  await served
  assert.match(String(err && err.message), /0x102 VERSION_MISMATCH/)
})

test('wrong datastore OPS fail closed', async () => {
  const a = new PeerStore({ seed: seed(10) })
  const b = new PeerStore({ seed: seed(11) })
  a.applySchemaEpoch(schemaPin())
  const created = a.createNode('Todo')
  a.setLww(created.node, 'title', 'from-a')
  b.applySchemaEpoch(schemaPin())
  const other = b.createNode('Todo')
  b.setLww(other.node, 'title', 'keep-b')

  const left = new FakeDataChannel()
  const right = new FakeDataChannel()
  pairDataChannels(left, right)

  const served = serveDirect(b, right, { expectedDs: b.dsHex })
  const client = connectDirect(a, left, { joinDs: a.dsHex })
  const [answer] = await Promise.all([served, client])

  assert.equal(answer.phase, 'ops')
  assert.equal(answer.applied, 0)
  assert.ok(answer.rejected >= 1)
  assert.ok(answer.outcomes.some((o) => o.reason === AUTH_WRONG_DATASTORE))
  assert.equal(b.getLww(created.node, 'title'), null)
  assert.equal(b.getLww(other.node, 'title'), 'keep-b')
  assert.notEqual(b.dsHex, a.dsHex)
})

test('SIGNAL to a disconnected target yields 0x307', () => {
  const relay = new SignalRelay()
  const a = new PeerStore({ seed: seed(12) })
  relay.connect(a.authorHex)
  const missing = bytesToHex(new Uint8Array(32).fill(0xab))
  const frame = encodeSignal(1, {
    target: missing,
    payload: new TextEncoder().encode('{"kind":"offer"}'),
  })
  const err = relay.handle(a.authorHex, frame)
  assert.ok(err)
  const env = decodeEnvelope(err)
  assert.equal(env.type, 0xff)
  assert.equal(env.payload.code, ERR_TARGET_NOT_CONNECTED)
  assert.equal(env.payload.message, 'TARGET_NOT_CONNECTED')
  assert.equal(env.payload.fatal, false)
})

test('SIGNAL payload is forwarded opaque with relay-asserted sender', async () => {
  const relay = new SignalRelay()
  const a = new PeerStore({ seed: seed(13) })
  const b = new PeerStore({ seed: seed(14) })
  const dest = relay.connect(b.authorHex)
  relay.connect(a.authorHex)
  const blob = new TextEncoder().encode('not-inspected-sdp-or-ice')
  const err = relay.handle(
    a.authorHex,
    encodeSignal(4, { target: b.authorHex, payload: blob }),
  )
  assert.equal(err, null)
  const env = decodeEnvelope(await dest.recv())
  assert.equal(env.type, 0x42)
  assert.equal(env.payload.sender, a.authorHex)
  assert.equal(env.payload.target, undefined)
  assert.equal(env.payload.payload, bytesToHex(blob))
})
