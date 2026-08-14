// Frontiers + snapshot identity per doc/FRONTIER.md (M0f).

import { blake3 } from './blake3.mjs';
import { bytesToHex, hexToBytes } from './cbor.mjs';
import { merkleRootOnce } from './merkle.mjs';

const DOMAIN_SNAPSHOT = new TextEncoder().encode('zerodb-snapshot-v1');
const SNAPSHOT_FORMAT_VERSION = 1;

function parseOp(o) {
  return {
    op_id: hexToBytes(o.op_id),
    physical_ms: o.physical_ms,
    logical: o.logical ?? 0,
    author: hexToBytes(o.author),
  };
}

function orderKey(op) {
  return [op.physical_ms, op.logical, bytesToHex(op.author), bytesToHex(op.op_id)];
}

function cmpKey(a, b) {
  const ka = orderKey(a);
  const kb = orderKey(b);
  for (let i = 0; i < ka.length; i++) {
    if (ka[i] < kb[i]) return -1;
    if (ka[i] > kb[i]) return 1;
  }
  return 0;
}

export function buildFrontier(ops) {
  const best = new Map();
  for (const op of ops) {
    const a = bytesToHex(op.author);
    if (!best.has(a) || cmpKey(op, best.get(a)) > 0) best.set(a, op);
  }
  const out = {};
  for (const [a, op] of [...best.entries()].sort((x, y) => (x[0] < y[0] ? -1 : 1))) {
    out[a] = {
      op_id: bytesToHex(op.op_id),
      physical_ms: op.physical_ms,
      logical: op.logical,
    };
  }
  return out;
}

export function tailBoundary(ops) {
  if (ops.length === 0) return null;
  let best = ops[0];
  for (const op of ops) if (cmpKey(op, best) > 0) best = op;
  return bytesToHex(best.op_id);
}

function frontierBytes(frontier) {
  const authors = Object.keys(frontier).sort();
  const parts = [];
  for (const a of authors) {
    const tip = frontier[a];
    parts.push(hexToBytes(a));
    parts.push(hexToBytes(typeof tip === 'string' ? tip : tip.op_id));
    const p = typeof tip === 'string' ? 0 : tip.physical_ms;
    const l = typeof tip === 'string' ? 0 : tip.logical;
    const pb = new Uint8Array(8);
    new DataView(pb.buffer).setBigUint64(0, BigInt(p), false);
    const lb = new Uint8Array(2);
    new DataView(lb.buffer).setUint16(0, l, false);
    parts.push(pb);
    parts.push(lb);
  }
  let n = 0;
  for (const p of parts) n += p.length;
  const out = new Uint8Array(n);
  let o = 0;
  for (const p of parts) {
    out.set(p, o);
    o += p.length;
  }
  return out;
}

export function snapshotId(datastoreIdHex, frontier, merkleHex, tailHex) {
  const parts = [
    DOMAIN_SNAPSHOT,
    new Uint8Array([SNAPSHOT_FORMAT_VERSION]),
    hexToBytes(datastoreIdHex),
    frontierBytes(frontier),
    hexToBytes(merkleHex),
  ];
  if (tailHex) {
    parts.push(new Uint8Array([1]));
    parts.push(hexToBytes(tailHex));
  } else {
    parts.push(new Uint8Array([0]));
  }
  let n = 0;
  for (const p of parts) n += p.length;
  const pre = new Uint8Array(n);
  let o = 0;
  for (const p of parts) {
    pre.set(p, o);
    o += p.length;
  }
  return bytesToHex(blake3(pre));
}

export function isLateOp(op, frontier) {
  const tip = frontier[bytesToHex(op.author)];
  if (!tip) return false;
  const tipOp = {
    op_id: hexToBytes(typeof tip === 'string' ? tip : tip.op_id),
    physical_ms: typeof tip === 'string' ? 0 : tip.physical_ms,
    logical: typeof tip === 'string' ? 0 : tip.logical,
    author: op.author,
  };
  return cmpKey(op, tipOp) < 0 && bytesToHex(op.op_id) !== bytesToHex(tipOp.op_id);
}

export function isLateAgainstOps(op, frontierOps) {
  return isLateOp(op, buildFrontier(frontierOps));
}

export function runFrontierBuildVector(vector) {
  const ops = (vector.ops || []).map(parseOp);
  const got = buildFrontier(ops);
  const gotIds = {};
  for (const [a, tip] of Object.entries(got)) gotIds[a] = tip.op_id;
  if (JSON.stringify(gotIds) !== JSON.stringify(vector.expect_frontier)) {
    throw new Error(
      `frontier mismatch:\n  expected ${JSON.stringify(vector.expect_frontier)}\n  got ${JSON.stringify(gotIds)}`
    );
  }
}

export function runSnapshotIdVector(vector) {
  const ops = (vector.ops || []).map(parseOp);
  const f = buildFrontier(ops);
  const root = bytesToHex(merkleRootOnce(ops));
  const tail = tailBoundary(ops);
  const id = snapshotId(vector.datastore_id_hex, f, root, tail);
  if (id !== vector.expect_snapshot_id_hex) {
    throw new Error(`snapshot id: expected ${vector.expect_snapshot_id_hex}, got ${id}`);
  }
  if (vector.expect_merkle_root_hex && root !== vector.expect_merkle_root_hex) {
    throw new Error(`merkle root: expected ${vector.expect_merkle_root_hex}, got ${root}`);
  }
}

export function runLateOpVector(vector) {
  const op = parseOp(vector.op);
  const late = vector.encoded_frontier
    ? isLateOp(op, vector.encoded_frontier)
    : isLateAgainstOps(op, (vector.frontier_ops || []).map(parseOp));
  if (late !== vector.expect_late) {
    throw new Error(`late: expected ${vector.expect_late}, got ${late}`);
  }
}
