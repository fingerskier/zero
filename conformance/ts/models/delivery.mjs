// Delivery / anti-replay / resume model per doc/DELIVERY.md (M0e.2).

import { hexToBytes, bytesToHex } from './cbor.mjs';

function runSchedule(steps) {
  const seen = new Set();
  const held = new Set();
  let lastOutcomes = [];
  let resumeSent = [];

  for (const step of steps) {
    switch (step.op) {
      case 'hold':
        held.add(step.op_id);
        break;
      case 'deliver': {
        if (step.reject) {
          lastOutcomes = ['REJECT'];
        } else if (seen.has(step.op_id)) {
          lastOutcomes = ['DUPLICATE'];
        } else {
          seen.add(step.op_id);
          lastOutcomes = ['ACCEPT'];
        }
        break;
      }
      case 'deliver_batch': {
        lastOutcomes = [];
        const rejects = step.rejects || [];
        step.op_ids.forEach((id, i) => {
          if (rejects[i]) lastOutcomes.push('REJECT');
          else if (seen.has(id)) lastOutcomes.push('DUPLICATE');
          else {
            seen.add(id);
            lastOutcomes.push('ACCEPT');
          }
        });
        break;
      }
      case 'resume': {
        resumeSent = [...held].filter((id) => !seen.has(id)).sort();
        for (const id of resumeSent) seen.add(id);
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
