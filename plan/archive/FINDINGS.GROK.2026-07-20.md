# ZeroDB open findings — Grok (2026-07-20)

**Baseline:** `d8af390` (local `main`; one commit ahead of `origin/main` @ `3b269e8`)  
**Role:** Grok review of **project state + execution plan** — open issues, gaps, discrepancies, and obvious improvements. Work tracking remains [LEDGER.md](LEDGER.md). Normative roadmap: [SPEC §10](../doc/SPEC.md).  
**Related:** [FINDINGS.CODEX.md](FINDINGS.CODEX.md) (same-day deep code/contract review at `3b269e8`) — this file does not re-litigate every CX/HX ID; it focuses on **plan/status truthfulness**, gate readiness, and tracking hygiene. Where Grok independently re-checked code, agreement with Codex is noted.

**Verdict:** Composite M0 contract-model + experimental M1/M2 scaffolds are real progress. Tracker language has **advanced faster than evidence**. Formal M1 exit and format freeze should stay **blocked**. Prefer a short **stabilization / evidence-repair** pass before any exit ceremony.

---

## 1. Executive scorecard

| Area | Assessment | Why |
|------|------------|-----|
| Architecture / sequencing | **Sound** | M0 → M1 → M2 → M3a/b/c still correct |
| Composite M0 (as “done”) | **Over-closed for freeze** | Draft-1 models + 103 vectors exist; several contracts remain non-implementable or unfalsified (envelope AAD cycle, frontier late-op, resume cursor, WAL MUST gaps) — see Codex CX-03..06 |
| M1 implementation | **Substantial prototype** | SQLite store, signed ops, edges, groups slice, schema pin, query CLI, TCP LAN path |
| M1 exit readiness | **Not ready** | SPEC M1 checkboxes still open; confirmed SEC + destructive-init defects; E1/E4 evidence thinner than claimed |
| M2 | **Scaffold only** | NAPI CRUD smoke; no subscribe/query/parity/clean-CI matrix |
| M3 readiness | **Not implementation-ready** | RELAY still pre-M0 placeholders; accepted-set/Merkle divergence open |
| Plan / LEDGER hygiene | **Drifting** | `done` rows and “largely met” language conflict with code, M1-LOCAL, SPEC, and this backlog |
| This file (prior revision) | **Was stale** | Prior pin `f9cc660` still called E4/E9/schema/query “missing” after they landed as prototypes |

---

## 2. What the prior Grok backlog got wrong (self-audit)

The 2026-07-19 `FINDINGS.GROK` index mapped G-01..G-14 → LEDGER and left many rows “open/missing.” **Code and LEDGER moved on**; the index did not.

| Prior ID | Prior one-liner | Current reality (2026-07-20) | Disposition |
|----------|-----------------|------------------------------|-------------|
| G-01 | UUID-tiled seed | OS CSPRNG | **done** (keep closed) |
| G-02 | meta-only HLC | oplog max on open/replay | **done** as backend high-water; own-device vs all-authors still a **design tension** (KERNEL vs store) |
| G-03 | No groups / crash matrix | `atomic_group` + mid-batch rollback tests | **partial** — transactional rollback ≠ WAL named crash-injection / kill matrix |
| G-04 | Node tombstone only; no edges | edges + derived hide exist | **partial** — arrival-order tombstone-before-create still SEC-broken; no edge tombstone/props |
| G-05 | Plaintext serve fail-closed | loopback / flag | **done** (still plaintext by design) |
| G-06 | No CRDT type pin | local JSON pin | **partial** — pin is local JSON metadata, not canonical Schema IR / remote import pin / `ep` |
| G-07 | format version / freeze | meta v1 written; freeze open | **partial** (unchanged intent) |
| G-08 | CLI schema/query missing | `schema-apply` + `query` exist | **partial** — no interactive `repl`; not full M0b |
| G-09 | E1/E2 store harness | `e1_e2_acceptance` present | **partial** — happy-path restart/import; not kill-not-shutdown / 1h clock rollback |
| G-10 | Dead `HelloOk.need` | documented | **deferred** (experimental TCP) |
| G-11 | Signing seed in SQLite | documented threat | **deferred** (M1 disposable) |
| G-12 | Rematerialize full scan | still true | **defer** until scale pressure |
| G-13 | Local genesis ≠ AUTH | documented | **deferred** → M3b |
| G-14 | TS→IR absent | `tools/ts-to-ir` minimal JSON→pin | **partial** — not SCHEMA CBOR / SchemaId |

**Lesson:** this file must be refreshed when LEDGER rows flip, or demoted to archive. A stale “open backlog” actively misleads.

---

## 3. Critical / high issues (plan-relevant)

### G20-01 — LEDGER/PLAN overstate M1 exit readiness
**Severity:** high (process / gate integrity)

- [PLAN.md](PLAN.md) §5: “M1 remainders largely implemented… Formal Decision Log exit… when ready.”
- [LEDGER.md](LEDGER.md): M1-e4, M1-e9, M1-schema, M1-query, M1-tsir all `done(...)`; M1-exit says checklist “largely met.”
- [README.md](../README.md): “M1 experimental local core substantially complete.”
- [SPEC.md](../SPEC.md) §10 M1: **all exit checkboxes still unchecked**; requires crash atomicity at every commit boundary, H3 delete machine, canonical schema IR, `repl`, etc.

Happy-path green tests are not the same as EXEMPLAR/SPEC acceptance. **Correction:** use `partial` / `prototype` / `blocked(...)` until evidence matches the normative row text. Treat formal exit as a separate LEDGER row with a mechanical checklist (commit + test path per SPEC bullet).

### G20-02 — Confirmed SEC defect: tombstone-before-create is arrival-order dependent
**Severity:** bug (blocks honest E9 / I-1 / I-16)

`apply_wire` applies tombstone as `UPDATE nodes SET deleted=1` (no-op if missing); later `CreateNode` inserts `deleted=0` ([`zerodb-storage/src/lib.rs`](../zerodb-storage/src/lib.rs) ~1227–1259). Same op set, opposite order → live vs deleted. Codex CX-01; independent code read confirms. Property-before-create is tested; **create/tombstone permutation is not**.

**Correction:** set-derived materialization for entity existence/tombstone; red permutation + replay identity tests before any E9 `done`.

### G20-03 — `init` can silently re-key a nonempty DB
**Severity:** bug (destructive CLI)

`LocalStore::init` always writes new seed/ds/salt/HLC meta with no initialized-or-nonempty guard ([`zerodb-storage/src/lib.rs`](../zerodb-storage/src/lib.rs) ~160–185). Codex CX-02; code still matches. Risk: retained ops under old `ds` + new top-level identity → broken export/import.

**Correction:** fail closed by default; explicit destructive reset command/flag + tests.

### G20-04 — “Composite M0 done” is not freeze-ready (and freeze language conflicts)
**Severity:** high (contract / docs)

Independent agreement with Codex CX-03..06 on implementability gaps:

| Contract | Gap (short) |
|----------|-------------|
| M0a envelope | AAD binds final `OpId` that hashes ciphertext → construction cycle |
| M0f frontier | tip is OpId-only; `is_late_op` is a proxy; snapshot artifact underspecified |
| M0e delivery | `resume` is in-process set-diff, not cursor across independent peers |
| M0e WAL | model does not enforce several WAL MUST rules; recovery visibility for unsealed groups weak |

Meanwhile freeze messaging disagrees across surfaces:

- Registry / package headers: freeze “at composite M0”
- PLAN / ISSUES: freeze only via explicit Decision Log
- README: formal M1 exit **+ freeze** still open
- PLAN §5: freeze **optional** after M1

**Correction:** single official state: **`draft-1, unfrozen`**. Freeze is a separate Decision Log act, never implied by “M0 closed” or “M1 exit.” Reopen or amend deficient models with vectors before any freeze discussion.

### G20-05 — E1 / E4 evidence ≠ named acceptance scenarios
**Severity:** high (evidence)

- E1: store tests do clean drop/reopen/import — not kill-not-shutdown, not 1-hour clock rollback ([EXEMPLAR](../doc/EXEMPLAR.md)).
- E4: atomic SQLite txn rollback + group id on export — not process death at every WAL append/sync/apply/HLC/seal boundary. M1-LOCAL still calls fine-grained crash points “optional”; SPEC does not.

**Correction:** map each SPEC/EXEMPLAR requirement to a failing-then-passing test; do not close LEDGER rows on surrogate happy paths.

### G20-06 — Schema / TS→IR / query “done” without normative layer
**Severity:** high (scope honesty)

LEDGER marks M1-schema / M1-tsir / M1-query done while admitting CBOR IR, SchemaId, and interactive repl are later/absent. Runtime: `ep = 0`, empty deps, JSON pin in meta, no secondary indexes, parameters always empty on query path (Codex CX-07 / HX-08). SPEC still requires canonical IR, strict+schemaless, indexes, `repl`.

**Correction:** either reopen those rows as `partial`, or amend SPEC M1 scope via Decision Log **before** claiming exit.

### G20-07 — M2 tracker contradicts itself
**Severity:** suggestion → process

M2 is `in-progress` depending on M1; M2-query/schema are `blocked(M1-query)` / `blocked(M1-schema/tsir)` while those M1 rows are `done`. CI does not build NAPI or `tools/ts-to-ir` on clean checkout; package is Windows-only; public surface is sync CRUD smoke.

**Correction:** split statuses:

1. `prototype-on-experimental-M1` (allowed now),
2. `M2-exit depends on formal M1 surface`,
3. true technical blockers.

### G20-08 — M3 must not start on the current relay draft
**Severity:** high (sequencing)

RELAY still carries pre-M0 placeholders; peer accepted-set vs relay-validated Merkle roots diverge under unauthorized ops (Codex CX-08). No R0-style “relay contract ready” gate in PLAN.

---

## 4. Documentation / tracker discrepancies

| ID | Discrepancy | Fix |
|----|-------------|-----|
| D-G01 | **This file** was pinned to `f9cc660` and listed landed work as missing | Refresh on each major LEDGER flip (this revision) |
| D-G02 | LEDGER “Closed” claims historical review files **removed** from `plan/`; `FINDINGS.GROK.md` + new `FINDINGS.CODEX.md` remain | Fix closed-index wording; define archive policy (`archive/` or “open backlog only”) |
| D-G03 | [M1-LOCAL.md](../doc/M1-LOCAL.md) still says edges unimplemented, `grp` always absent, schema/query out of scope — code has edges, groups, schema-apply, query | Refresh descriptive doc every implementation milestone |
| D-G04 | M1-LOCAL marks fine-grained WAL crash points **optional**; SPEC requires every commit boundary | SPEC wins; drop “optional” |
| D-G05 | SPEC M1 checkboxes all open; README/PLAN/LEDGER imply near-exit | Either tick with evidence links or keep narrative “experimental / exit open” only |
| D-G06 | Registry: `storage_format_version` status “pending M1” while store writes v1 and backfills | Align registry with experimental-vs-frozen |
| D-G07 | Registry / package docs say freeze at composite M0; PLAN/ISSUES require Decision Log freeze | One freeze story everywhere |
| D-G08 | README lists FINDINGS.GROK as “open review backlog” while Codex is the denser current review | Index both; or single FINDINGS index + dated attachments |
| D-G09 | M2 blocked-on-done M1 rows | See G20-07 |
| D-G10 | CI: no clippy, no NAPI clean build, no ts-to-ir, no `cargo test --locked`, Rust xfail may fail required job | Gate completeness before “scaffold done” language |

---

## 5. Open items → LEDGER (refreshed index)

Use this as the live Grok → LEDGER map. Prefer **partial** over false `done`.

| ID | Severity | One-liner | LEDGER / action |
|----|----------|-----------|-----------------|
| G-01 | — | OS CSPRNG seed | **closed** M1-fix-rng |
| G-02 | — | HLC from oplog on open/replay | **closed** M1-fix-hlc (revisit own-device filter) |
| G-05 | — | serve fail-closed | **closed** M1-fix-serve |
| G20-02 | bug | Tombstone-before-create SEC | **reopen/partial** M1-e9; new red tests |
| G20-03 | bug | `init` re-keys nonempty DB | **new** M1-fix-init (fail closed) |
| G20-05a | bug | E1 kill + clock-rollback missing | **partial** M1-e1 |
| G20-05b | bug | E4 WAL crash matrix missing | **partial** M1-e4 |
| G20-06 | bug | Canonical schema / deps / ep / indexes | **partial** M1-schema (not exit-complete) |
| G20-06b | suggestion | TS→IR is JSON pin tool | rename or complete M1-tsir |
| G20-06c | suggestion | No interactive repl | **partial** M1-query or SPEC defer |
| G-07 | suggestion | Freeze still open | M1-fmtver + Decision Log only |
| G-10 | suggestion | Dead `HelloOk.need` | experimental; document only |
| G-11 | suggestion | Plaintext seed in SQLite | threat note OK for disposable M1 |
| G-12 | suggestion | Full rematerialize scans | defer |
| G-13 | suggestion | Local ≠ AUTH genesis | M3b |
| G20-04 | bug | M0 contract implementability | Decision Log targeted reopen / amend (not freeze) |
| G20-07 | process | M2 status model | LEDGER wording only until M1 surface stable |
| G20-08 | process | Relay accepted-set | blocks M3a implementation |

**Intentionally out of M1 product scope (still experimental):** plaintext TCP multi-machine; AUTH membership on ingest; any claimed format freeze of JSON bundle / Hello / SQLite layout.

---

## 6. M1 exit gap (honest summary)

| Criterion | Status |
|-----------|--------|
| Local SQLite + signed property CRDTs | **met** (experimental path) |
| Edges + derived delete visibility | **partial** (SEC hole on tombstone order) |
| Ingress / arrival-order (properties) | **met** for property-before-create |
| Ingress / arrival-order (entity delete) | **fail** |
| Safe `init` | **fail** |
| E1 full EXEMPLAR | **partial** |
| E2 store equal-ts | **partial / good direction** |
| E4 groups + crash injection | **partial** (txn rollback only) |
| E9 / H3 complete | **partial** |
| Canonical schema + query + repl + TS→IR | **partial / mislabeled** |
| DQ-7 backend HLC | **met** for high-water (design edge cases remain) |
| Format freeze | **open** (must stay open) |
| LEDGER M1-exit | **blocked** on above + Decision Log |

---

## 7. Recommended plan adjustments

1. **Add R0 (or “M1-stabilize”)** before formal M1 exit / freeze — fail-closed init, set-derived entity state, E1/E4 evidence upgrade, freeze-wording cleanup. (Aligns with Codex §7 R0; Grok agrees this is the highest-leverage sequencing fix.)
2. **Demote over-closed LEDGER rows** to `partial` with explicit remaining evidence lists; never close on “first slice.”
3. **Refresh M1-LOCAL** in the same commit as behavior changes so reviewers stop treating it as authority when stale.
4. **Freeze policy one-liner** in README + PLAN + registry + KERNEL header: *draft-1 unfrozen until Decision Log freeze names a profile.*
5. **M2 parallel spike OK**; M2-exit and public API commitments blocked on stabilized M1 surface + clean multi-platform CI.
6. **M3a** starts only after relay accepted-set + durable-ack + two-language wire transcripts exist.
7. **Archive policy:** either move dated FINDINGS to `plan/archive/` after disposition, or keep one `FINDINGS.md` index pointing at dated reviews so LEDGER never claims “removed” when files remain.

### Immediate decision queue (short)

1. Confirm freeze state: `draft-1, unfrozen`.
2. Entity delete/create CRDT (tombstone-before-create legal? recreate same id?).
3. Narrow SPEC M1 (schema/repl/indexes) **or** implement before exit.
4. HLC recovery: own-ops only vs receive-clock durable state.
5. Encrypted AAD preimage (non-circular).
6. Whether R0 is explicit LEDGER gate or folded into reopened M1 rows.
7. DQ-12 capacity bands (still open; schedule still unratified).

---

## 8. Verification (this review)

| Check | Result |
|-------|--------|
| Baseline | `d8af390`; tree clean at review start except this file write |
| Required vectors on disk | **103** under `conformance/vectors/required` |
| Code read | `init`, `apply_wire` create/tombstone/edge, envelope AAD, frontier `is_late_op`, delivery resume, WAL model |
| `cargo test -p zerodb-storage --test arrival_order` | **4/4 pass** (property-before-create only — does not cover tombstone order) |
| Cross-doc | PLAN, LEDGER, README, M1-LOCAL, SPEC §10 M1, ISSUES freeze note, registry, CI workflow, FINDINGS.CODEX |

Not re-run this session: full workspace tests, JS conformance runner, NAPI, clippy (Codex recorded pass/fail at `3b269e8`; treat as prior evidence).

---

## 9. Conclusion

ZeroDB’s direction and experimental substrate are worth preserving. The main failure mode is **status inflation**: LEDGER `done` and plan “exit pass next” language outrun SPEC checkboxes, EXEMPLAR fault schedules, and at least two confirmed storage defects.

Highest-leverage next move: **stabilize state and evidence** (G20-02, G20-03, E1/E4 honesty, freeze wording), then complete M1 against a mechanical checklist. Do not reward incomplete evidence with format freeze or an early `v0.1.0-local` tag.

---

*Full narrative code/contract review with CX/HX IDs and line cites: [FINDINGS.CODEX.md](FINDINGS.CODEX.md). This Grok file is the plan/status + open-backlog companion.*
