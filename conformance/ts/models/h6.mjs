// H6 closed (protocol): SIGNAL, PeerId roles, v2 AUTH, WELCOME
// version reject, datastore admission, resume-cursor. Optional
// HELLO.datastore is in AuthTranscript when present. Same rules as
// handshake.rs / webrtc/peer.mjs. Not M4a complete.

import { AUTH_WRONG_DATASTORE } from '../peer/store.mjs'
import { checkWelcomeProtocol } from '../peer/client.mjs'
import { admitDatastore, signAuthV1NonceOnly } from '../webrtc/peer.mjs'
import { encodeSignal } from '../webrtc/signal.mjs'
import { decode } from './cbor.mjs'
import {
  DOMAIN_RELAY_AUTH,
  DOMAIN_RELAY_AUTH_V1,
  ERR_AUTH_FAILED,
  MSG_ERROR,
  MSG_SIGNAL,
  authenticate,
  authTranscript,
  authTranscriptPreimage,
  decodeEnvelope,
  encodeEnvelope,
  isHandshakeServer,
  retransmit,
  signAuth,
} from './relay.mjs'

export const ERR_TARGET_NOT_CONNECTED = 0x307
export const ERR_VERSION_MISMATCH = 0x102
export const ERR_AUTH_WRONG_DATASTORE = 0x203

function hex32(s) {
  const out = new Uint8Array(32)
  const h = String(s)
  for (let i = 0; i < 32; i++) out[i] = parseInt(h.slice(i * 2, i * 2 + 2), 16)
  return out
}

function signalPayloadTagged(frame) {
  const tagged = decode(frame)
  const pl = tagged && tagged.v && tagged.v.payload && tagged.v.payload.v
  return pl && pl.payload
}

function runSignalForward(v) {
  // Use encodeSignal (CBOR bytes), not encodeEnvelope — `payload` is not a
  // registry byte_field and would otherwise become CBOR text.
  const inbound = encodeSignal(v.request_id, { target: v.target, payload: v.payload_hex })
  const forwarded = encodeSignal(v.request_id, { sender: v.sender, payload: v.payload_hex })
  const innTag = signalPayloadTagged(inbound)
  const outTag = signalPayloadTagged(forwarded)
  if (!innTag || innTag.t !== 'bytes' || innTag.hex !== String(v.payload_hex).toLowerCase()) {
    throw new Error('inbound SIGNAL.payload must be CBOR bytes')
  }
  if (!outTag || outTag.t !== 'bytes' || outTag.hex !== String(v.payload_hex).toLowerCase()) {
    throw new Error('forwarded SIGNAL.payload must be CBOR bytes')
  }
  const inn = decodeEnvelope(inbound)
  const out = decodeEnvelope(forwarded)
  if (inn.type !== MSG_SIGNAL || out.type !== MSG_SIGNAL) {
    throw new Error('SIGNAL type must be 0x42')
  }
  if (inn.payload.target !== String(v.target).toLowerCase()) {
    throw new Error('inbound SIGNAL must carry target')
  }
  if (out.payload.sender !== String(v.sender).toLowerCase()) {
    throw new Error('forwarded SIGNAL must assert sender')
  }
  if (out.payload.target != null) {
    throw new Error('forwarded SIGNAL must omit target')
  }
  if (v.expect && v.expect.message_type != null && v.expect.message_type !== MSG_SIGNAL) {
    throw new Error('expect.message_type')
  }
}

function runSignalMissing(v) {
  const frame = encodeEnvelope(MSG_ERROR, v.request_id || 1, {
    code: ERR_TARGET_NOT_CONNECTED,
    message: 'TARGET_NOT_CONNECTED',
    fatal: false,
  })
  const env = decodeEnvelope(frame)
  if (env.type !== MSG_ERROR) throw new Error('expected ERROR')
  if (env.payload.code !== ERR_TARGET_NOT_CONNECTED) {
    throw new Error(`code ${env.payload.code} != 0x307`)
  }
  if (env.payload.message !== v.expect.message) {
    throw new Error(`message ${env.payload.message}`)
  }
  if (env.payload.fatal !== v.expect.fatal) throw new Error('fatal')
  if (v.expect.code != null && v.expect.code !== ERR_TARGET_NOT_CONNECTED) {
    throw new Error('expect.code')
  }
}

function runPeerRole(v) {
  const cases = v.cases || [
    { local: v.local, remote: v.remote, handshake_server: v.expect.handshake_server },
  ]
  for (const c of cases) {
    const got = isHandshakeServer(hex32(c.local), hex32(c.remote))
    if (got !== c.handshake_server) {
      throw new Error(`role ${c.local} vs ${c.remote}: got ${got}, want ${c.handshake_server}`)
    }
  }
}

function runAuth(v) {
  const peer = hex32(v.peer_id)
  const pk = hex32(v.public_key)
  const nonce = hex32(v.nonce)
  const seed = hex32(v.secret_key)
  const t = authTranscript(peer, pk, 1, v.hello_capabilities, nonce, undefined, v.hello_datastore)
  const pre = authTranscriptPreimage(t)
  const domain = new TextDecoder().decode(pre.subarray(0, DOMAIN_RELAY_AUTH.length))
  if (domain !== 'zerodb-relay-auth-v2') {
    throw new Error(`preimage domain ${domain}`)
  }
  if (new TextDecoder().decode(pre.subarray(0, DOMAIN_RELAY_AUTH_V1.length)) === 'zerodb-relay-auth-v1') {
    throw new Error('preimage must not be v1')
  }
  if (v.kind === 'auth-v1-reject') {
    const sig = signAuthV1NonceOnly(seed, nonce)
    const err = authenticate(peer, pk, t, sig)
    if (err !== ERR_AUTH_FAILED) throw new Error(`v1 must be AUTH_FAILED, got ${err}`)
    return
  }
  if (v.kind === 'auth-swapped-ds') {
    const sig = signAuth(seed, t)
    const honest = authenticate(peer, pk, t, sig)
    if (honest !== null) throw new Error(`honest HELLO.datastore must AUTH, got ${honest}`)
    const swapped = authTranscript(peer, pk, 1, v.hello_capabilities, nonce, undefined, v.swapped_datastore)
    const err = authenticate(peer, pk, swapped, sig)
    if (err !== ERR_AUTH_FAILED) throw new Error(`swapped datastore must be AUTH_FAILED, got ${err}`)
    return
  }
  const sig = signAuth(seed, t)
  const err = authenticate(peer, pk, t, sig)
  if (err !== null) throw new Error(`v2 AUTH failed: ${err}`)
}

function runWelcomeVersion(v) {
  if (v.expect.code !== ERR_VERSION_MISMATCH) throw new Error('expect 0x102')
  if (v.expect.name !== 'VERSION_MISMATCH') throw new Error('expect VERSION_MISMATCH')
  try {
    checkWelcomeProtocol({ protocol_version: 1 })
  } catch (e) {
    throw new Error(`draft-1 WELCOME must accept: ${e.message}`)
  }
  let threw = null
  try {
    checkWelcomeProtocol({ protocol_version: v.protocol_version })
  } catch (e) {
    threw = e
  }
  if (!threw) throw new Error('WELCOME protocol_version other than 1 must reject')
  if (!String(threw.message).includes('0x102 VERSION_MISMATCH')) {
    throw new Error(`got ${threw.message}, want 0x102 VERSION_MISMATCH`)
  }
}

function runAdmit(v) {
  const bound = v.bound_ds || null
  const offered = v.offered_ds || null
  const reason = admitDatastore(bound, offered)
  if (v.expect.ok) {
    if (reason !== null) throw new Error(`admit should allow, got ${reason}`)
  } else if (reason !== v.expect.reason) {
    throw new Error(`admit got ${reason}, want ${v.expect.reason}`)
  }
  if (!v.expect.ok && v.expect.reason !== AUTH_WRONG_DATASTORE) {
    throw new Error('admission error must be AUTH_WRONG_DATASTORE')
  }
}

function runResume(v) {
  const got = retransmit(v.held, v.cursor, v.rejected || [])
  const want = (v.expect.retransmit || []).slice().sort()
  if (JSON.stringify(got) !== JSON.stringify(want)) {
    throw new Error(`retransmit ${JSON.stringify(got)} != ${JSON.stringify(want)}`)
  }
}

const kinds = {
  'signal-forward': runSignalForward,
  'signal-missing': runSignalMissing,
  'peer-role': runPeerRole,
  'auth-v2': runAuth,
  'auth-v1-reject': runAuth,
  'auth-swapped-ds': runAuth,
  'welcome-version': runWelcomeVersion,
  admit: runAdmit,
  'resume-cursor': runResume,
}

export function runH6ProfileVector(vector) {
  if (vector.type !== 'h6-profile') throw new Error(`type ${vector.type}`)
  const fn = kinds[vector.kind]
  if (!fn) throw new Error(`unknown h6-profile kind ${vector.kind}`)
  fn(vector)
}
