// Epoch-aware replay model per doc/SCHEMA.md §3/§4.1. Materializes one
// property from its operation set and the canonical epoch chain, reusing the
// KERNEL §6 CRDT apply rules (imported unchanged from crdt.mjs) so epoch
// semantics can never drift from the base CRDT semantics.
//
// Pipeline: dedup by op_id (step 4) + equivocation exclusion (step 5) over the
// real op set; resolve the SchemaEpoch records into one canonical chain
// (lowest §4.5 order key wins a fork; losers + descendants quarantine); split
// the chain into maximal single-CRDT segments; pool each segment's ops into
// one replica; carry a change_crdt boundary across as a synthetic seed op at
// the SchemaEpoch order key. Order-independent by construction (I-1, I-17).

import { CRDTS } from './crdt.mjs';

// §4.5 total order over (physical_ms, logical, author, op_id).
function cmpKey(a, b) {
  if (a.physical_ms !== b.physical_ms) return a.physical_ms < b.physical_ms ? -1 : 1;
  if (a.logical !== b.logical) return a.logical < b.logical ? -1 : 1;
  if (a.author !== b.author) return a.author < b.author ? -1 : 1;
  if (a.op_id !== b.op_id) return a.op_id < b.op_id ? -1 : 1;
  return 0;
}

// Resolve SchemaEpoch records into one linear chain. Level 1's winner is the
// lowest-order-key record with prev=null; level n's winner is the lowest-key
// record whose prev is level n-1's winner. Everything else quarantines.
function resolveChain(records) {
  const byLevel = new Map();
  for (const r of records) {
    if (!byLevel.has(r.epoch)) byLevel.set(r.epoch, []);
    byLevel.get(r.epoch).push(r);
  }
  const levels = [...byLevel.keys()].sort((a, b) => a - b);
  const winners = new Map();
  const quarantined = new Set();
  let prevWinner = null; // record id of the previous level's winner (null baseline)
  let broken = false;
  for (const level of levels) {
    const atLevel = byLevel.get(level);
    if (broken) {
      for (const r of atLevel) quarantined.add(r.record);
      continue;
    }
    const candidates = atLevel.filter((r) => (r.prev ?? null) === prevWinner);
    for (const r of atLevel) if (!candidates.includes(r)) quarantined.add(r.record);
    if (candidates.length === 0) {
      broken = true; // chain breaks; later ops here are EPOCH_UNKNOWN
      continue;
    }
    let win = candidates[0];
    for (const c of candidates.slice(1)) if (cmpKey(c.order_key, win.order_key) < 0) win = c;
    winners.set(level, win);
    for (const c of candidates) if (c !== win) quarantined.add(c.record);
    prevWinner = win.record;
  }
  return { winners, quarantined };
}

// A change_crdt boundary materializes the completed segment's value into one
// synthetic seed op of the next type, placed exactly at the SchemaEpoch op's
// order key so post-migration writes outrank it iff genuinely newer.
function buildSeed(migration, prevRead, seedKey) {
  const base = {
    op_id: seedKey.op_id,
    author: seedKey.author,
    ts: { physical_ms: seedKey.physical_ms, logical: seedKey.logical },
  };
  switch (migration.transform) {
    case 'counter_value_to_lww_int':
      return { ...base, payload: { set: prevRead.value } };
    case 'reset_to':
      return { ...base, payload: { set: migration.default ?? null } };
    default:
      throw new Error(`unsupported change_crdt transform ${migration.transform}`);
  }
}

// Materialize one property. Returns { read, quarantined }. Throws Error with
// message 'EPOCH_UNKNOWN' when a surviving op resolves to an epoch absent from
// the known chain.
export function materializeEpoch(ops, records) {
  // Steps 4-5: dedup by op_id, exclude equivocation groups (author + equal ts).
  const ingested = new Map();
  for (const op of ops) if (!ingested.has(op.op_id)) ingested.set(op.op_id, op);
  const groups = new Map();
  for (const op of ingested.values()) {
    const k = `${op.author}|${op.ts.physical_ms}|${op.ts.logical}`;
    if (!groups.has(k)) groups.set(k, []);
    groups.get(k).push(op.op_id);
  }
  const equivocated = new Set();
  for (const ids of groups.values()) if (ids.length > 1) for (const id of ids) equivocated.add(id);

  const { winners, quarantined } = resolveChain(records);
  const recById = new Map(records.map((r) => [r.record, r]));

  // Resolve each surviving op to a level, or mark it quarantined/EPOCH_UNKNOWN.
  const quarantinedOps = [];
  const activeByLevel = new Map();
  const addActive = (level, op) => {
    if (!activeByLevel.has(level)) activeByLevel.set(level, []);
    activeByLevel.get(level).push(op);
  };
  for (const op of ingested.values()) {
    if (equivocated.has(op.op_id)) continue; // excluded from state
    if (op.ep_record !== undefined) {
      const rec = recById.get(op.ep_record);
      if (!rec) throw new Error('EPOCH_UNKNOWN');
      if (quarantined.has(rec.record)) {
        quarantinedOps.push(op.op_id);
        continue;
      }
      addActive(rec.epoch, op);
    } else if (op.ep === 0) {
      addActive(0, op); // schemaless default (lww)
    } else {
      if (!winners.has(op.ep)) throw new Error('EPOCH_UNKNOWN');
      addActive(op.ep, op);
    }
  }

  // Build the level walk and split into single-CRDT segments (a change_crdt
  // migration starts a new segment; add_prop/entity steps do not).
  const maxLevel = winners.size ? Math.max(...winners.keys()) : 0;
  const hasEp0 = activeByLevel.has(0);
  const startLevel = hasEp0 ? 0 : winners.size ? Math.min(...winners.keys()) : 0;

  const segments = [];
  for (let l = startLevel; l <= maxLevel; l += 1) {
    if (l !== 0 && !winners.has(l)) continue;
    const crdt = l === 0 ? 'lww' : winners.get(l).crdt;
    const migration = l === 0 ? null : winners.get(l).migration ?? null;
    if (segments.length === 0 || migration !== null) {
      segments.push({
        crdt,
        migration,
        seedKey: l === 0 ? null : winners.get(l).order_key,
        levels: [l],
      });
    } else {
      const seg = segments[segments.length - 1];
      if (seg.crdt !== crdt) throw new Error('SCHEMA_TYPE_CHANGE_WITHOUT_MIGRATION');
      seg.levels.push(l);
    }
  }

  // Materialize segment by segment; each is a set-based CRDT materialization.
  let prevRead = { value: null };
  for (let si = 0; si < segments.length; si += 1) {
    const seg = segments[si];
    const model = CRDTS[seg.crdt];
    if (!model) throw new Error(`unknown crdt "${seg.crdt}"`);
    const state = model.init();
    if (si > 0) model.apply(state, buildSeed(seg.migration, prevRead, seg.seedKey));
    const opsInSeg = [];
    for (const l of seg.levels) for (const op of activeByLevel.get(l) ?? []) opsInSeg.push(op);
    // Deterministic set order (arrival order is irrelevant by CRDT convergence).
    opsInSeg.sort((a, b) => (a.op_id < b.op_id ? -1 : a.op_id > b.op_id ? 1 : 0));
    for (const op of opsInSeg) model.apply(state, op);
    prevRead = model.read(state);
  }

  return { read: prevRead, quarantined: quarantinedOps };
}

export function runEpochReplayVector(vector) {
  vector.orders.forEach((order, oi) => {
    const ops = order.map((i) => {
      const op = vector.ops[i];
      if (op === undefined) throw new Error(`order ${oi}: bad op index ${i}`);
      return op;
    });

    let read = null;
    let quarantined = [];
    let error = null;
    try {
      const r = materializeEpoch(ops, vector.epochs);
      read = r.read;
      quarantined = r.quarantined;
    } catch (e) {
      error = e.message;
    }

    if (vector.expect_error) {
      if (error !== vector.expect_error) {
        throw new Error(`order ${oi}: expected error ${vector.expect_error}, got ${error ?? 'success'}`);
      }
      return;
    }
    if (error !== null) throw new Error(`order ${oi}: unexpected error ${error}`);

    if (JSON.stringify(read) !== JSON.stringify(vector.expect)) {
      throw new Error(`order ${oi} [${order.join(',')}]: expected ${JSON.stringify(vector.expect)}, got ${JSON.stringify(read)}`);
    }
    const expectQ = [...(vector.expect_quarantine ?? [])].sort();
    const gotQ = [...quarantined].sort();
    if (JSON.stringify(gotQ) !== JSON.stringify(expectQ)) {
      throw new Error(`order ${oi}: quarantine ${JSON.stringify(gotQ)}, expected ${JSON.stringify(expectQ)}`);
    }
  });
}
