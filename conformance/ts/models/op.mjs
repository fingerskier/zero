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

function envelopeToTagged(op) {
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
      grp: op.grp === null ? { t: 'null' } : { t: 'bytes', hex: op.grp },
      kind: { t: 'uint', v: op.kind },
      body: op.body,
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
