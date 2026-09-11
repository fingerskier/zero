// In-process SIGNAL (0x42) double. Same shapes as RELAY-SPEC §4.5:
// peer→relay {target, payload}; forwarded {sender, payload}.
// Relay neither inspects nor distinguishes SDP vs ICE. Target missing → 0x307.

import { encode, bytesToHex } from '../models/cbor.mjs'
import { decodeEnvelope, MSG_ERROR, MSG_SIGNAL } from '../models/relay.mjs'

export const ERR_TARGET_NOT_CONNECTED = 0x307

function asHex32(id) {
  if (id instanceof Uint8Array) return bytesToHex(id)
  return String(id).toLowerCase()
}

function asPayloadHex(payload) {
  if (payload instanceof Uint8Array) return bytesToHex(payload)
  if (typeof payload === 'string') {
    if (/^[0-9a-f]*$/i.test(payload) && payload.length % 2 === 0) return payload.toLowerCase()
    return bytesToHex(new TextEncoder().encode(payload))
  }
  throw new Error('SIGNAL.payload must be bytes or hex')
}

/** CBOR SIGNAL with byte `payload` (and target or sender). */
export function encodeSignal(requestId, fields) {
  const pl = {
    payload: { t: 'bytes', hex: asPayloadHex(fields.payload) },
  }
  if (fields.target != null) {
    pl.target = { t: 'bytes', hex: asHex32(fields.target) }
  }
  if (fields.sender != null) {
    pl.sender = { t: 'bytes', hex: asHex32(fields.sender) }
  }
  return encode({
    t: 'map',
    v: {
      type: { t: 'uint', v: MSG_SIGNAL },
      request_id: { t: 'uint', v: requestId },
      payload: { t: 'map', v: pl },
    },
  })
}

export function encodeError(requestId, code, message, fatal) {
  return encode({
    t: 'map',
    v: {
      type: { t: 'uint', v: MSG_ERROR },
      request_id: { t: 'uint', v: requestId },
      payload: {
        t: 'map',
        v: {
          code: { t: 'uint', v: code },
          message: { t: 'text', v: message },
          fatal: { t: 'bool', v: fatal },
        },
      },
    },
  })
}

export function signalingBytes(obj) {
  return new TextEncoder().encode(JSON.stringify(obj))
}

export function signalingObject(bytesOrHex) {
  const bytes = bytesOrHex instanceof Uint8Array
    ? bytesOrHex
    : Uint8Array.from(
        String(bytesOrHex)
          .match(/.{2}/g)
          .map((h) => parseInt(h, 16)),
      )
  return JSON.parse(new TextDecoder().decode(bytes))
}

/**
 * Authenticated-session SIGNAL forwarder. Does not speak HELLO/AUTH;
 * callers register after they would have been welcome'd on a real relay.
 */
export class SignalRelay {
  constructor() {
    this.sessions = new Map()
  }

  connect(peerId) {
    const key = asHex32(peerId)
    const inbox = []
    const waiters = []
    const session = {
      peerIdHex: key,
      push(frame) {
        if (waiters.length) waiters.shift()(frame)
        else inbox.push(frame)
      },
      async recv() {
        if (inbox.length) return inbox.shift()
        return await new Promise((resolve) => {
          waiters.push(resolve)
        })
      },
    }
    this.sessions.set(key, session)
    return session
  }

  disconnect(peerId) {
    this.sessions.delete(asHex32(peerId))
  }

  /** Handle one peer→relay SIGNAL. Returns ERROR frame or null. */
  handle(fromPeerId, frame) {
    const env = decodeEnvelope(frame)
    if (env.type !== MSG_SIGNAL) throw new Error(`expected SIGNAL, got ${env.type}`)
    const target = env.payload && env.payload.target
    if (target == null) {
      return encodeError(env.request_id, 0x400, 'BAD_SIGNAL', false)
    }
    const dest = this.sessions.get(asHex32(target))
    if (!dest) {
      return encodeError(env.request_id, ERR_TARGET_NOT_CONNECTED, 'TARGET_NOT_CONNECTED', false)
    }
    dest.push(
      encodeSignal(env.request_id, {
        sender: fromPeerId,
        payload: env.payload.payload,
      }),
    )
    return null
  }
}
