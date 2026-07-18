#!/usr/bin/env node
// Independent conformance model runner (JS/TS side).
// Pure encoder/decoder + semantic models only — never backed by the Rust core.
//
// Usage: node conformance/ts/runner.mjs [--lane required|xfail]
//   required (default): every vector must pass; unregistered types fail.
//   xfail: failures are reported but exit 0 (expected-failure lane).

import { readdirSync, readFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

import { runHlcVector } from './models/hlc.mjs';
import { runCrdtVector } from './models/crdt.mjs';
import {
  runOpEncodingVector,
  runOpDecodeNegativeVector,
  runOpSignatureVector,
} from './models/op.mjs';
import { runEnvelopeVector } from './models/envelope.mjs';
import { runSchemaIrVector, runSchemaIrInvalidVector } from './models/schema.mjs';
import { runEpochReplayVector } from './models/epoch.mjs';
import { runMigrationTransformVector } from './models/migration.mjs';
import { runQueryParseVector } from './models/query.mjs';
import { runQueryEvalVector } from './models/queryeval.mjs';
import {
  runDeviceCertVector,
  runGenesisIdVector,
  runAuthzPredicateVector,
  runAdmissionTokenVector,
} from './models/auth.mjs';
import { runWalCrashVector, runGroupSealVector } from './models/wal.mjs';
import { runMerkleRootVector, runMerkleTranscriptVector, merkleRootOnce } from './models/merkle.mjs';
import { runDeliveryScheduleVector } from './models/delivery.mjs';
import {
  runFrontierBuildVector,
  runSnapshotIdVector,
  runLateOpVector,
  isLateAgainstOps,
} from './models/frontier.mjs';
import { hexToBytes, bytesToHex } from './models/cbor.mjs';

const laneArg = process.argv.indexOf('--lane');
const lane = laneArg === -1 ? 'required' : process.argv[laneArg + 1];
if (!['required', 'xfail'].includes(lane)) {
  console.error(`unknown lane: ${lane}`);
  process.exit(2);
}

// Vector-type dispatch table. M0 packages register their handlers here
// (e.g. 'op-encoding' for M0a golden bytes, 'merkle-transcript' for M0c).
const handlers = {
  'hlc-transition': runHlcVector,
  'crdt-apply': runCrdtVector,
  'op-encoding': runOpEncodingVector,
  'op-decode-negative': runOpDecodeNegativeVector,
  'op-signature': runOpSignatureVector,
  envelope: runEnvelopeVector,
  'schema-ir': runSchemaIrVector,
  'schema-ir-invalid': runSchemaIrInvalidVector,
  'epoch-replay': runEpochReplayVector,
  'migration-transform': runMigrationTransformVector,
  'query-parse': runQueryParseVector,
  'query-eval': runQueryEvalVector,
  'device-cert': runDeviceCertVector,
  'genesis-id': runGenesisIdVector,
  'authz-predicate': runAuthzPredicateVector,
  'admission-token': runAdmissionTokenVector,
  'wal-crash': runWalCrashVector,
  'group-seal': runGroupSealVector,
  'merkle-root': runMerkleRootVector,
  'merkle-transcript': runMerkleTranscriptVector,
  'delivery-schedule': runDeliveryScheduleVector,
  'frontier-build': runFrontierBuildVector,
  'snapshot-id': runSnapshotIdVector,
  'late-op': runLateOpVector,
  'composite-smoke': (vector) => {
    const op = {
      op_id: hexToBytes(vector.op.op_id),
      physical_ms: vector.op.physical_ms,
      logical: vector.op.logical ?? 0,
      author: hexToBytes(vector.op.author),
    };
    const root = bytesToHex(merkleRootOnce([op]));
    if (root !== vector.expect_merkle_root_hex) {
      throw new Error(`merkle root: expected ${vector.expect_merkle_root_hex}, got ${root}`);
    }
    // delivery single accept
    runDeliveryScheduleVector({
      schedule: [{ op: 'deliver', op_id: vector.op.op_id }],
      expect: { last_outcomes: [vector.expect_delivery] },
    });
    const late = isLateAgainstOps(op, [op]);
    if (late !== vector.expect_late) {
      throw new Error(`late: expected ${vector.expect_late}, got ${late}`);
    }
  },
};

const vectorsDir = join(dirname(fileURLToPath(import.meta.url)), '..', 'vectors', lane);

let files = [];
try {
  files = readdirSync(vectorsDir, { recursive: true })
    .filter((f) => String(f).endsWith('.json'))
    .sort();
} catch {
  // Lane directory absent — zero vectors is a valid (green) state pre-M0a.
}

let passed = 0;
const failures = [];

for (const file of files) {
  const path = join(vectorsDir, String(file));
  let vector;
  try {
    vector = JSON.parse(readFileSync(path, 'utf8'));
  } catch (e) {
    failures.push(`${file}: unparseable vector (${e.message})`);
    continue;
  }
  const handler = handlers[vector.type];
  if (!handler) {
    failures.push(`${file}: no handler registered for type "${vector.type}" — a vector without a runner is not evidence`);
    continue;
  }
  try {
    handler(vector);
    passed += 1;
  } catch (e) {
    failures.push(`${file}: ${e.message}`);
  }
}

console.log(`[conformance:${lane}] ${passed} passed, ${failures.length} failed, ${files.length} total`);
for (const f of failures) console.log(`  FAIL ${f}`);

process.exit(lane === 'xfail' ? 0 : failures.length === 0 ? 0 : 1);
