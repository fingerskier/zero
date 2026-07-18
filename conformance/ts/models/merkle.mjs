// Merkle sync tree + mismatch-recovery walk per doc/MERKLE.md (M0c).
// Independent of the Rust core.

import { blake3 } from './blake3.mjs';
import { bytesToHex, hexToBytes } from './cbor.mjs';

const DOMAIN_LEAF = new TextEncoder().encode('zerodb-merkle-leaf-v1');
const DOMAIN_NODE = new TextEncoder().encode('zerodb-merkle-node-v1');
const DOMAIN_EMPTY = new TextEncoder().encode('zerodb-merkle-empty-v1');
const MERKLE_FORMAT_VERSION = 1;
const BUCKET_WIDTH_MS = 60_000;

function concatBytes(...parts) {
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

function u64be(n) {
  const b = new Uint8Array(8);
  const v = BigInt(n);
  for (let i = 7; i >= 0; i--) {
    b[i] = Number((v >> BigInt((7 - i) * 8)) & 0xffn);
  }
  return b;
}

function eq32(a, b) {
  for (let i = 0; i < 32; i++) if (a[i] !== b[i]) return false;
  return true;
}

export function emptyLeaf() {
  return blake3(concatBytes(DOMAIN_EMPTY, new Uint8Array([MERKLE_FORMAT_VERSION])));
}

function leafHash(bucketIndex, opIds) {
  const parts = [DOMAIN_LEAF, new Uint8Array([MERKLE_FORMAT_VERSION]), u64be(bucketIndex)];
  for (const id of opIds) parts.push(id);
  return blake3(concatBytes(...parts));
}

function nodeHash(left, right) {
  return blake3(
    concatBytes(DOMAIN_NODE, new Uint8Array([MERKLE_FORMAT_VERSION]), left, right)
  );
}

function nextPowerOfTwo(n) {
  if (n <= 1) return 1;
  let p = 1;
  while (p < n) p <<= 1;
  return p;
}

function cmpKey(a, b) {
  if (a.physical_ms !== b.physical_ms) return a.physical_ms < b.physical_ms ? -1 : 1;
  if (a.logical !== b.logical) return a.logical < b.logical ? -1 : 1;
  for (let i = 0; i < 32; i++) {
    if (a.author[i] !== b.author[i]) return a.author[i] < b.author[i] ? -1 : 1;
  }
  for (let i = 0; i < 32; i++) {
    if (a.op_id[i] !== b.op_id[i]) return a.op_id[i] < b.op_id[i] ? -1 : 1;
  }
  return 0;
}

function parseOp(o) {
  return {
    op_id: hexToBytes(o.op_id),
    physical_ms: o.physical_ms,
    logical: o.logical ?? 0,
    author: hexToBytes(o.author),
  };
}

export function buildTree(ops) {
  if (ops.length === 0) {
    const e = emptyLeaf();
    return {
      levels: [[e]],
      leaves: [{ hash: e, bucket_index: null, op_ids: [] }],
      ops: [],
    };
  }

  const buckets = new Map();
  for (const op of ops) {
    const idx = Math.floor(op.physical_ms / BUCKET_WIDTH_MS);
    if (!buckets.has(idx)) buckets.set(idx, []);
    buckets.get(idx).push(op);
  }

  const indices = [...buckets.keys()].sort((a, b) => a - b);
  const leaves = [];
  for (const idx of indices) {
    const members = buckets.get(idx).slice().sort(cmpKey);
    const op_ids = members.map((m) => m.op_id);
    leaves.push({
      hash: leafHash(idx, op_ids),
      bucket_index: idx,
      op_ids,
    });
  }

  const p = nextPowerOfTwo(leaves.length);
  const empty = emptyLeaf();
  while (leaves.length < p) {
    leaves.push({ hash: empty, bucket_index: null, op_ids: [] });
  }

  let level = leaves.map((l) => l.hash);
  const levels = [level.slice()];
  while (level.length > 1) {
    const next = [];
    for (let i = 0; i < level.length; i += 2) {
      next.push(nodeHash(level[i], level[i + 1]));
    }
    levels.push(next);
    level = next;
  }

  return { levels, leaves, ops: ops.slice() };
}

export function merkleRoot(ops) {
  return buildTree(ops).levels[buildTree(ops).levels.length - 1][0];
}

// efficient single build
export function merkleRootOnce(ops) {
  const t = buildTree(ops);
  return t.levels[t.levels.length - 1][0];
}

function nodeChildren(tree, level, index) {
  if (level === 0) return null;
  const childLevel = level - 1;
  if (!tree.levels[childLevel]) return null;
  const left = tree.levels[childLevel][index * 2];
  const right = tree.levels[childLevel][index * 2 + 1];
  if (!left || !right) return null;
  return [left, right];
}

function walkNode(local, remote, level, index, msgs, missing) {
  if (level === 0) {
    const remoteLeaf = remote.leaves[index];
    const localHash =
      (local.leaves[index] && local.leaves[index].hash) || emptyLeaf();
    if (eq32(remoteLeaf.hash, localHash)) return;

    msgs.push({
      type: 'LeafRequest',
      from: 'A',
      leaf_index: index,
      bucket_index: remoteLeaf.bucket_index,
    });
    const opIdsHex = remoteLeaf.op_ids.map((id) => bytesToHex(id));
    msgs.push({
      type: 'LeafResponse',
      from: 'B',
      leaf_index: index,
      bucket_index: remoteLeaf.bucket_index,
      op_ids: opIdsHex,
    });

    const localIds = new Set(
      (local.leaves[index] ? local.leaves[index].op_ids : []).map((id) => bytesToHex(id))
    );
    for (const id of remoteLeaf.op_ids) {
      const h = bytesToHex(id);
      if (!localIds.has(h)) missing.add(h);
    }
    return;
  }

  msgs.push({ type: 'NodeRequest', from: 'A', level, index });
  const [rleft, rright] = nodeChildren(remote, level, index);
  msgs.push({
    type: 'NodeResponse',
    from: 'B',
    level,
    index,
    hash: bytesToHex(remote.levels[level][index]),
    left: bytesToHex(rleft),
    right: bytesToHex(rright),
  });

  const localKids = nodeChildren(local, level, index);
  const lleft = localKids ? localKids[0] : emptyLeaf();
  const lright = localKids ? localKids[1] : emptyLeaf();

  if (!eq32(rleft, lleft)) walkNode(local, remote, level - 1, index * 2, msgs, missing);
  if (!eq32(rright, lright)) walkNode(local, remote, level - 1, index * 2 + 1, msgs, missing);
}

/**
 * A pulls missing ops from B. Fixtures should use B ⊇ A (or equal).
 * Both end at the union root.
 */
export function syncTranscript(requesterOps, responderOps) {
  const msgs = [];
  const reqTree = buildTree(requesterOps);
  const resTree = buildTree(responderOps);
  const reqRoot = bytesToHex(reqTree.levels[reqTree.levels.length - 1][0]);
  const resRoot = bytesToHex(resTree.levels[resTree.levels.length - 1][0]);

  msgs.push({ type: 'RootOffer', from: 'A', root: reqRoot });
  msgs.push({ type: 'RootOffer', from: 'B', root: resRoot });

  if (reqRoot === resRoot) {
    msgs.push({ type: 'Converged', root: reqRoot });
    return msgs;
  }

  const missing = new Set();
  const rootLevel = resTree.levels.length - 1;
  walkNode(reqTree, resTree, rootLevel, 0, msgs, missing);

  if (missing.size > 0) {
    msgs.push({
      type: 'Delta',
      from: 'B',
      op_ids: [...missing].sort(),
    });
  }

  const byId = new Map();
  for (const op of requesterOps) byId.set(bytesToHex(op.op_id), op);
  for (const op of responderOps) byId.set(bytesToHex(op.op_id), op);
  const merged = [...byId.values()];
  const finalRoot = bytesToHex(merkleRootOnce(merged));
  msgs.push({ type: 'RootOffer', from: 'A', root: finalRoot });
  msgs.push({ type: 'RootOffer', from: 'B', root: finalRoot });
  msgs.push({ type: 'Converged', root: finalRoot });
  return msgs;
}

function deepEqual(a, b) {
  return JSON.stringify(a) === JSON.stringify(b);
}

export function runMerkleRootVector(vector) {
  const ops = (vector.ops || []).map(parseOp);
  const root = merkleRootOnce(ops);
  const hex = bytesToHex(root);
  if (hex !== vector.root_hex) {
    throw new Error(`root mismatch:\n  expected ${vector.root_hex}\n  got      ${hex}`);
  }
  if (vector.ops_permuted) {
    const root2 = bytesToHex(merkleRootOnce(vector.ops_permuted.map(parseOp)));
    if (root2 !== vector.root_hex) {
      throw new Error(`permuted root mismatch: ${root2}`);
    }
  }
}

export function runMerkleTranscriptVector(vector) {
  const a = (vector.peer_a || []).map(parseOp);
  const b = (vector.peer_b || []).map(parseOp);
  const got = syncTranscript(a, b);
  if (vector.transcript) {
    if (!deepEqual(got, vector.transcript)) {
      throw new Error(
        `transcript mismatch:\n  expected ${JSON.stringify(vector.transcript, null, 2)}\n  got      ${JSON.stringify(got, null, 2)}`
      );
    }
  }
  const last = got[got.length - 1];
  if (last.type !== 'Converged') {
    throw new Error(`expected Converged, got ${last.type}`);
  }
  if (vector.final_root_hex && last.root !== vector.final_root_hex) {
    throw new Error(`final root: expected ${vector.final_root_hex}, got ${last.root}`);
  }
}
