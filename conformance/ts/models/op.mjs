// op-encoding / op-decode-negative / op-signature handlers per
// doc/KERNEL.md §4/§9.
//
// op-encoding: rebuild the §4.1 envelope map from the vector's logical
// description, encode with the independent CBOR codec, require byte
// agreement with canonical_hex plus decode→re-encode identity, and derive
// OpId = BLAKE3(domain ‖ bytes) with the independent JS BLAKE3 (I-6).
//
// op-signature: verify the Rust-emitted Ed25519 golden signature over the
// §4.4 sig-preimage using node:crypto, plus tamper negatives.

import { createPublicKey, verify as edVerify } from 'node:crypto';
import { Buffer } from 'node:buffer';

import { encode, decode, bytesToHex, hexToBytes } from './cbor.mjs';
import { blake3 } from './blake3.mjs';

const DOMAIN_OP_ID = new TextEncoder().encode('zerodb-op-id-v1');
const DOMAIN_OP_SIG = new TextEncoder().encode('zerodb-op-sig-v1');
// DER SPKI prefix for a raw Ed25519 public key.
const SPKI_PREFIX = hexToBytes('302a300506032b6570032100');

function opId(canonicalBytes) {
  const preimage = new Uint8Array(DOMAIN_OP_ID.length + canonicalBytes.length);
  preimage.set(DOMAIN_OP_ID, 0);
  preimage.set(canonicalBytes, DOMAIN_OP_ID.length);
  return blake3(preimage);
}

function verifySignature(pub32, message, sig64) {
  const key = createPublicKey({
    key: Buffer.concat([Buffer.from(SPKI_PREFIX), Buffer.from(pub32)]),
    format: 'der',
    type: 'spki',
  });
  return edVerify(null, Buffer.from(message), key, Buffer.from(sig64));
}

/** HEX body keys that encode as CBOR bytes (same list as Rust `json_to_cbor_body`). */
export const HEX_BODY_KEYS = new Set([
  'node',
  'edge',
  'src',
  'dst',
  'founder',
  'salt',
  'subject',
  'ds_bind',
  'grant',
  'encrypted',
  'key_id',
  'recipient',
  'eph_pk',
  'nonce',
  'wrapped',
  'device',
  'principal',
  'root_pk',
  'cert_sig',
  'revoke_of',
  'schema',
  'ir',
  'prev',
]);

/** JSON WireOp body → tagged CBOR matching `json_to_cbor_body`. */
export function jsonToBodyTagged(val, key) {
  if (val === null || val === undefined) return { t: 'null' };
  if (typeof val === 'boolean') return { t: 'bool', v: val };
  if (typeof val === 'number') {
    if (!Number.isInteger(val) || val < 0) throw new Error(`non-uint ${val}`);
    return { t: 'uint', v: val };
  }
  if (typeof val === 'string') {
    if (key && HEX_BODY_KEYS.has(key)) {
      if (!/^[0-9a-f]*$/i.test(val) || val.length % 2 !== 0) {
        throw new Error(`hex body field ${key} must be even hex`);
      }
      return { t: 'bytes', hex: val.toLowerCase() };
    }
    return { t: 'text', v: val };
  }
  if (Array.isArray(val)) {
    return { t: 'array', v: val.map((item) => jsonToBodyTagged(item)) };
  }
  if (typeof val === 'object') {
    const v = {};
    for (const [k, item] of Object.entries(val)) v[k] = jsonToBodyTagged(item, k);
    return { t: 'map', v };
  }
  throw new Error(`unsupported body ${typeof val}`);
}

export function envelopeToTagged(op) {
  const body = op.body && op.body.t ? op.body : jsonToBodyTagged(op.body);
  return {
    t: 'map',
    v: {
      v: { t: 'uint', v: op.v },
      ds: { t: 'bytes', hex: op.ds },
      ep: { t: 'uint', v: op.ep },
      author: { t: 'bytes', hex: op.author },
      ts: {
        t: 'map',
        v: {
          l: { t: 'uint', v: op.ts.l },
          p: { t: 'uint', v: op.ts.p },
        },
      },
      deps: { t: 'array', v: op.deps.map((d) => ({ t: 'bytes', hex: d })) },
      grp: op.grp === null || op.grp === undefined ? { t: 'null' } : { t: 'bytes', hex: op.grp },
      kind: { t: 'uint', v: op.kind },
      body,
    },
  };
}

export function runOpEncodingVector(vector) {
  const bytes = encode(envelopeToTagged(vector.op));
  const hex = bytesToHex(bytes);
  if (hex !== vector.canonical_hex) {
    throw new Error(
      `canonical bytes mismatch:\n  expected ${vector.canonical_hex}\n  got      ${hex}`
    );
  }
  const reencoded = bytesToHex(encode(decode(bytes)));
  if (reencoded !== hex) {
    throw new Error('decode -> re-encode is not byte-identical');
  }
  const idHex = bytesToHex(opId(bytes));
  if (idHex !== vector.op_id_hex) {
    throw new Error(`op id mismatch:\n  expected ${vector.op_id_hex}\n  got      ${idHex}`);
  }
}

export function runOpSignatureVector(vector) {
  const canonical = encode(envelopeToTagged(vector.op));
  const id = opId(canonical);
  const idHex = bytesToHex(id);
  if (idHex !== vector.op_id_hex) {
    throw new Error(`op id mismatch: expected ${vector.op_id_hex}, got ${idHex}`);
  }

  const preimage = new Uint8Array(DOMAIN_OP_SIG.length + id.length);
  preimage.set(DOMAIN_OP_SIG, 0);
  preimage.set(id, DOMAIN_OP_SIG.length);

  const pub = hexToBytes(vector.pub_hex);
  const sig = hexToBytes(vector.sig_hex);
  if (!verifySignature(pub, preimage, sig)) {
    throw new Error('golden signature failed to verify');
  }

  // Tamper negatives: a flipped signature byte and a flipped preimage byte
  // must both fail verification (I-8).
  const badSig = Uint8Array.from(sig);
  badSig[0] ^= 1;
  if (verifySignature(pub, preimage, badSig)) {
    throw new Error('tampered signature verified');
  }
  const badPreimage = Uint8Array.from(preimage);
  badPreimage[DOMAIN_OP_SIG.length] ^= 1;
  if (verifySignature(pub, badPreimage, sig)) {
    throw new Error('signature verified over a tampered op id');
  }
}

export function runOpDecodeNegativeVector(vector) {
  let error = null;
  try {
    decode(hexToBytes(vector.raw_hex));
  } catch (e) {
    error = e.message;
  }
  if (error === null) {
    throw new Error(`expected ${vector.expect_error}, but decode succeeded`);
  }
  if (error !== vector.expect_error) {
    throw new Error(`expected ${vector.expect_error}, got ${error}`);
  }
}

export function computeOpId(canonicalBytes) {
  return opId(canonicalBytes);
}

export function sigPreimage(id) {
  const preimage = new Uint8Array(DOMAIN_OP_SIG.length + id.length);
  preimage.set(DOMAIN_OP_SIG, 0);
  preimage.set(id, DOMAIN_OP_SIG.length);
  return preimage;
}

export { verifySignature };
