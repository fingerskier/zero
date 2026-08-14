// RELAY 0.2.2-draft wire-transcript model (M3a). Independent of the Rust core.
// Handshake, dual-root catch-up invariant, resume cursor, reject-ack.

import { createPrivateKey, createPublicKey, sign as edSign, verify as edVerify } from 'node:crypto';
import { Buffer } from 'node:buffer';

import { hexToBytes, bytesToHex } from './cbor.mjs';
import { blake3 } from './blake3.mjs';
import { merkleRootOnce } from './merkle.mjs';

export const DOMAIN_RELAY_AUTH = new TextEncoder().encode('zerodb-relay-auth-v1');
export const RELAY_CAPS = ['dual-root', 'reject-ack', 'resume-cursor'];
export const ERR_AUTH_FAILED = 0x201;

const SPKI_PREFIX = hexToBytes('302a300506032b6570032100');
const PKCS8_PREFIX = hexToBytes('302e020100300506032b657004220420');

function hex32(s) {
  const b = hexToBytes(s);
  if (b.length !== 32) throw new Error(`expected 32-byte hex, got ${b.length}`);
  return b;
}

function hex64(s) {
  const b = hexToBytes(s);
  if (b.length !== 64) throw new Error(`expected 64-byte hex, got ${b.length}`);
  return b;
}

export function peerIdFromPk(pk) {
  return blake3(pk);
}

export function negotiateCapabilities(hello, relay) {
  return RELAY_CAPS.filter((c) => hello.includes(c) && relay.includes(c));
}

function ed25519Private(seed) {
  return createPrivateKey({ key: Buffer.concat([Buffer.from(PKCS8_PREFIX), Buffer.from(seed)]), format: 'der', type: 'pkcs8' });
}

function ed25519Public(pk) {
  return createPublicKey({ key: Buffer.concat([Buffer.from(SPKI_PREFIX), Buffer.from(pk)]), format: 'der', type: 'spki' });
}

export function authPreimage(nonce) {
  const out = new Uint8Array(DOMAIN_RELAY_AUTH.length + nonce.length);
  out.set(DOMAIN_RELAY_AUTH, 0);
  out.set(nonce, DOMAIN_RELAY_AUTH.length);
  return out;
}

export function signAuth(seed, nonce) {
  return new Uint8Array(edSign(null, Buffer.from(authPreimage(nonce)), ed25519Private(seed)));
}

export function verifyAuth(pk, nonce, sig) {
  try {
    return edVerify(null, Buffer.from(authPreimage(nonce)), ed25519Public(pk), Buffer.from(sig));
  } catch {
    return false;
  }
}

function merkleOps(list) {
  return list.map((o) => ({
    op_id: hexToBytes(o.op_id),
    physical_ms: o.physical_ms,
    logical: o.logical ?? 0,
    author: hexToBytes(o.author),
  }));
}

function cmpOp(a, b) {
  if (a.physical_ms !== b.physical_ms) return a.physical_ms < b.physical_ms ? -1 : 1;
  if (a.logical !== b.logical) return a.logical < b.logical ? -1 : 1;
  if (a.author !== b.author) return a.author < b.author ? -1 : 1;
  if (a.op_id !== b.op_id) return a.op_id < b.op_id ? -1 : 1;
  return 0;
}

function covered(frontier, op) {
  const tip = frontier[op.author];
  if (!tip) return false;
  return cmpOp(op, { ...tip, author: op.author }) <= 0;
}

function retransmit(held, cursor, rejected) {
  const frontier = cursor.frontier || {};
  const skip = new Set(rejected || []);
  return held
    .filter((op) => !skip.has(op.op_id) && !covered(frontier, op))
    .map((op) => op.op_id)
    .sort();
}

function runHandshake(v) {
  const pk = hex32(v.public_key);
  const seed = hex32(v.secret_key);
  const nonce = hex32(v.nonce);
  const pid = bytesToHex(peerIdFromPk(pk));
  const honest = signAuth(seed, nonce);
  const sig = v.auth_signature ? hex64(v.auth_signature) : honest;
  const authOk = verifyAuth(pk, nonce, sig);
  const expect = v.expect;
  if (authOk !== expect.auth_ok) {
    throw new Error(`auth_ok ${authOk}, expected ${expect.auth_ok}`);
  }
  if (!authOk) {
    if (expect.error_code !== ERR_AUTH_FAILED) {
      throw new Error(`error_code ${expect.error_code}, expected ${ERR_AUTH_FAILED}`);
    }
    return;
  }
  if (pid !== expect.peer_id) throw new Error(`peer_id ${pid}, expected ${expect.peer_id}`);
  const caps = negotiateCapabilities(v.hello_capabilities, v.relay_capabilities);
  const want = expect.welcome_capabilities;
  if (JSON.stringify(caps) !== JSON.stringify(want)) {
    throw new Error(`welcome_capabilities ${JSON.stringify(caps)}, expected ${JSON.stringify(want)}`);
  }
  if (expect.signature && bytesToHex(honest) !== expect.signature) {
    throw new Error(`signature ${bytesToHex(honest)}, expected ${expect.signature}`);
  }
}

function runDualRoot(v) {
  const validated = bytesToHex(merkleRootOnce(merkleOps(v.validated)));
  const a = bytesToHex(merkleRootOnce(merkleOps(v.accepted_a)));
  const b = bytesToHex(merkleRootOnce(merkleOps(v.accepted_b)));
  const equal = validated === a;
  if (equal !== v.expect.roots_equal) {
    throw new Error(`roots_equal ${equal} (validated=${validated} accepted=${a})`);
  }
  const peers = a === b;
  if (peers !== v.expect.peer_accepted_equal) {
    throw new Error(`peer_accepted_equal ${peers} (a=${a} b=${b})`);
  }
}

function runResume(v) {
  const got = retransmit(v.held, v.cursor, v.rejected || []);
  const want = [...v.expect.retransmit].sort();
  if (JSON.stringify(got) !== JSON.stringify(want)) {
    throw new Error(`retransmit ${JSON.stringify(got)}, expected ${JSON.stringify(want)}`);
  }
}

function runRejectAck(v) {
  const rejected = v.outcomes.filter((o) => o.outcome === 'REJECT').map((o) => o.op_id);
  const got = retransmit(v.held, v.cursor, rejected);
  const want = [...v.expect.retransmit].sort();
  if (JSON.stringify(got) !== JSON.stringify(want)) {
    throw new Error(`retransmit after reject ${JSON.stringify(got)}, expected ${JSON.stringify(want)}`);
  }
}

export function runRelayTranscriptVector(vector) {
  switch (vector.kind) {
    case 'handshake':
      return runHandshake(vector);
    case 'dual-root':
      return runDualRoot(vector);
    case 'resume':
      return runResume(vector);
    case 'reject-ack':
      return runRejectAck(vector);
    default:
      throw new Error(`unknown relay-transcript kind "${vector.kind}"`);
  }
}
