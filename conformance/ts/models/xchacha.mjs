// XChaCha20-Poly1305 for the conformance runner, built from HChaCha20
// (pure JS, RFC draft-irtf-cfrg-xchacha) + node's native chacha20-poly1305:
// subkey = HChaCha20(key, nonce[0..16]); 12-byte nonce = 4 zero bytes ‖
// nonce[16..24]. Anchored by the Rust chacha20poly1305 golden vectors.

import { createCipheriv, createDecipheriv } from 'node:crypto';
import { Buffer } from 'node:buffer';

const rotl = (x, n) => ((x << n) | (x >>> (32 - n))) >>> 0;

function quarterRound(st, a, b, c, d) {
  st[a] = (st[a] + st[b]) >>> 0;
  st[d] = rotl(st[d] ^ st[a], 16);
  st[c] = (st[c] + st[d]) >>> 0;
  st[b] = rotl(st[b] ^ st[c], 12);
  st[a] = (st[a] + st[b]) >>> 0;
  st[d] = rotl(st[d] ^ st[a], 8);
  st[c] = (st[c] + st[d]) >>> 0;
  st[b] = rotl(st[b] ^ st[c], 7);
}

function leWords(bytes, count) {
  const words = new Array(count);
  for (let i = 0; i < count; i += 1) {
    words[i] =
      (bytes[i * 4] | (bytes[i * 4 + 1] << 8) | (bytes[i * 4 + 2] << 16) | (bytes[i * 4 + 3] << 24)) >>> 0;
  }
  return words;
}

/** HChaCha20: 32-byte key + 16-byte nonce → 32-byte subkey (no feed-forward). */
export function hchacha20(key, nonce16) {
  const st = [
    0x61707865, 0x3320646e, 0x79622d32, 0x6b206574,
    ...leWords(key, 8),
    ...leWords(nonce16, 4),
  ];
  for (let i = 0; i < 10; i += 1) {
    quarterRound(st, 0, 4, 8, 12);
    quarterRound(st, 1, 5, 9, 13);
    quarterRound(st, 2, 6, 10, 14);
    quarterRound(st, 3, 7, 11, 15);
    quarterRound(st, 0, 5, 10, 15);
    quarterRound(st, 1, 6, 11, 12);
    quarterRound(st, 2, 7, 8, 13);
    quarterRound(st, 3, 4, 9, 14);
  }
  const outWords = [st[0], st[1], st[2], st[3], st[12], st[13], st[14], st[15]];
  const out = new Uint8Array(32);
  outWords.forEach((w, i) => {
    out[i * 4] = w & 0xff;
    out[i * 4 + 1] = (w >>> 8) & 0xff;
    out[i * 4 + 2] = (w >>> 16) & 0xff;
    out[i * 4 + 3] = (w >>> 24) & 0xff;
  });
  return out;
}

function derive(key, nonce24) {
  const subkey = hchacha20(key, nonce24.subarray(0, 16));
  const nonce12 = new Uint8Array(12);
  nonce12.set(nonce24.subarray(16, 24), 4);
  return { subkey, nonce12 };
}

/** Encrypt; returns ciphertext ‖ 16-byte tag. */
export function xchachaSeal(key, nonce24, aad, plaintext) {
  const { subkey, nonce12 } = derive(key, nonce24);
  const cipher = createCipheriv('chacha20-poly1305', subkey, nonce12, { authTagLength: 16 });
  cipher.setAAD(Buffer.from(aad), { plaintextLength: plaintext.length });
  const ct = Buffer.concat([cipher.update(Buffer.from(plaintext)), cipher.final()]);
  return Buffer.concat([ct, cipher.getAuthTag()]);
}

/** Decrypt ciphertext ‖ tag; returns plaintext or null on auth failure. */
export function xchachaOpen(key, nonce24, aad, ctAndTag) {
  if (ctAndTag.length < 16) return null;
  const { subkey, nonce12 } = derive(key, nonce24);
  const decipher = createDecipheriv('chacha20-poly1305', subkey, nonce12, { authTagLength: 16 });
  decipher.setAAD(Buffer.from(aad), { plaintextLength: ctAndTag.length - 16 });
  decipher.setAuthTag(Buffer.from(ctAndTag.subarray(ctAndTag.length - 16)));
  try {
    return Buffer.concat([
      decipher.update(Buffer.from(ctAndTag.subarray(0, ctAndTag.length - 16))),
      decipher.final(),
    ]);
  } catch {
    return null;
  }
}
