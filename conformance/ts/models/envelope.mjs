// Encrypted-value envelope handler per doc/KERNEL.md §7 (vector type
// "envelope"). Seals with the vector's fixed key/nonce and requires byte
// agreement with the Rust-emitted golden; opens it back; then runs the
// negative matrix: every AAD component flipped, truncation, unknown
// version, and a wrong key id must fail (I-10).

import { bytesToHex, hexToBytes } from './cbor.mjs';
import { blake3 } from './blake3.mjs';
import { xchachaSeal, xchachaOpen } from './xchacha.mjs';

const DOMAIN = new TextEncoder().encode('zerodb-value-aad-v1');
const VERSION = 1;
const KEY_ID_LEN = 16;
const NONCE_LEN = 24;

function aadFor(ctx) {
  const path = new TextEncoder().encode(ctx.path);
  const aad = new Uint8Array(DOMAIN.length + 32 + 32 + 8 + path.length);
  let o = 0;
  aad.set(DOMAIN, o);
  o += DOMAIN.length;
  aad.set(ctx.ds, o);
  o += 32;
  aad.set(ctx.opId, o);
  o += 32;
  const epBytes = new Uint8Array(8);
  new DataView(epBytes.buffer).setBigUint64(0, BigInt(ctx.ep), false);
  aad.set(epBytes, o);
  o += 8;
  aad.set(path, o);
  return aad;
}

function seal(key, nonce, ctx, plaintext) {
  const ct = xchachaSeal(key, nonce, aadFor(ctx), plaintext);
  const out = new Uint8Array(1 + KEY_ID_LEN + NONCE_LEN + ct.length);
  out[0] = VERSION;
  out.set(blake3(key).subarray(0, KEY_ID_LEN), 1);
  out.set(nonce, 1 + KEY_ID_LEN);
  out.set(ct, 1 + KEY_ID_LEN + NONCE_LEN);
  return out;
}

// Returns { plaintext } or { error: name } per the KERNEL §7 outcomes.
function open(key, envelope, ctx) {
  if (envelope.length < 1 + KEY_ID_LEN + NONCE_LEN + 16) return { error: 'Truncated' };
  if (envelope[0] !== VERSION) return { error: 'UnknownVersion' };
  const keyId = blake3(key).subarray(0, KEY_ID_LEN);
  for (let i = 0; i < KEY_ID_LEN; i += 1) {
    if (envelope[1 + i] !== keyId[i]) return { error: 'UnknownKeyId' };
  }
  const nonce = envelope.subarray(1 + KEY_ID_LEN, 1 + KEY_ID_LEN + NONCE_LEN);
  const ct = envelope.subarray(1 + KEY_ID_LEN + NONCE_LEN);
  const plaintext = xchachaOpen(key, nonce, aadFor(ctx), ct);
  return plaintext === null ? { error: 'DecryptFailed' } : { plaintext };
}

export function runEnvelopeVector(vector) {
  const key = hexToBytes(vector.key_hex);
  const nonce = hexToBytes(vector.nonce_hex);
  const ctx = {
    ds: hexToBytes(vector.ds),
    opId: hexToBytes(vector.op_id),
    ep: vector.ep,
    path: vector.path,
  };
  const plaintext = hexToBytes(vector.plaintext_hex);

  const envelope = seal(key, nonce, ctx, plaintext);
  const hex = bytesToHex(envelope);
  if (hex !== vector.envelope_hex) {
    throw new Error(`envelope bytes mismatch:\n  expected ${vector.envelope_hex}\n  got      ${hex}`);
  }

  const opened = open(key, envelope, ctx);
  if (opened.error || bytesToHex(opened.plaintext) !== vector.plaintext_hex) {
    throw new Error(`open failed: ${JSON.stringify(opened)}`);
  }

  const mustFail = (label, k, env, c, want) => {
    const result = open(k, env, c);
    if (!result.error) throw new Error(`${label}: expected failure, got plaintext`);
    if (want && result.error !== want) throw new Error(`${label}: expected ${want}, got ${result.error}`);
  };

  // AAD binding negatives (I-10).
  const flip = (bytes, i) => {
    const copy = Uint8Array.from(bytes);
    copy[i] ^= 1;
    return copy;
  };
  mustFail('ds flip', key, envelope, { ...ctx, ds: flip(ctx.ds, 0) }, 'DecryptFailed');
  mustFail('op_id flip', key, envelope, { ...ctx, opId: flip(ctx.opId, 0) }, 'DecryptFailed');
  mustFail('ep flip', key, envelope, { ...ctx, ep: ctx.ep + 1 }, 'DecryptFailed');
  mustFail('path flip', key, envelope, { ...ctx, path: ctx.path + 'x' }, 'DecryptFailed');

  // Header negatives.
  const badVersion = Uint8Array.from(envelope);
  badVersion[0] = 2;
  mustFail('version', key, badVersion, ctx, 'UnknownVersion');
  mustFail('key id', key, flip(envelope, 1), ctx, 'UnknownKeyId');
  mustFail('truncated', key, envelope.subarray(0, 40), ctx, 'Truncated');
  mustFail('tag flip', key, flip(envelope, envelope.length - 1), ctx, 'DecryptFailed');
}
