# ZeroDB — Delivery Ledger

Canonical work tracker. A gate closes only with **Exit evidence** (commit, fixture path, or CI run).

**DRI:** `fingerskier` unless a row names someone else. Effort bands (rough solo): S ≤ 1 wk, M ≤ 1 mo, L > 1 mo. DQ-12 capacity ratification remains open.

Status: `open` · `in-progress` · `blocked(<on>)` · `done(<evidence>)`.

---

## Closed (index only)

| Gate | Closed | Where remembered |
|------|--------|------------------|
| P0 readiness (P0-1..P0-7) | 2026-07-16 | ISSUES Decision Log; conformance + INVARIANTS + EXEMPLAR in `doc/` |
| Composite M0a–M0f (contract-model) | 2026-07-18 | [SPEC §10](../doc/SPEC.md), package docs (KERNEL…FRONTIER), 103 vectors |
| DQ-1..DQ-8, DQ-10 | 2026-07-16/18 | AUTH / KERNEL / SCHEMA / WAL + Decision Log |
| Historical plan reviews (Codex 07-16, Grok plan 07-15) | dispositioned | Decision Log; work executed — review files removed from `plan/` |

Detailed resolved-issue audit prose lives in the [ISSUES Decision Log](../doc/ISSUES.md) only (no second copy here).

---

## Decisions still open

| ID | Status | Blocks |
|----|--------|--------|
| DQ-9 L2 catch-up mandatory for v0.1 | plan default **yes** (SPEC §10) | M3a |
| DQ-11 approver + records | plan default: this ledger + Decision Log | — |
| DQ-12 capacity/effort bands | **open** | schedule credibility |

---

## M1 — Local durable core (`v0.1.0-local`)

**Overall:** `in-progress` — experimental slice [M1-LOCAL.md](../doc/M1-LOCAL.md) (`1ef16d6`, `f9cc660`); **exit not closed**.

Depends: composite M0 model (done). Release: `v0.1.0-local`.

### Implemented (non-exit)

| ID | Work | Status | Evidence |
|----|------|--------|----------|
| M1-proto-store | SQLite LocalStore, signed ops, 5 property CRDTs, single-op txn | done (experimental) | `zerodb-storage`; [M1-LOCAL.md](../doc/M1-LOCAL.md) |
| M1-proto-cli | init/CRUD/inspect/replay/export/import/sync | done (experimental) | `zerodb-cli` |
| M1-proto-tcp | serve/pull one-way set-diff; LAN runbook | done (experimental) | CLI; [M1-LAN-TEST.md](../doc/M1-LAN-TEST.md); `scripts/test-mvp.ps1` |
| M1-proto-ingress | full-bundle prevalidation, atomic adopt, shadow props, drift bound | done (experimental) | `f9cc660`; storage tests arrival/ingress |

### Exit / correctness backlog

| ID | Work | Status | Effort | Exit evidence required | Source |
|----|------|--------|--------|------------------------|--------|
| M1-fix-rng | OS CSPRNG for seed/salt (`getrandom`) | done(`m1_wave1` seed test) | S | seed fill uses OS RNG; unit test | G-01 |
| M1-fix-hlc | HLC on open/replay from durable oplog (DQ-7 backend) | done(`m1_wave1` HLC tests) | S | open/replay rewrite meta from oplog max | G-02 |
| M1-fix-serve | Fail-closed serve defaults (loopback / unsafe flag) | done(`serve_bind` tests) | S | refuse non-loopback without `--allow-insecure-lan` | G-05 |
| M1-e1 | Full E1 (restart, replay, larger load, HLC mono) | done(`e1_e2_acceptance` fifty-todo + HLC mono) | M | 50-todo restart/import/replay; post-restart ts > pre max | G-09 |
| M1-e2-store | Store-level equal-ts / equivocation suite | done(`e1_e2_acceptance` e2_*) | S | same-author exclude + cross-peer total order | G-09 |
| M1-e4 | Groups + WAL layer-2 crash injection | in-progress | L | `atomic_group` + grp export done (`e4_groups`); named WAL crash injection still open | G-03 |
| M1-e9 | H3 derived visibility + edges + late-edge E9 | open | L | H3 resolution + E9 fixtures | G-04 |
| M1-schema | Schema apply / IR load; type pin; `ep` | open | M | schema apply path; type-mix policy | G-06/08 |
| M1-query | O3 minimal query + repl | open | M | CLI query greening SCHEMA vectors | G-08 |
| M1-tsir | TS→IR compiler (≤ M1, O2) | open | M | npm tool emits SCHEMA §2 IR | G-14 |
| M1-fmtver | `storage_format_version` meta + freeze discipline | done(meta v1 + backfill; freeze still open) | S | meta written at init/open; freeze still needs Decision Log | G-07 |
| M1-exit | Close M1 when exit criteria met | blocked(above) | — | SPEC §10 M1 checklist + E1/E2/E4/E9 | — |

---

## Post-M1

| ID | Work | Status | Depends | Effort | Release |
|----|------|--------|---------|--------|---------|
| M2 | Node/NAPI SDK vertical (E11 provisional) | in-progress | M1 (rides experimental LocalStore) | M | `v0.1.0-sdk` |

### M2 subtasks

| ID | Work | Status | Evidence |
|----|------|--------|----------|
| M2-napi-scaffold | `zerodb-napi` crate + `@zerodb/node` package | done | `zerodb-napi/`; `napi build` |
| M2-napi-crud | Database init/open/mutate/get/inspect/export/import/close | done | `test/m2-basic.test.mjs` (3) |
| M2-subscribe | `subscribe` / live change notifications | open | — |
| M2-query | O3 query via NAPI | open | blocked(M1-query) |
| M2-schema | schema apply / TS→IR pipeline (O2) | open | blocked(M1-schema/tsir) |
| M2-crdts | MVRegister + resolve, RGA, LWWMap | open | kernel + binding |
| M2-parity | binding parity vectors vs core fixtures | open | — |
| M2-exit | Close `v0.1.0-sdk` | blocked(above) | SPEC §10 M2 checklist |
| M3a | L2 relay + offline catch-up (E3) | open | M2, M0c/M0f | L | internal |
| M3b | Security: auth, envelope, negatives (E5–E8) | open | M3a, M0d | L | internal |
| M3c | Interop TS wire peer + release | open | M3b | M | `v0.1.0` |
| M4a | Browser/WASM/WebRTC/React | open | M3c | L | feature |
| M4b | Migration/snapshots/upgrade (E10) | open | M3c | L | feature |
| M5a | Operability (backup/restore, SLOs) | open | M3c | M | GA program |
| M5b | Lifecycle safety (GC, rolling upgrade) | open | M4b, C7 | L | GA program |
| M5c | Release assurance (fuzz/soak/audit) | open | M5a, M5b | L | GA decision |
| M6 | Ecosystem (per-epic) | open | compat stability | — | epics |
