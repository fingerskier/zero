// Pure-JS BLAKE3 (hash mode only — no keyed/derive-key), implemented from
// the BLAKE3 specification for the conformance runner's OpId cross-check.
// Correctness is anchored by the Rust-emitted golden vectors: any
// divergence from the reference implementation fails the required lane.

const IV = [
  0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
  0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];
const MSG_PERM = [2, 6, 3, 10, 7, 0, 4, 13, 1, 11, 12, 5, 9, 14, 15, 8];

const CHUNK_START = 1;
const CHUNK_END = 2;
const PARENT = 4;
const ROOT = 8;

const CHUNK_LEN = 1024;
const BLOCK_LEN = 64;

const rotr = (x, n) => ((x >>> n) | (x << (32 - n))) >>> 0;

function g(st, a, b, c, d, mx, my) {
  st[a] = (st[a] + st[b] + mx) >>> 0;
  st[d] = rotr(st[d] ^ st[a], 16);
  st[c] = (st[c] + st[d]) >>> 0;
  st[b] = rotr(st[b] ^ st[c], 12);
  st[a] = (st[a] + st[b] + my) >>> 0;
  st[d] = rotr(st[d] ^ st[a], 8);
  st[c] = (st[c] + st[d]) >>> 0;
  st[b] = rotr(st[b] ^ st[c], 7);
}

function compress(cv, block, counter, blockLen, flags) {
  const st = [
    cv[0], cv[1], cv[2], cv[3], cv[4], cv[5], cv[6], cv[7],
    IV[0], IV[1], IV[2], IV[3],
    Number(counter & 0xffffffffn) >>> 0,
    Number((counter >> 32n) & 0xffffffffn) >>> 0,
    blockLen,
    flags,
  ];
  let m = block;
  for (let round = 0; round < 7; round += 1) {
    g(st, 0, 4, 8, 12, m[0], m[1]);
    g(st, 1, 5, 9, 13, m[2], m[3]);
    g(st, 2, 6, 10, 14, m[4], m[5]);
    g(st, 3, 7, 11, 15, m[6], m[7]);
    g(st, 0, 5, 10, 15, m[8], m[9]);
    g(st, 1, 6, 11, 12, m[10], m[11]);
    g(st, 2, 7, 8, 13, m[12], m[13]);
    g(st, 3, 4, 9, 14, m[14], m[15]);
    if (round < 6) m = MSG_PERM.map((i) => m[i]);
  }
  const out = new Array(8);
  for (let i = 0; i < 8; i += 1) out[i] = (st[i] ^ st[i + 8]) >>> 0;
  return out;
}

// 16 little-endian u32 words from up to 64 bytes (zero-padded).
function blockWords(bytes) {
  const w = new Array(16).fill(0);
  for (let i = 0; i < bytes.length; i += 1) {
    w[i >> 2] = (w[i >> 2] | (bytes[i] << ((i & 3) * 8))) >>> 0;
  }
  return w;
}

// Fold a chunk's blocks into a pre-finalization "output": the last block's
// compress inputs, so ROOT can still be OR-ed in at the top of the tree.
function chunkOutput(chunk, chunkIndex) {
  const nBlocks = Math.max(1, Math.ceil(chunk.length / BLOCK_LEN));
  let cv = IV;
  for (let b = 0; b < nBlocks - 1; b += 1) {
    const block = chunk.subarray(b * BLOCK_LEN, (b + 1) * BLOCK_LEN);
    const flags = b === 0 ? CHUNK_START : 0;
    cv = compress(cv, blockWords(block), chunkIndex, BLOCK_LEN, flags);
  }
  const last = chunk.subarray((nBlocks - 1) * BLOCK_LEN, chunk.length);
  let flags = CHUNK_END;
  if (nBlocks === 1) flags |= CHUNK_START;
  return { cv, block: blockWords(last), counter: chunkIndex, blockLen: last.length, flags };
}

function parentOutput(leftCV, rightCV) {
  return {
    cv: IV,
    block: [...leftCV, ...rightCV],
    counter: 0n,
    blockLen: BLOCK_LEN,
    flags: PARENT,
  };
}

const finalizeCV = (o) => compress(o.cv, o.block, o.counter, o.blockLen, o.flags);

// Non-root CV of a subtree of chunks starting at chunkIndex.
function subtreeCV(chunks, startIndex) {
  if (chunks.length === 1) return finalizeCV(chunkOutput(chunks[0], BigInt(startIndex)));
  let split = 1;
  while (split * 2 < chunks.length) split *= 2; // largest power of two < length
  const left = subtreeCV(chunks.slice(0, split), startIndex);
  const right = subtreeCV(chunks.slice(split), startIndex + split);
  return finalizeCV(parentOutput(left, right));
}

/** 32-byte BLAKE3 hash of `input` (Uint8Array). */
export function blake3(input) {
  const chunks = [];
  if (input.length === 0) {
    chunks.push(input.subarray(0, 0));
  } else {
    for (let i = 0; i < input.length; i += CHUNK_LEN) {
      chunks.push(input.subarray(i, Math.min(i + CHUNK_LEN, input.length)));
    }
  }

  let output;
  if (chunks.length === 1) {
    output = chunkOutput(chunks[0], 0n);
  } else {
    let split = 1;
    while (split * 2 < chunks.length) split *= 2;
    const left = subtreeCV(chunks.slice(0, split), 0);
    const right = subtreeCV(chunks.slice(split), split);
    output = parentOutput(left, right);
  }

  const words = compress(output.cv, output.block, output.counter, output.blockLen, output.flags | ROOT);
  const out = new Uint8Array(32);
  for (let i = 0; i < 8; i += 1) {
    out[i * 4] = words[i] & 0xff;
    out[i * 4 + 1] = (words[i] >>> 8) & 0xff;
    out[i * 4 + 2] = (words[i] >>> 16) & 0xff;
    out[i * 4 + 3] = (words[i] >>> 24) & 0xff;
  }
  return out;
}
