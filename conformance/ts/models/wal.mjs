// WAL / group reference model per doc/WAL.md (M0e.1). Pure in-memory.

function cloneState(st) {
  return {
    wal: st.wal.map((r) => ({ ...r, author_ts: { ...r.author_ts } })),
    wal_buf: st.wal_buf.map((r) => ({ ...r, author_ts: { ...r.author_ts } })),
    applied: new Set(st.applied),
    material: { ...st.material },
    hlc: st.hlc ? { ...st.hlc } : null,
    sealed: new Set(st.sealed),
  };
}

function emptyState() {
  return {
    wal: [],
    wal_buf: [],
    applied: new Set(),
    material: {},
    hlc: null,
    sealed: new Set(),
  };
}

function walAppend(st, rec) {
  st.wal_buf.push(rec);
}

function walSync(st) {
  st.wal.push(...st.wal_buf);
  st.wal_buf = [];
}

function stateApply(st, opId) {
  if (st.applied.has(opId)) return 'ok';
  const rec = st.wal.find((r) => r.op_id === opId);
  if (!rec) return 'UNKNOWN_OP';
  st.material[opId] = rec.body;
  st.applied.add(opId);
  return 'ok';
}

function tsEq(a, b) {
  return a && b && a.p === b.p && a.l === b.l;
}

function hlcPersist(st, ts) {
  if (!st.wal.some((r) => tsEq(r.author_ts, ts))) return 'INVALID';
  st.hlc = { ...ts };
  return 'ok';
}

function groupSeal(st, m) {
  if (m.abort) return 'GROUP_ABORTED';
  if (!m.members || m.members.length === 0) return 'INVALID';
  if (m.n !== undefined && m.n !== m.members.length) return 'INVALID';
  if (new Set(m.members).size !== m.members.length) return 'INVALID';
  for (const id of m.members) {
    if (!st.applied.has(id)) return 'GROUP_INCOMPLETE';
    const rec = st.wal.find((r) => r.op_id === id);
    if (!rec || rec.group !== m.group_id) return 'INVALID';
  }
  st.sealed.add(m.group_id);
  return 'ok';
}

function visibleMaterial(st) {
  const out = [];
  for (const id of st.applied) {
    const rec = st.wal.find((r) => r.op_id === id);
    if (!rec || rec.group == null || st.sealed.has(rec.group)) out.push(id);
  }
  return out;
}

function walTruncate(st, n) {
  if (n > st.wal.length) return 'INVALID';
  for (let i = 0; i < n; i++) {
    const rec = st.wal[i];
    if (!st.applied.has(rec.op_id)) return 'TRUNCATE_UNSAFE';
    if (rec.group != null && !st.sealed.has(rec.group)) return 'TRUNCATE_UNSAFE';
  }
  st.wal = st.wal.slice(n);
  return 'ok';
}

function recover(st) {
  st.wal_buf = [];
  for (const rec of st.wal) {
    stateApply(st, rec.op_id);
  }
  let max = null;
  for (const rec of st.wal) {
    const t = rec.author_ts;
    if (
      !max ||
      t.p > max.p ||
      (t.p === max.p && t.l > max.l)
    ) {
      max = { p: t.p, l: t.l };
    }
  }
  st.hlc = max;
}

function runStep(st, step) {
  switch (step.op) {
    case 'wal_append':
      walAppend(st, step.record);
      return 'ok';
    case 'wal_sync':
      walSync(st);
      return 'ok';
    case 'state_apply':
      return stateApply(st, step.op_id);
    case 'hlc_persist':
      return hlcPersist(st, step.ts);
    case 'group_seal':
      return groupSeal(st, step.manifest);
    case 'wal_truncate':
      return walTruncate(st, step.up_to_index);
    default:
      throw new Error(`unknown step op ${step.op}`);
  }
}

function runUntilCrash(schedule, crashAfterStep) {
  const st = emptyState();
  const last =
    crashAfterStep < 0 ? -1 : Math.min(crashAfterStep, schedule.length - 1);
  for (let i = 0; i < schedule.length; i++) {
    if (i > last) break;
    runStep(st, schedule[i]);
  }
  recover(st);
  return st;
}

function expectMatch(st, exp) {
  if (exp.wal_len !== undefined && st.wal.length !== exp.wal_len) {
    throw new Error(`wal_len: expected ${exp.wal_len}, got ${st.wal.length}`);
  }
  if (exp.applied) {
    const got = [...st.applied].sort();
    const want = [...exp.applied].sort();
    if (JSON.stringify(got) !== JSON.stringify(want)) {
      throw new Error(`applied: expected ${want}, got ${got}`);
    }
  }
  if (exp.hlc === null) {
    if (st.hlc !== null) throw new Error(`hlc: expected null, got ${JSON.stringify(st.hlc)}`);
  } else if (exp.hlc) {
    if (!st.hlc || st.hlc.p !== exp.hlc.p || st.hlc.l !== exp.hlc.l) {
      throw new Error(`hlc: expected ${JSON.stringify(exp.hlc)}, got ${JSON.stringify(st.hlc)}`);
    }
  }
  if (exp.visible) {
    const got = visibleMaterial(st).sort();
    const want = [...exp.visible].sort();
    if (JSON.stringify(got) !== JSON.stringify(want)) {
      throw new Error(`visible: expected ${want}, got ${got}`);
    }
  }
  if (exp.sealed) {
    const got = [...st.sealed].sort();
    const want = [...exp.sealed].sort();
    if (JSON.stringify(got) !== JSON.stringify(want)) {
      throw new Error(`sealed: expected ${want}, got ${got}`);
    }
  }
}

export function runWalCrashVector(vector) {
  const st = runUntilCrash(vector.schedule, vector.crash_after_step);
  expectMatch(st, vector.expect_after_recover);
}

export function runGroupSealVector(vector) {
  const st = emptyState();
  for (const step of vector.schedule) {
    const outcome = runStep(st, step);
    if (step.expect_error) {
      if (outcome !== step.expect_error) {
        throw new Error(`step expect ${step.expect_error}, got ${outcome}`);
      }
    } else if (outcome !== 'ok') {
      throw new Error(`step failed: ${outcome}`);
    }
  }
  if (vector.expect_sealed) {
    const got = [...st.sealed].sort();
    const want = [...vector.expect_sealed].sort();
    if (JSON.stringify(got) !== JSON.stringify(want)) {
      throw new Error(`sealed: expected ${want}, got ${got}`);
    }
  }
}

void cloneState;
