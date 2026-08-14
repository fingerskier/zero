// RELAY 0.2.2-draft wire-transcript model (M3a). Independent of the Rust core.
// Handshake, dual-root catch-up invariant, resume cursor, reject-ack.
// Vectors carry ordered {type, request_id, payload} frames (RELAY §3.3 / §4).

import { createPrivateKey, createPublicKey, sign as edSign, verify as edVerify } from 'node:crypto';
import { Buffer } from 'node:buffer';

import { hexToBytes, bytesToHex } from './cbor.mjs';
import { blake3 } from './blake3.mjs';
import { merkleRootOnce } from './merkle.mjs';

export const DOMAIN_RELAY_AUTH = new TextEncoder().encode('zerodb-relay-auth-v1');
export const RELAY_CAPS = ['dual-root', 'reject-ack', 'resume-cursor'];
export const ERR_AUTH_FAILED = 0x201;

export const MSG_HELLO = 0x01;
export const MSG_CHALLENGE = 0x02;
export const MSG_AUTH = 0x03;
export const MSG_WELCOME = 0x04;
export const MSG_SYNC_REQUEST = 0x20;
export const MSG_SYNC_RESPONSE = 0x21;
export const MSG_OPS = 0x30;
export const MSG_OP_ACK = 0x31;
export const MSG_ERROR = 0xff;

export const DIR_PEER_TO_RELAY = 'P→R';
export const DIR_RELAY_TO_PEER = 'R→P';

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

/** RELAY §4.1 / §5.2: signature over domain||nonce AND claimed PeerId == BLAKE3(pk). */
export function authenticate(claimedPeerId, pk, nonce, sig) {
  const claimed = claimedPeerId instanceof Uint8Array ? claimedPeerId : hex32(claimedPeerId);
  const ok = verifyAuth(pk, nonce, sig) && bytesToHex(claimed) === bytesToHex(peerIdFromPk(pk));
  return ok ? null : ERR_AUTH_FAILED;
}

export function knownMessageType(ty) {
  return (
    ty === MSG_HELLO ||
    ty === MSG_CHALLENGE ||
    ty === MSG_AUTH ||
    ty === MSG_WELCOME ||
    ty === MSG_ERROR ||
    ty === MSG_SYNC_REQUEST ||
    ty === MSG_SYNC_RESPONSE ||
    ty === MSG_OPS ||
    ty === MSG_OP_ACK
  );
}

export function fixedDirection(ty) {
  if (ty === MSG_HELLO || ty === MSG_AUTH) return DIR_PEER_TO_RELAY;
  if (ty === MSG_CHALLENGE || ty === MSG_WELCOME || ty === MSG_OP_ACK) return DIR_RELAY_TO_PEER;
  return null;
}

export function requiredPayloadKeys(ty) {
  switch (ty) {
    case MSG_HELLO:
      return ['peer_id', 'public_key', 'protocol_version', 'capabilities'];
    case MSG_CHALLENGE:
      return ['nonce'];
    case MSG_AUTH:
      return ['signature'];
    case MSG_WELCOME:
      return ['protocol_version', 'relay_level', 'capabilities', 'limits'];
    case MSG_ERROR:
      return ['code', 'message', 'fatal'];
    case MSG_SYNC_REQUEST:
    case MSG_SYNC_RESPONSE:
      return ['datastore'];
    case MSG_OPS:
      return ['datastore', 'operations'];
    case MSG_OP_ACK:
      return ['outcomes'];
    default:
      return [];
  }
}

export function requiredSyncRoot(dir) {
  if (dir === DIR_PEER_TO_RELAY) return 'accepted_root';
  if (dir === DIR_RELAY_TO_PEER) return 'validated_root';
  return null;
}

export function isRequest(ty, dir, requestId) {
  return ty === MSG_HELLO || ty === MSG_AUTH || ty === MSG_SYNC_REQUEST ||
    (ty === MSG_OPS && dir === DIR_PEER_TO_RELAY && requestId !== 0);
}

export function isResponse(ty, requestId) {
  return ty === MSG_CHALLENGE || ty === MSG_WELCOME || ty === MSG_SYNC_RESPONSE || ty === MSG_OP_ACK ||
    (ty === MSG_ERROR && requestId !== 0);
}

export function expectedResponseTypes(requestTy) {
  switch (requestTy) {
    case MSG_HELLO:
      return [MSG_CHALLENGE, MSG_ERROR];
    case MSG_AUTH:
      return [MSG_WELCOME, MSG_ERROR];
    case MSG_SYNC_REQUEST:
      return [MSG_SYNC_RESPONSE];
    case MSG_OPS:
      return [MSG_OP_ACK];
    default:
      return [];
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

function hasField(payload, key) {
  return payload != null && Object.prototype.hasOwnProperty.call(payload, key) && payload[key] != null;
}

function checkHandshakeFrames(v, frames) {
  if (frames.length < 4) throw new Error('handshake frames must be HELLO/CHALLENGE/AUTH/final');
  if (frames[0].type !== MSG_HELLO) throw new Error('frames[0] must be HELLO');
  if (frames[1].type !== MSG_CHALLENGE) throw new Error('frames[1] must be CHALLENGE');
  if (frames[2].type !== MSG_AUTH) throw new Error('frames[2] must be AUTH');
  const last = v.expect.auth_ok ? MSG_WELCOME : MSG_ERROR;
  if (frames[3].type !== last) {
    throw new Error(`frames[3] type ${frames[3].type}, expected ${last}`);
  }
  if (frames[0].payload.peer_id !== v.peer_id) {
    throw new Error('HELLO.peer_id must equal the claimed transcript peer_id');
  }
  if (frames[0].payload.public_key !== v.public_key) {
    throw new Error('HELLO.public_key must match the transcript public_key');
  }
  if (frames[1].payload.nonce !== v.nonce) {
    throw new Error('CHALLENGE.nonce must match the transcript nonce');
  }
  if (!v.expect.auth_ok) {
    if (frames[3].payload.code !== ERR_AUTH_FAILED) {
      throw new Error(`ERROR.code ${frames[3].payload.code}, expected ${ERR_AUTH_FAILED}`);
    }
    if (frames[3].payload.fatal !== true) throw new Error('AUTH_FAILED ERROR must be fatal');
  }
}

function checkDualRootFrames(frames) {
  let peerResp = false;
  let relayResp = false;
  for (const f of frames) {
    if (f.type !== MSG_SYNC_RESPONSE) continue;
    if (f.dir === DIR_PEER_TO_RELAY) peerResp = true;
    if (f.dir === DIR_RELAY_TO_PEER) relayResp = true;
  }
  if (!peerResp) throw new Error('dual-root frames must include a peer SYNC_RESPONSE (accepted_root)');
  if (!relayResp) throw new Error('dual-root frames must include a relay SYNC_RESPONSE (validated_root)');
}

function checkResumeFrames(v, frames) {
  let sawCursor = false;
  const ops = [];
  for (const f of frames) {
    if (f.type === MSG_SYNC_REQUEST) {
      if (!hasField(f.payload, 'cursor')) throw new Error('resume SYNC_REQUEST must carry cursor');
      sawCursor = true;
    }
    if (f.type === MSG_OPS && f.dir === DIR_PEER_TO_RELAY) {
      for (const op of f.payload.operations) ops.push(op.op_id);
    }
  }
  if (!sawCursor) throw new Error('resume frames must include SYNC_REQUEST.cursor');
  const got = [...ops].sort();
  const want = [...v.expect.retransmit].sort();
  if (JSON.stringify(got) !== JSON.stringify(want)) {
    throw new Error(`OPS must carry the retransmit set ${JSON.stringify(want)}, got ${JSON.stringify(got)}`);
  }
}

function checkRejectFrames(v, frames) {
  let sawOps = false;
  const rejected = [];
  for (const f of frames) {
    if (f.type === MSG_OPS) sawOps = true;
    if (f.type === MSG_OP_ACK) {
      for (const o of f.payload.outcomes) {
        if (o.outcome === 'REJECT') rejected.push(o.op_id);
      }
    }
  }
  if (!sawOps) throw new Error('reject-ack frames must include OPS');
  const want = v.outcomes.filter((o) => o.outcome === 'REJECT').map((o) => o.op_id);
  if (JSON.stringify([...rejected].sort()) !== JSON.stringify([...want].sort())) {
    throw new Error(`OP_ACK REJECT set ${JSON.stringify(rejected)}, expected ${JSON.stringify(want)}`);
  }
}

export function checkFrames(v) {
  const frames = v.frames;
  if (!Array.isArray(frames) || frames.length === 0) {
    throw new Error('frames must be a non-empty array of {type, request_id, payload}');
  }

  const pending = new Map();

  for (let i = 0; i < frames.length; i++) {
    const f = frames[i];
    const label = `frames[${i}]`;
    if (typeof f.type !== 'number') throw new Error(`${label}: type must be a number`);
    if (typeof f.request_id !== 'number') throw new Error(`${label}: request_id must be a number`);
    if (f.dir !== DIR_PEER_TO_RELAY && f.dir !== DIR_RELAY_TO_PEER) {
      throw new Error(`${label}: dir must be ${DIR_PEER_TO_RELAY} or ${DIR_RELAY_TO_PEER}`);
    }
    if (!f.payload || typeof f.payload !== 'object' || Array.isArray(f.payload)) {
      throw new Error(`${label}: payload required`);
    }
    if (!knownMessageType(f.type)) throw new Error(`${label}: unknown type 0x${f.type.toString(16)}`);

    const wantDir = fixedDirection(f.type);
    if (wantDir && f.dir !== wantDir) {
      throw new Error(`${label}: type 0x${f.type.toString(16)} dir ${f.dir}, expected ${wantDir}`);
    }
    for (const key of requiredPayloadKeys(f.type)) {
      if (!hasField(f.payload, key)) throw new Error(`${label}: missing ${key}`);
    }
    if (f.type === MSG_SYNC_REQUEST || f.type === MSG_SYNC_RESPONSE) {
      const root = requiredSyncRoot(f.dir);
      if (!hasField(f.payload, root)) {
        const who = f.dir === DIR_PEER_TO_RELAY ? 'peer' : 'relay';
        throw new Error(`${label}: ${who} SYNC must carry ${root}`);
      }
    }

    if (isRequest(f.type, f.dir, f.request_id)) {
      if (f.request_id === 0) throw new Error(`${label}: request must have non-zero request_id`);
      pending.set(f.request_id, expectedResponseTypes(f.type));
    }
    if (isResponse(f.type, f.request_id)) {
      if (f.request_id === 0) throw new Error(`${label}: response must echo a request_id`);
      const want = pending.get(f.request_id);
      if (!want) throw new Error(`${label}: no open request for request_id ${f.request_id}`);
      if (!want.includes(f.type)) {
        throw new Error(`${label}: type 0x${f.type.toString(16)} does not correlate with request_id ${f.request_id}`);
      }
      pending.delete(f.request_id);
    }
  }

  if (pending.size > 0) {
    throw new Error(`unmatched request_id(s): ${[...pending.keys()].join(',')}`);
  }

  switch (v.kind) {
    case 'handshake':
      checkHandshakeFrames(v, frames);
      break;
    case 'dual-root':
      checkDualRootFrames(frames);
      break;
    case 'resume':
      checkResumeFrames(v, frames);
      break;
    case 'reject-ack':
      checkRejectFrames(v, frames);
      break;
    default:
      break;
  }
}

function runHandshake(v) {
  const pk = hex32(v.public_key);
  const seed = hex32(v.secret_key);
  const nonce = hex32(v.nonce);
  if (!v.peer_id) throw new Error('claimed HELLO.peer_id required');
  const pid = bytesToHex(peerIdFromPk(pk));
  const honest = signAuth(seed, nonce);
  const sig = v.auth_signature ? hex64(v.auth_signature) : honest;
  const err = authenticate(v.peer_id, pk, nonce, sig);
  const authOk = err === null;
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
  for (const [i, f] of (v.frames || []).entries()) {
    if (f.type !== MSG_SYNC_REQUEST && f.type !== MSG_SYNC_RESPONSE) continue;
    if (f.dir === DIR_RELAY_TO_PEER && f.payload.validated_root && f.payload.validated_root !== validated) {
      throw new Error(`frames[${i}]: validated_root ${f.payload.validated_root}, computed ${validated}`);
    }
    if (f.dir === DIR_PEER_TO_RELAY && f.payload.accepted_root && f.payload.accepted_root !== a) {
      throw new Error(`frames[${i}]: accepted_root ${f.payload.accepted_root}, computed ${a}`);
    }
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
      runHandshake(vector);
      break;
    case 'dual-root':
      runDualRoot(vector);
      break;
    case 'resume':
      runResume(vector);
      break;
    case 'reject-ack':
      runRejectAck(vector);
      break;
    default:
      throw new Error(`unknown relay-transcript kind "${vector.kind}"`);
  }
  checkFrames(vector);
}
