// Independent HLC model per doc/KERNEL.md §5.
// Written against the contract prose only — NOT ported from zerodb-core/src/hlc.rs;
// divergence between the two is exactly what the vectors exist to catch.

const U16_MAX = 65535;

export class HlcModel {
  constructor({ resumeFrom = null, maxDriftMs = 60000 } = {}) {
    // restart transition (DQ-7): latest := persisted maximum, else zero state
    this.latest = resumeFrom
      ? { p: resumeFrom.physical_ms, l: resumeFrom.logical }
      : { p: 0, l: 0 };
    this.maxDriftMs = maxDriftMs;
  }

  #commit(p, l) {
    if (l > U16_MAX) {
      p += 1;
      l = 0;
    }
    this.latest = { p, l };
    return { physical_ms: p, logical: l };
  }

  local(wallMs) {
    const p = Math.max(wallMs, this.latest.p);
    const l = p === this.latest.p ? this.latest.l + 1 : 0;
    return this.#commit(p, l);
  }

  recv(wallMs, remote) {
    if (remote.physical_ms > wallMs + this.maxDriftMs) {
      return { error: 'DRIFT_EXCEEDED' };
    }
    const p = Math.max(wallMs, this.latest.p, remote.physical_ms);
    let base = -1;
    if (p === this.latest.p) base = Math.max(base, this.latest.l);
    if (p === remote.physical_ms) base = Math.max(base, remote.logical);
    const l = base === -1 ? 0 : base + 1;
    return this.#commit(p, l);
  }
}

export function runHlcVector(vector) {
  const hlc = new HlcModel({
    resumeFrom: vector.resume_from ?? null,
    maxDriftMs: vector.max_drift_ms ?? 60000,
  });
  const wall = vector.wall_ms;

  vector.steps.forEach((step, i) => {
    const result =
      step.action === 'local' ? hlc.local(wall)
      : step.action === 'recv' ? hlc.recv(wall, step.remote)
      : (() => { throw new Error(`step ${i}: unknown action ${step.action}`); })();

    if (step.expect_error) {
      if (result.error !== step.expect_error) {
        throw new Error(`step ${i}: expected ${step.expect_error}, got ${JSON.stringify(result)}`);
      }
      return;
    }
    if (result.error) {
      throw new Error(`step ${i}: unexpected error ${result.error}`);
    }
    if (result.physical_ms !== step.expect.physical_ms || result.logical !== step.expect.logical) {
      throw new Error(
        `step ${i}: expected (${step.expect.physical_ms}, ${step.expect.logical}), ` +
        `got (${result.physical_ms}, ${result.logical})`
      );
    }
  });
}
