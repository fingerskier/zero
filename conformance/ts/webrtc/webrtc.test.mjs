/**
 * H6 closed (protocol): SIGNAL → fake DataChannel → v2 transcript
 * AUTH → WELCOME → OPS, plus admission + reconnect/resume + HELLO.datastore
 * bind. Not M4a complete.
 *
 * The DataChannel is an in-process ordered/reliable double (not wrtc).
 */
import { test } from 'node:test'
import assert from 'node:assert/strict'

import { bytesToHex } from '../models/cbor.mjs'
import {
  MSG_HELLO,
  authTranscript,
  authTranscriptPreimage,
  decodeEnvelope,
  encodeEnvelope,
  isHandshakeServer,
  optHelloDatastore,
  signAuth,
} from '../models/relay.mjs'
import { PeerStore } from '../peer/store.mjs'
import { AUTH_WRONG_DATASTORE } from '../peer/store.mjs'
import {
  ERR_AUTH_FAILED,
  ERR_AUTH_WRONG_DATASTORE,
  ERR_TARGET_NOT_CONNECTED,
  ERR_VERSION_MISMATCH,
  FakeDataChannel,
  SignalRelay,
  admitDatastore,
  connectDirect,
  encodeSignal,
  negotiateViaSignal,
  pairDataChannels,
  runNegotiated,
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
  const built = authTranscript(t.peer_id, t.public_key, 1, t.hello_capabilities, t.nonce)
  assert.deepEqual(authTranscriptPreimage(built), pre)
  const withDs = authTranscript(
    t.peer_id,
    t.public_key,
    1,
    t.hello_capabilities,
    t.nonce,
    undefined,
    new Uint8Array(32).fill(0xaa),
  )
  assert.notDeepEqual(authTranscriptPreimage(withDs), pre)
  assert.deepEqual(authTranscriptPreimage(withDs).subarray(0, DOMAIN.length), DOMAIN)
  assert.equal(optHelloDatastore(null), undefined)
  assert.equal(optHelloDatastore(undefined), undefined)
  assert.throws(() => optHelloDatastore('not-a-datastore'), /HELLO\.datastore/)
  assert.throws(() => optHelloDatastore(''), /HELLO\.datastore/)
  assert.throws(() => optHelloDatastore(new Uint8Array(7)), /HELLO\.datastore/)
  assert.throws(
    () => authTranscript(t.peer_id, t.public_key, 1, t.hello_capabilities, t.nonce, undefined, 'not-a-datastore'),
    /HELLO\.datastore/,
  )
})

function channelFor(neg, peerHex, initiatorHex) {
  return peerHex === initiatorHex ? neg.initiatorChannel : neg.answererChannel
}

test('SIGNAL → DataChannel → negotiated roles → v2 AUTH → WELCOME → OPS converges', async () => {
  const a = new PeerStore({ seed: seed(2) })
  const b = new PeerStore({ seed: seed(3) })
  const client = isHandshakeServer(a.author, b.author) ? b : a
  const server = client === a ? b : a
  client.applySchemaEpoch(schemaPin())
  const { node } = client.createNode('Todo')
  client.setLww(node, 'title', 'milk')

  const relay = new SignalRelay()
  const neg = await negotiateViaSignal(relay, a.authorHex, b.authorHex)
  assert.equal(neg.error, undefined)
  assert.equal(neg.initiatorChannel.label, 'zerodb-relay')
  assert.equal(neg.initiatorChannel.ordered, true)
  assert.equal(neg.answererChannel.label, 'zerodb-relay')

  const [left, right] = await Promise.all([
    runNegotiated(a, b.author, channelFor(neg, a.authorHex, a.authorHex), {
      joinDs: client.dsHex,
      expectedDs: client.dsHex,
    }),
    runNegotiated(b, a.author, channelFor(neg, b.authorHex, a.authorHex), {
      joinDs: client.dsHex,
      expectedDs: client.dsHex,
    }),
  ])
  const served = left.role === 'server' ? left : right
  const init = left.role === 'client' ? left : right
  assert.equal(served.role, 'server')
  assert.equal(init.role, 'client')
  assert.equal(served.phase, 'ops')
  assert.ok(served.applied >= 2)
  assert.equal(served.rejected, 0)
  assert.equal(init.sent, client.exportOps(client.dsHex).length)
  assert.equal(server.getLww(node, 'title'), 'milk')
  assert.equal(server.dsHex, client.dsHex)
})

test('handshake role is PeerId order, not RTC initiator, and either peer can serve', async () => {
  async function once(serverSeed, clientSeed, serverOffersRtc) {
    const server = new PeerStore({ seed: seed(serverSeed) })
    const client = new PeerStore({ seed: seed(clientSeed) })
    assert.equal(isHandshakeServer(server.author, client.author), true)
    assert.equal(isHandshakeServer(client.author, server.author), false)
    client.applySchemaEpoch(schemaPin())
    const { node } = client.createNode('Todo')
    client.setLww(node, 'title', 'tea')

    const relay = new SignalRelay()
    const offerId = serverOffersRtc ? server.authorHex : client.authorHex
    const answerId = serverOffersRtc ? client.authorHex : server.authorHex
    const neg = await negotiateViaSignal(relay, offerId, answerId)
    assert.equal(neg.error, undefined)

    const [s, c] = await Promise.all([
      runNegotiated(server, client.author, channelFor(neg, server.authorHex, offerId), {
        expectedDs: client.dsHex,
      }),
      runNegotiated(client, server.author, channelFor(neg, client.authorHex, offerId), {
        joinDs: client.dsHex,
      }),
    ])
    assert.equal(s.role, 'server')
    assert.equal(c.role, 'client')
    assert.equal(s.phase, 'ops')
    assert.ok(s.applied >= 2)
    assert.equal(server.getLww(node, 'title'), 'tea')
  }

  const pairs = []
  for (let i = 2; i < 24 && pairs.length < 2; i++) {
    for (let j = i + 1; j < 24 && pairs.length < 2; j++) {
      const p = new PeerStore({ seed: seed(i) })
      const q = new PeerStore({ seed: seed(j) })
      if (isHandshakeServer(p.author, q.author)) pairs.push([i, j])
      else pairs.push([j, i])
    }
  }
  assert.ok(pairs.length >= 2)
  await once(pairs[0][0], pairs[0][1], true)
  await once(pairs[1][0], pairs[1][1], false)
})

test('negotiated-role AUTH is still v2; v1 nonce-only fails closed', async () => {
  const a = new PeerStore({ seed: seed(25) })
  const b = new PeerStore({ seed: seed(26) })
  const client = isHandshakeServer(a.author, b.author) ? b : a
  const server = client === a ? b : a
  client.applySchemaEpoch(schemaPin())
  const { node } = client.createNode('Todo')
  client.setLww(node, 'title', 'secret')

  const left = new FakeDataChannel()
  const right = new FakeDataChannel()
  pairDataChannels(left, right)
  const serverCh = server === a ? left : right
  const clientCh = client === a ? left : right

  const served = runNegotiated(server, client.author, serverCh)
  let thrown = null
  const started = runNegotiated(client, server.author, clientCh, {
    signAuthFn: (seedBytes, transcript) => signAuthV1NonceOnly(seedBytes, transcript.nonce),
  }).catch((e) => {
    thrown = e
    return null
  })
  const [answer] = await Promise.all([served, started])
  assert.equal(answer.role, 'server')
  assert.equal(answer.phase, 'auth-failed')
  assert.equal(answer.code, ERR_AUTH_FAILED)
  assert.equal(thrown && thrown.code, ERR_AUTH_FAILED)
  assert.equal(server.getLww(node, 'title'), null)
  assert.notEqual(server.dsHex, client.dsHex)
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

test('HELLO protocol_version other than 1 is 0x102 before CHALLENGE', async () => {
  const a = new PeerStore({ seed: seed(15) })
  const b = new PeerStore({ seed: seed(16) })
  const left = new FakeDataChannel()
  const right = new FakeDataChannel()
  pairDataChannels(left, right)

  const served = serveDirect(b, right)
  const client = connectDirect(a, left, { helloProtocolVersion: 2 }).catch((e) => e)
  const [answer, err] = await Promise.all([served, client])
  assert.equal(answer.phase, 'version-mismatch')
  assert.equal(answer.code, ERR_VERSION_MISMATCH)
  assert.equal(err && err.code, ERR_VERSION_MISMATCH)
})

test('populated answerer binds its own datastore when expectedDs is omitted', async () => {
  const a = new PeerStore({ seed: seed(17) })
  const b = new PeerStore({ seed: seed(18) })
  a.applySchemaEpoch(schemaPin())
  const created = a.createNode('Todo')
  a.setLww(created.node, 'title', 'from-a')
  b.applySchemaEpoch(schemaPin())
  const other = b.createNode('Todo')
  b.setLww(other.node, 'title', 'keep-b')
  const bDs = b.dsHex
  const bOps = b.ops.length

  const left = new FakeDataChannel()
  const right = new FakeDataChannel()
  pairDataChannels(left, right)

  const served = serveDirect(b, right)
  const client = connectDirect(a, left, { joinDs: a.dsHex }).catch((e) => e)
  const [answer, err] = await Promise.all([served, client])

  assert.equal(answer.phase, 'wrong-datastore')
  assert.equal(answer.reason, AUTH_WRONG_DATASTORE)
  assert.equal(answer.code, ERR_AUTH_WRONG_DATASTORE)
  assert.equal(err && err.code, ERR_AUTH_WRONG_DATASTORE)
  assert.equal(b.getLww(created.node, 'title'), null)
  assert.equal(b.getLww(other.node, 'title'), 'keep-b')
  assert.equal(b.dsHex, bDs)
  assert.equal(b.ops.length, bOps)
})

test('OPS honors advertised WELCOME max_batch_ops', async () => {
  const a = new PeerStore({ seed: seed(19) })
  const b = new PeerStore({ seed: seed(20) })
  a.applySchemaEpoch(schemaPin())
  const titles = []
  for (let i = 0; i < 65; i++) {
    const { node } = a.createNode('Todo')
    a.setLww(node, 'title', `n${i}`)
    titles.push(node)
  }

  const left = new FakeDataChannel()
  const right = new FakeDataChannel()
  pairDataChannels(left, right)

  const served = serveDirect(b, right, { expectedDs: a.dsHex })
  const client = connectDirect(a, left, { joinDs: a.dsHex })
  const [answer, init] = await Promise.all([served, client])

  assert.equal(answer.phase, 'ops')
  assert.ok(init.batches >= 2)
  assert.equal(answer.batches, init.batches)
  assert.equal(answer.rejected, 0)
  assert.equal(b.getLww(titles[0], 'title'), 'n0')
  assert.equal(b.getLww(titles[64], 'title'), 'n64')
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
  const bOps = b.ops.length

  const left = new FakeDataChannel()
  const right = new FakeDataChannel()
  pairDataChannels(left, right)

  const served = serveDirect(b, right, { expectedDs: b.dsHex })
  const client = connectDirect(a, left, { joinDs: a.dsHex }).catch((e) => e)
  const [answer, err] = await Promise.all([served, client])

  assert.equal(answer.phase, 'wrong-datastore')
  assert.equal(answer.reason, AUTH_WRONG_DATASTORE)
  assert.equal(err && err.code, ERR_AUTH_WRONG_DATASTORE)
  assert.equal(b.getLww(created.node, 'title'), null)
  assert.equal(b.getLww(other.node, 'title'), 'keep-b')
  assert.notEqual(b.dsHex, a.dsHex)
  assert.equal(b.ops.length, bOps)
})

test('admitDatastore: empty adopts, populated A vs B is AUTH_WRONG_DATASTORE', () => {
  const a = 'aa'.repeat(32)
  const b = 'bb'.repeat(32)
  assert.equal(admitDatastore(null, a), null)
  assert.equal(admitDatastore(undefined, a), null)
  assert.equal(admitDatastore(a, a), null)
  assert.equal(admitDatastore(a, a.toUpperCase()), null)
  assert.equal(admitDatastore(a, b), AUTH_WRONG_DATASTORE)
})

test('joinDs not-a-datastore is rejected; empty answerer does not adopt', async () => {
  const a = new PeerStore({ seed: seed(33) })
  const b = new PeerStore({ seed: seed(34) })
  a.applySchemaEpoch(schemaPin())
  const { node } = a.createNode('Todo')
  a.setLww(node, 'title', 'garbage-ds')
  const emptyDs = b.dsHex
  const emptyOps = b.ops.length

  const left = new FakeDataChannel()
  const right = new FakeDataChannel()
  pairDataChannels(left, right)

  const served = serveDirect(b, right)
  const client = connectDirect(a, left, { joinDs: 'not-a-datastore' }).catch((e) => e)
  const [answer, err] = await Promise.all([served, client])

  assert.equal(answer.phase, 'auth-failed')
  assert.equal(answer.code, ERR_AUTH_FAILED)
  assert.equal(err && err.code, ERR_AUTH_FAILED)
  assert.equal(b.getLww(node, 'title'), null)
  assert.equal(b.dsHex, emptyDs)
  assert.equal(b.ops.length, emptyOps)
  assert.notEqual(b.dsHex, 'not-a-datastore')
  assert.notEqual(b.ds.length, 7)
})

test('joinDs null omits HELLO.datastore but OPS still carries the store ds', async () => {
  const a = new PeerStore({ seed: seed(23) })
  const b = new PeerStore({ seed: seed(24) })
  a.applySchemaEpoch(schemaPin())
  const { node } = a.createNode('Todo')
  a.setLww(node, 'title', 'ops-ds')

  const left = new FakeDataChannel()
  const right = new FakeDataChannel()
  pairDataChannels(left, right)

  const served = serveDirect(b, right)
  const client = connectDirect(a, left, { joinDs: null })
  const [answer] = await Promise.all([served, client])

  assert.equal(answer.phase, 'ops')
  assert.ok(answer.applied >= 2)
  assert.equal(answer.rejected, 0)
  assert.equal(b.dsHex, a.dsHex)
  assert.equal(b.getLww(node, 'title'), 'ops-ds')
})

test('MITM-swapped HELLO.datastore fails AUTH before OPS mix graphs', async () => {
  const a = new PeerStore({ seed: seed(31) })
  const b = new PeerStore({ seed: seed(32) })
  a.applySchemaEpoch(schemaPin())
  const { node } = a.createNode('Todo')
  a.setLww(node, 'title', 'should-not-land')
  const swapped = 'bb'.repeat(32)
  const emptyDs = b.dsHex
  const emptyOps = b.ops.length

  const left = new FakeDataChannel()
  const right = new FakeDataChannel()
  pairDataChannels(left, right)

  const origSend = left.send.bind(left)
  left.send = (data) => {
    const env = decodeEnvelope(data)
    if (env.type === MSG_HELLO && env.payload.datastore) {
      assert.notEqual(String(env.payload.datastore).toLowerCase(), swapped)
      return origSend(
        encodeEnvelope(MSG_HELLO, env.request_id, {
          ...env.payload,
          datastore: swapped,
        }),
      )
    }
    return origSend(data)
  }

  const served = serveDirect(b, right)
  const client = connectDirect(a, left, { joinDs: a.dsHex }).catch((e) => e)
  const [answer, err] = await Promise.all([served, client])

  assert.equal(answer.phase, 'auth-failed')
  assert.equal(answer.code, ERR_AUTH_FAILED)
  assert.equal(err && err.code, ERR_AUTH_FAILED)
  assert.equal(b.getLww(node, 'title'), null)
  assert.equal(b.dsHex, emptyDs)
  assert.equal(b.ops.length, emptyOps)
  assert.notEqual(b.dsHex, swapped)
  assert.notEqual(b.dsHex, a.dsHex)
})

test('empty answerer adopts HELLO.datastore A', async () => {
  const a = new PeerStore({ seed: seed(21) })
  const b = new PeerStore({ seed: seed(22) })
  a.applySchemaEpoch(schemaPin())
  const { node } = a.createNode('Todo')
  a.setLww(node, 'title', 'adopt-me')

  const left = new FakeDataChannel()
  const right = new FakeDataChannel()
  pairDataChannels(left, right)

  const served = serveDirect(b, right)
  const client = connectDirect(a, left, { joinDs: a.dsHex })
  const [answer] = await Promise.all([served, client])

  assert.equal(answer.phase, 'ops')
  assert.ok(answer.applied >= 2)
  assert.equal(answer.rejected, 0)
  assert.equal(b.dsHex, a.dsHex)
  assert.equal(b.getLww(node, 'title'), 'adopt-me')
})

test('reconnect resume: post-drop mutation converges; pre-drop ops omitted not double-applied', async () => {
  const a = new PeerStore({ seed: seed(27) })
  const b = new PeerStore({ seed: seed(28) })
  const client = isHandshakeServer(a.author, b.author) ? b : a
  const server = client === a ? b : a
  client.applySchemaEpoch(schemaPin())
  const { node } = client.createNode('Todo')
  client.setLww(node, 'title', 'before-drop')
  const preDropIds = client.exportOps(client.dsHex).map((w) => w.id)

  const relay1 = new SignalRelay()
  const neg1 = await negotiateViaSignal(relay1, a.authorHex, b.authorHex)
  const [left1, right1] = await Promise.all([
    runNegotiated(a, b.author, channelFor(neg1, a.authorHex, a.authorHex), {
      joinDs: client.dsHex,
      expectedDs: client.dsHex,
    }),
    runNegotiated(b, a.author, channelFor(neg1, b.authorHex, a.authorHex), {
      joinDs: client.dsHex,
      expectedDs: client.dsHex,
    }),
  ])
  const served1 = left1.role === 'server' ? left1 : right1
  assert.equal(served1.phase, 'ops')
  assert.equal(server.getLww(node, 'title'), 'before-drop')
  const remoteCursor = served1.frontier
  assert.ok(remoteCursor && remoteCursor.frontier)
  const serverCountAfterFirst = server.ops.length

  neg1.initiatorChannel.close()
  neg1.answererChannel.close()

  client.setLww(node, 'title', 'after-reconnect')
  const newId = client.ops[client.ops.length - 1].id

  const relay2 = new SignalRelay()
  const neg2 = await negotiateViaSignal(relay2, a.authorHex, b.authorHex)
  const [left2, right2] = await Promise.all([
    runNegotiated(a, b.author, channelFor(neg2, a.authorHex, a.authorHex), {
      joinDs: client.dsHex,
      expectedDs: client.dsHex,
      remoteCursor,
    }),
    runNegotiated(b, a.author, channelFor(neg2, b.authorHex, a.authorHex), {
      joinDs: client.dsHex,
      expectedDs: client.dsHex,
      remoteCursor,
    }),
  ])
  const served2 = left2.role === 'server' ? left2 : right2
  const init2 = left2.role === 'client' ? left2 : right2
  assert.equal(served2.phase, 'ops')
  assert.equal(server.getLww(node, 'title'), 'after-reconnect')
  assert.equal(init2.sent, 1)
  assert.deepEqual(init2.sentIds, [newId])
  assert.ok(!init2.sentIds.some((id) => preDropIds.includes(id)))
  assert.equal(
    served2.outcomes.filter((o) => o.outcome === 'DUPLICATE').length,
    0,
  )
  assert.equal(served2.applied, 1)
  assert.equal(server.ops.length, serverCountAfterFirst + 1)
  assert.equal(server.ops.filter((w) => w.id === newId).length, 1)
  for (const id of preDropIds) {
    assert.equal(server.ops.filter((w) => w.id === id).length, 1)
  }
})

test('reconnect without cursor re-sends pre-drop ops as DUPLICATE', async () => {
  const a = new PeerStore({ seed: seed(29) })
  const b = new PeerStore({ seed: seed(30) })
  const client = isHandshakeServer(a.author, b.author) ? b : a
  const server = client === a ? b : a
  client.applySchemaEpoch(schemaPin())
  const { node } = client.createNode('Todo')
  client.setLww(node, 'title', 'first')
  const preDropIds = new Set(client.exportOps(client.dsHex).map((w) => w.id))

  const left = new FakeDataChannel()
  const right = new FakeDataChannel()
  pairDataChannels(left, right)
  await Promise.all([
    runNegotiated(server, client.author, server === a ? left : right, { expectedDs: client.dsHex }),
    runNegotiated(client, server.author, client === a ? left : right, { joinDs: client.dsHex }),
  ])
  left.close()
  right.close()

  client.setLww(node, 'title', 'second')
  const newId = client.ops[client.ops.length - 1].id
  const serverCount = server.ops.length

  const left2 = new FakeDataChannel()
  const right2 = new FakeDataChannel()
  pairDataChannels(left2, right2)
  const [served, init] = await Promise.all([
    runNegotiated(server, client.author, server === a ? left2 : right2, { expectedDs: client.dsHex }),
    runNegotiated(client, server.author, client === a ? left2 : right2, { joinDs: client.dsHex }),
  ])
  assert.equal(served.phase, 'ops')
  assert.equal(server.getLww(node, 'title'), 'second')
  const dupes = served.outcomes.filter((o) => o.outcome === 'DUPLICATE')
  assert.ok(dupes.length >= preDropIds.size)
  assert.ok(dupes.every((o) => preDropIds.has(o.op_id)))
  assert.ok(served.outcomes.some((o) => o.op_id === newId && o.outcome === 'ACCEPT'))
  assert.equal(server.ops.length, serverCount + 1)
  assert.equal(init.sent, client.exportOps(client.dsHex).length)
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
