// Delivery / anti-replay / resume model per doc/DELIVERY.md (M0e.2).

import { hexToBytes, bytesToHex } from './cbor.mjs';

function orderKey(op) {
  return [op.physical_ms, op.logical, op.author, op.op_id];
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

function knownFrom(step) {
  const id = step.op_id;
  return {
    op_id: id,
    author: step.author || id,
    physical_ms: step.physical_ms ?? 0,
    logical: step.logical ?? 0,
  };
}

function receiverFrontier(seenMeta) {
  const best = new Map();
  for (const op of seenMeta.values()) {
    if (!best.has(op.author) || cmpKey(op, best.get(op.author)) > 0) {
      best.set(op.author, op);
    }
  }
  return best;
}

function covered(frontier, op) {
  const tip = frontier.get(op.author);
  if (!tip) return false;
  return cmpKey(op, tip) <= 0;
}

function runSchedule(steps) {
  const seen = new Set();
  const seenMeta = new Map();
  const held = new Map();
  let lastOutcomes = [];
  let resumeSent = [];

  const accept = (op) => {
    if (seen.has(op.op_id)) return 'DUPLICATE';
    seen.add(op.op_id);
    seenMeta.set(op.op_id, op);
    return 'ACCEPT';
  };

  for (const step of steps) {
    switch (step.op) {
      case 'hold':
        held.set(step.op_id, knownFrom(step));
        break;
      case 'deliver': {
        if (step.reject) {
          lastOutcomes = ['REJECT'];
        } else {
          const op = held.get(step.op_id) || knownFrom(step);
          lastOutcomes = [accept(op)];
        }
        break;
      }
      case 'deliver_batch': {
        lastOutcomes = [];
        const rejects = step.rejects || [];
        step.op_ids.forEach((id, i) => {
          if (rejects[i]) lastOutcomes.push('REJECT');
          else {
            const op = held.get(id) || { op_id: id, author: id, physical_ms: 0, logical: 0 };
            lastOutcomes.push(accept(op));
          }
        });
        break;
      }
      case 'resume': {
        const frontier = receiverFrontier(seenMeta);
        resumeSent = [...held.values()]
          .filter((op) => !covered(frontier, op))
          .map((op) => op.op_id)
          .sort();
        for (const id of resumeSent) accept(held.get(id));
        break;
      }
      default:
        throw new Error(`unknown step ${step.op}`);
    }
  }

  return {
    seen: [...seen].sort(),
    last_outcomes: lastOutcomes,
    resume_sent: resumeSent,
  };
}

export function runDeliveryScheduleVector(vector) {
  const got = runSchedule(vector.schedule);
  const exp = vector.expect;
  if (exp.seen) {
    const want = [...exp.seen].sort();
    if (JSON.stringify(got.seen) !== JSON.stringify(want)) {
      throw new Error(`seen: expected ${want}, got ${got.seen}`);
    }
  }
  if (exp.last_outcomes) {
    if (JSON.stringify(got.last_outcomes) !== JSON.stringify(exp.last_outcomes)) {
      throw new Error(
        `outcomes: expected ${exp.last_outcomes}, got ${got.last_outcomes}`
      );
    }
  }
  if (exp.resume_sent) {
    const want = [...exp.resume_sent].sort();
    if (JSON.stringify(got.resume_sent) !== JSON.stringify(want)) {
      throw new Error(`resume_sent: expected ${want}, got ${got.resume_sent}`);
    }
  }
}

void hexToBytes;
void bytesToHex;
