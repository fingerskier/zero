// Encrypted-value envelope handler per doc/KERNEL.md §7 (vector type
// "envelope"). Seals with the vector's fixed key/nonce and requires byte
// agreement with the Rust-emitted golden; opens it back; then runs the
// negative matrix: every AAD component flipped, truncation, unknown
// version, and a wrong key id must fail (I-10).

import { bytesToHex, hexToBytes, encode } from './cbor.mjs';
import { blake3 } from './blake3.mjs';
import { xchachaSeal, xchachaOpen } from './xchacha.mjs';

const DOMAIN_AAD = new TextEncoder().encode('zerodb-value-aad-v1');
const DOMAIN_SLOT = new TextEncoder().encode('zerodb-value-slot-v1');
const VERSION = 1;
const KEY_ID_LEN = 16;
const NONCE_LEN = 24;

function be64(n) {
  const b = new Uint8Array(8);
  new DataView(b.buffer).setBigUint64(0, BigInt(n), false);
  return b;
}

function be16(n) {
  const b = new Uint8Array(2);
  new DataView(b.buffer).setUint16(0, n, false);
  return b;
}

function slotId(ctx) {
  const path = new TextEncoder().encode(ctx.path);
  const pre = new Uint8Array(
    DOMAIN_SLOT.length + 32 + 8 + path.length + 32 + 8 + 2,
  );
  let o = 0;
  pre.set(DOMAIN_SLOT, o);
  o += DOMAIN_SLOT.length;
  pre.set(ctx.ds, o);
  o += 32;
  pre.set(be64(ctx.ep), o);
  o += 8;
  pre.set(path, o);
  o += path.length;
  pre.set(ctx.author, o);
  o += 32;
  pre.set(be64(ctx.physical_ms), o);
  o += 8;
  pre.set(be16(ctx.logical), o);
  return blake3(pre);
}

function aadFor(ctx) {
  const slot = slotId(ctx);
  const aad = new Uint8Array(DOMAIN_AAD.length + 32);
  aad.set(DOMAIN_AAD, 0);
  aad.set(slot, DOMAIN_AAD.length);
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
    author: hexToBytes(vector.author),
    physical_ms: vector.physical_ms,
    logical: vector.logical ?? 0,
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
  mustFail('author flip', key, envelope, { ...ctx, author: flip(ctx.author, 0) }, 'DecryptFailed');
  mustFail('physical_ms flip', key, envelope, { ...ctx, physical_ms: ctx.physical_ms + 1 }, 'DecryptFailed');
  mustFail('logical flip', key, envelope, { ...ctx, logical: ctx.logical + 1 }, 'DecryptFailed');
  mustFail('ep flip', key, envelope, { ...ctx, ep: ctx.ep + 1 }, 'DecryptFailed');
  mustFail('path flip', key, envelope, { ...ctx, path: ctx.path + 'x' }, 'DecryptFailed');

  // Header negatives.
  const badVersion = Uint8Array.from(envelope);
  badVersion[0] = 2;
  mustFail('version', key, badVersion, ctx, 'UnknownVersion');
  mustFail('key id', key, flip(envelope, 1), ctx, 'UnknownKeyId');
  mustFail('truncated', key, envelope.subarray(0, 40), ctx, 'Truncated');
  mustFail('tag flip', key, flip(envelope, envelope.length - 1), ctx, 'DecryptFailed');

  if (vector.expect_op_id_hex) {
    const DOMAIN_OP_ID = new TextEncoder().encode('zerodb-op-id-v1');
    const tagged = {
      t: 'map',
      v: {
        v: { t: 'uint', v: 1 },
        ds: { t: 'bytes', hex: vector.ds },
        ep: { t: 'uint', v: vector.ep },
        author: { t: 'bytes', hex: vector.author },
        ts: {
          t: 'map',
          v: {
            l: { t: 'uint', v: vector.logical ?? 0 },
            p: { t: 'uint', v: vector.physical_ms },
          },
        },
        deps: { t: 'array', v: [] },
        grp: { t: 'null' },
        kind: { t: 'uint', v: 3 },
        body: {
          t: 'map',
          v: {
            crdt: { t: 'text', v: 'lww' },
            encrypted: { t: 'bytes', hex: bytesToHex(envelope) },
            path: { t: 'text', v: vector.path },
          },
        },
      },
    };
    const canonical = encode(tagged);
    const pre = new Uint8Array(DOMAIN_OP_ID.length + canonical.length);
    pre.set(DOMAIN_OP_ID, 0);
    pre.set(canonical, DOMAIN_OP_ID.length);
    const got = bytesToHex(blake3(pre));
    if (got !== vector.expect_op_id_hex) {
      throw new Error(`op_id: expected ${vector.expect_op_id_hex}, got ${got}`);
    }
  }
}
