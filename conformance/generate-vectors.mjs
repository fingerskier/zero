#!/usr/bin/env node
// Author M3c-c golden/negative relay+peer fixtures from the live RELAY 0.2
// codecs (never NAPI). Writes JSON vectors; does not freeze any format.
//
// Usage:
//   node conformance/generate-vectors.mjs --lane xfail
//   node conformance/generate-vectors.mjs --lane required

import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

import { bytesToHex, hexToBytes } from './ts/models/cbor.mjs'
import {
  MSG_SYNC_REQUEST,
  MSG_SYNC_RESPONSE,
  MSG_MERKLE_NODE_REQUEST,
  MSG_MERKLE_NODE_RESPONSE,
  MSG_MERKLE_LEAF_REQUEST,
  MSG_MERKLE_LEAF_RESPONSE,
  MSG_DELTA_REQUEST,
  MSG_DELTA_BATCH,
  MSG_OPS,
  MSG_OP_ACK,
  MSG_ERROR,
  DIR_PEER_TO_RELAY,
  DIR_RELAY_TO_PEER,
  ERR_PAYLOAD_TOO_LARGE,
  encodeEnvelope,
  merkleWalkMissing,
} from './ts/models/relay.mjs'
import { merkleRootOnce } from './ts/models/merkle.mjs'
import { PeerStore } from './ts/peer/store.mjs'

const here = dirname(fileURLToPath(import.meta.url))
const laneArg = process.argv.indexOf('--lane')
const lane = laneArg === -1 ? 'xfail' : process.argv[laneArg + 1]
if (!['required', 'xfail'].includes(lane)) {
  console.error(`unknown lane ${lane}`)
  process.exit(2)
}

const SEED_A = hexToBytes('11'.repeat(32))
const SALT_A = hexToBytes('22'.repeat(16))
const SEED_B = hexToBytes('33'.repeat(32))
const SALT_B = hexToBytes('44'.repeat(16))
const WALL = 1_700_000_000_000
const TODO_PIN = { nodes: { Todo: { props: { title: 'lww' } } } }

function frame(dir, type, requestId, payload) {
  return {
    dir,
    type,
    request_id: requestId,
    payload,
    cbor_hex: bytesToHex(encodeEnvelope(type, requestId, payload)),
  }
}

function writeVector(subdir, name, vector) {
  const dir = join(here, 'vectors', lane, subdir)
  mkdirSync(dir, { recursive: true })
  const path = join(dir, name)
  writeFileSync(path, `${JSON.stringify(vector, null, 2)}\n`)
  console.log(`wrote ${path}`)
}

function storeA() {
  return new PeerStore({ seed: SEED_A, salt: SALT_A, clock: () => WALL })
}

function storeB() {
  return new PeerStore({ seed: SEED_B, salt: SALT_B, clock: () => WALL })
}

function relayOp(wire) {
  return {
    op_id: wire.id,
    author: wire.author,
    physical_ms: wire.ts.p,
    logical: wire.ts.l,
    wire: JSON.stringify(wire),
  }
}

function merkleOp(wire) {
  return {
    op_id: wire.id,
    author: wire.author,
    physical_ms: wire.ts.p,
    logical: wire.ts.l,
  }
}

const a = storeA()
a.applySchemaEpoch(TODO_PIN)
const created = a.createNode('Todo', '0102030405060708090a0b0c0d0e0f10')
const set = a.setLww(created.node, 'title', 'milk')
const epoch = a.ops[0]

const b = storeB()
b.applySchemaEpoch(TODO_PIN)
const foreign = b.createNode('Todo', 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa')

const future = a.commit(3, { node: created.node, path: 'title', crdt: 'lww', value: 'x' }, { ep: 2 })

const farClock = new PeerStore({ seed: SEED_A, salt: SALT_A, clock: () => WALL + 120_000 })
farClock.adoptDatastore(a.dsHex)
const far = farClock.commit(1, { label: 'Todo', node: 'ffffffffffffffffffffffffffffffff' }, { ep: 0 })

const badSig = { ...set, sig: set.sig.replace(/^../, (h) => (h === '00' ? '01' : '00')) }

writeVector('relay', 'RELAY-OPS-001-push-ack.json', {
  id: 'RELAY-OPS-001',
  type: 'relay-transcript',
  kind: 'ops-push',
  invariants: ['I-6', 'I-9'],
  description:
    'Golden OPS push + OP_ACK ACCEPT for a signed CreateNode (RELAY §4.4). Live RELAY 0.2 draft; not a format freeze.',
  frames: [
    frame(DIR_PEER_TO_RELAY, MSG_OPS, 1, {
      datastore: a.dsHex,
      operations: [relayOp(created.wire)],
    }),
    frame(DIR_RELAY_TO_PEER, MSG_OP_ACK, 1, {
      outcomes: [{ op_id: created.wire.id, outcome: 'ACCEPT' }],
    }),
  ],
  expect: {
    outcomes: [{ op_id: created.wire.id, outcome: 'ACCEPT' }],
  },
})

writeVector('relay', 'RELAY-LIMIT-001-batch-ops.json', {
  id: 'RELAY-LIMIT-001',
  type: 'relay-transcript',
  kind: 'limits',
  invariants: ['I-12'],
  description:
    'OPS with two operations against advertised max_batch_ops=1 is ERROR 0x303 PAYLOAD_TOO_LARGE (RELAY §8.1 / §10.2).',
  limits: {
    max_payload_bytes: 1048576,
    max_batch_ops: 1,
    max_batch_bytes: 16777216,
    max_subscriptions: 64,
    ops_per_second: 100,
    bytes_per_second: 10485760,
  },
  frames: [
    frame(DIR_PEER_TO_RELAY, MSG_OPS, 1, {
      datastore: a.dsHex,
      operations: [relayOp(created.wire), relayOp(set)],
    }),
    frame(DIR_RELAY_TO_PEER, MSG_ERROR, 1, {
      code: ERR_PAYLOAD_TOO_LARGE,
      message: 'PAYLOAD_TOO_LARGE',
      fatal: false,
    }),
  ],
  expect: { error_code: ERR_PAYLOAD_TOO_LARGE },
})

const op1 = {
  op_id: '0101010101010101010101010101010101010101010101010101010101010101',
  physical_ms: 1000,
  logical: 0,
  author: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
}
const op2 = {
  op_id: '0202020202020202020202020202020202020202020202020202020202020202',
  physical_ms: 70000,
  logical: 0,
  author: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
}
const local = [op1]
const remote = [op1, op2]
const walk = merkleWalkMissing(
  local.map((o) => ({
    op_id: hexToBytes(o.op_id),
    physical_ms: o.physical_ms,
    logical: o.logical,
    author: hexToBytes(o.author),
  })),
  remote.map((o) => ({
    op_id: hexToBytes(o.op_id),
    physical_ms: o.physical_ms,
    logical: o.logical,
    author: hexToBytes(o.author),
  })),
)
const ds = 'aa'.repeat(32)
const walkFrames = []
let rid = 1
walkFrames.push(
  frame(DIR_PEER_TO_RELAY, MSG_SYNC_REQUEST, rid, {
    datastore: ds,
    accepted_root: walk.localRoot,
    cursor: { epoch: 0, frontier: {} },
  }),
)
walkFrames.push(
  frame(DIR_RELAY_TO_PEER, MSG_SYNC_RESPONSE, rid, {
    datastore: ds,
    validated_root: walk.remoteRoot,
    merkle_format_version: 1,
    bucket_width_ms: 60000,
    bucket_indices: walk.buckets,
  }),
)
rid += 1
for (const step of walk.steps) {
  if (step.type === 'node') {
    walkFrames.push(
      frame(DIR_PEER_TO_RELAY, MSG_MERKLE_NODE_REQUEST, rid, {
        datastore: ds,
        level: step.level,
        index: step.index,
      }),
    )
    walkFrames.push(
      frame(DIR_RELAY_TO_PEER, MSG_MERKLE_NODE_RESPONSE, rid, {
        datastore: ds,
        level: step.level,
        index: step.index,
        hash: step.hash,
        left: step.left,
        right: step.right,
      }),
    )
    rid += 1
  } else {
    walkFrames.push(
      frame(DIR_PEER_TO_RELAY, MSG_MERKLE_LEAF_REQUEST, rid, {
        datastore: ds,
        leaf_index: step.index,
      }),
    )
    walkFrames.push(
      frame(DIR_RELAY_TO_PEER, MSG_MERKLE_LEAF_RESPONSE, rid, {
        datastore: ds,
        leaf_index: step.index,
        bucket_index: walk.buckets[step.index] ?? step.index,
        op_ids: remote
          .filter((o) => Math.floor(o.physical_ms / 60000) === (walk.buckets[step.index] ?? step.index))
          .map((o) => o.op_id),
      }),
    )
    rid += 1
  }
}
if (walk.missing.length) {
  walkFrames.push(
    frame(DIR_PEER_TO_RELAY, MSG_DELTA_REQUEST, rid, {
      datastore: ds,
      op_ids: walk.missing,
    }),
  )
  walkFrames.push(
    frame(DIR_RELAY_TO_PEER, MSG_DELTA_BATCH, rid, {
      datastore: ds,
      operations: walk.missing.map((id) => ({ op_id: id })),
      remaining: 0,
    }),
  )
}

writeVector('relay', 'RELAY-WALK-001-catchup.json', {
  id: 'RELAY-WALK-001',
  type: 'relay-transcript',
  kind: 'merkle-walk',
  invariants: ['I-11'],
  description:
    'merkle-walk-v1 catch-up: local has bucket 0, remote has bucket 0+1; walk prunes the equal leaf and DELTA the missing OpId (RELAY §4.3).',
  local,
  remote,
  bucket_indices: walk.buckets,
  frames: walkFrames,
  expect: { missing: walk.missing },
})

writeVector('peer', 'PEER-EPOCH-001-schema-n1.json', {
  id: 'PEER-EPOCH-001',
  type: 'peer-ingest',
  kind: 'schema-epoch',
  invariants: ['I-17'],
  description:
    'Signed SchemaEpoch n=1 / prev=null / empty migration ingests; a following CreateNode at ep=1 applies. Draft wrap-body; not a format freeze.',
  seed: bytesToHex(SEED_A),
  salt: bytesToHex(SALT_A),
  clock: WALL,
  datastore: a.dsHex,
  setup: [],
  ingest: [epoch, created.wire],
  expect: {
    schema_epoch: 1,
    applied: [epoch.id, created.wire.id],
  },
})

writeVector('peer', 'PEER-REJECT-001-wrong-datastore.json', {
  id: 'PEER-REJECT-001',
  type: 'peer-ingest',
  kind: 'named-reject',
  invariants: ['I-9'],
  description: 'Well-signed op whose ds is B is AUTH_WRONG_DATASTORE when joined to A.',
  seed: bytesToHex(SEED_A),
  salt: bytesToHex(SALT_A),
  clock: WALL,
  datastore: a.dsHex,
  expected_ds: a.dsHex,
  setup: [epoch],
  ingest: [foreign.wire],
  expect: {
    rejects: [{ op_id: foreign.wire.id, reason: 'AUTH_WRONG_DATASTORE' }],
  },
})

writeVector('peer', 'PEER-REJECT-002-epoch-unknown.json', {
  id: 'PEER-REJECT-002',
  type: 'peer-ingest',
  kind: 'named-reject',
  invariants: ['I-17'],
  description: 'Data op with ep past the applied kind-5 chain is EPOCH_UNKNOWN and does not apply.',
  seed: bytesToHex(SEED_A),
  salt: bytesToHex(SALT_A),
  clock: WALL,
  datastore: a.dsHex,
  setup: [epoch, created.wire, set],
  ingest: [future],
  expect: {
    schema_epoch: 1,
    rejects: [{ op_id: future.id, reason: 'EPOCH_UNKNOWN' }],
    lww: [{ node: created.node, path: 'title', value: 'milk' }],
  },
})

writeVector('peer', 'PEER-REJECT-003-clock-drift.json', {
  id: 'PEER-REJECT-003',
  type: 'peer-ingest',
  kind: 'named-reject',
  invariants: ['I-5'],
  description: 'Signed member op with ts.p > wall + 60s is CLOCK_DRIFT and does not apply (KERNEL §5 / AUTH §6).',
  seed: bytesToHex(SEED_B),
  salt: bytesToHex(SALT_B),
  clock: WALL,
  datastore: a.dsHex,
  expected_ds: a.dsHex,
  setup: [],
  ingest: [far],
  expect: {
    rejects: [{ op_id: far.id, reason: 'CLOCK_DRIFT' }],
  },
})

writeVector('peer', 'PEER-REJECT-004-sig-invalid.json', {
  id: 'PEER-REJECT-004',
  type: 'peer-ingest',
  kind: 'named-reject',
  invariants: ['I-8'],
  description: 'Flipped envelope signature is AUTH_SIG_INVALID.',
  seed: bytesToHex(SEED_A),
  salt: bytesToHex(SALT_A),
  clock: WALL,
  datastore: a.dsHex,
  setup: [epoch, created.wire],
  ingest: [badSig],
  expect: {
    rejects: [{ op_id: badSig.id, reason: 'AUTH_SIG_INVALID' }],
  },
})

void merkleRootOnce
void merkleOp
console.log(`[generate-vectors] lane=${lane}`)
