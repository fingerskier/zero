// Ed25519 identity helpers for the independent TS wire peer.
// Reuses the same PKCS8/SPKI prefixes as models/relay.mjs and models/op.mjs.
// MUST NOT import zerodb-napi or the Node SDK.

import { createPrivateKey, createPublicKey, sign as edSign, randomBytes } from 'node:crypto'
import { Buffer } from 'node:buffer'

import { hexToBytes, bytesToHex } from '../models/cbor.mjs'
import { blake3 } from '../models/blake3.mjs'
import { sigPreimage } from '../models/op.mjs'

const SPKI_PREFIX = hexToBytes('302a300506032b6570032100')
const PKCS8_PREFIX = hexToBytes('302e020100300506032b657004220420')
const DOMAIN_LOCAL_DS = new TextEncoder().encode('zerodb-local-ds-v1')

export function concatBytes(...parts) {
  let n = 0
  for (const p of parts) n += p.length
  const out = new Uint8Array(n)
  let o = 0
  for (const p of parts) {
    out.set(p, o)
    o += p.length
  }
  return out
}

export function ed25519Private(seed) {
  return createPrivateKey({
    key: Buffer.concat([Buffer.from(PKCS8_PREFIX), Buffer.from(seed)]),
    format: 'der',
    type: 'pkcs8',
  })
}

export function publicKeyFromSeed(seed) {
  const pub = createPublicKey(ed25519Private(seed))
  const der = pub.export({ type: 'spki', format: 'der' })
  return new Uint8Array(der.subarray(SPKI_PREFIX.length))
}

export function signBytes(seed, message) {
  return new Uint8Array(edSign(null, Buffer.from(message), ed25519Private(seed)))
}

export function signOp(seed, opId) {
  return signBytes(seed, sigPreimage(opId))
}

export function generateIdentity(seed) {
  const s = seed instanceof Uint8Array ? seed : new Uint8Array(randomBytes(32))
  const pk = publicKeyFromSeed(s)
  const author = blake3(pk)
  return { seed: s, pk, author, authorHex: bytesToHex(author), pkHex: bytesToHex(pk) }
}

export function deriveLocalDatastore(author, salt) {
  const s = salt instanceof Uint8Array ? salt : new Uint8Array(randomBytes(16))
  const ds = blake3(concatBytes(DOMAIN_LOCAL_DS, author, s))
  return { ds, salt: s, dsHex: bytesToHex(ds) }
}

export { bytesToHex, hexToBytes }
