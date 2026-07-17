# ZeroDB — Delivery Ledger

Canonical tracking table for all planned work (PLAN P0-4). The critical path derives from this file, not from document order. Update the row when status changes; a gate closes only with an **Exit evidence** link (commit, fixture path, or CI run).

**DRI:** single-maintainer project — DRI is `fingerskier` unless a row names someone else; the column exists so external contributions get owners (DQ-12 capacity/effort bands remain to be ratified — effort values below are rough solo-work bands: S ≤ 1 wk, M ≤ 1 mo, L > 1 mo).

Status values: `open`, `in-progress`, `blocked(<on>)`, `done(<evidence>)`.

## P0 — readiness

| ID | Work | DRI | Status | Depends | Effort |
|----|------|-----|--------|---------|--------|
| P0-1 | Green baseline (build, A-02, fmt, tests) | fingerskier | done(`a4266fa`) | — | S |
| P0-2 | INVARIANTS.md I-1..I-17 | fingerskier | done(`24b23c5`) | — | S |
| P0-3 | EXEMPLAR scenarios E1–E11 | fingerskier | done(`24b23c5`) | — | S |
| P0-4 | This ledger | fingerskier | done(this file) | — | S |
| P0-5 | `conformance/` + CI green baseline | fingerskier | open | P0-1 | S |
| P0-6 | SPEC two-layer gate language | fingerskier | done(`24b23c5`) | — | S |
| P0-7 | D-01..D-16 hygiene pass | fingerskier | done(`1db205e`) | — | S |

## Decisions (gate-shaping)

DQ-1..DQ-12 are defined in [PLAN.md §6](PLAN.md). Track resolution here; closure requires the SPEC §10 approved-resolution checklist.

| ID | Status | Blocks |
|----|--------|--------|
| DQ-1 identity model | proposal drafted ([DQ-PROPOSALS](DQ-PROPOSALS.md)) | M0d |
| DQ-2 datastore genesis/root authority | proposal drafted ([DQ-PROPOSALS](DQ-PROPOSALS.md)) | M0d |
| DQ-3 per-op membership verification + historical auth | proposal drafted ([DQ-PROPOSALS](DQ-PROPOSALS.md)) | M0d |
| DQ-4 C8 executable model w/o SQLite | proposal drafted ([DQ-PROPOSALS](DQ-PROPOSALS.md)) | M0e.1 |
| DQ-5 encryption envelope scope + frozen bytes | proposal drafted ([DQ-PROPOSALS](DQ-PROPOSALS.md)) | M0a/M0b |
| DQ-6 extension/blob strategy | proposal drafted ([DQ-PROPOSALS](DQ-PROPOSALS.md)) | M0a |
| DQ-7 durable HLC state rule | proposal drafted ([DQ-PROPOSALS](DQ-PROPOSALS.md)) | M0a, M1 |
| DQ-8 equal-timestamp equivocation/tie-break | proposal drafted ([DQ-PROPOSALS](DQ-PROPOSALS.md)) | M0a |
| DQ-9 L2 catch-up mandatory for v0.1 | plan default: **yes** | M3a |
| DQ-10 `unique` removed from v0.1 profile | plan default: **removed** | M0b |
| DQ-11 resolution approver + records location | plan default: **this ledger** | — |
| DQ-12 capacity/effort ratification | open | schedule credibility |

## M0 — executable contracts

| ID | Work | Status | Depends | Effort | Entry gate | Exit evidence required | Top risk |
|----|------|--------|---------|--------|------------|------------------------|----------|
| VR | Version registry (5 namespaces) | open | P0-5 | S | P0 done | registry file + fixtures | naming churn |
| M0a | Semantic & operation kernel | open | VR, DQ-5..8 | L | registry merged | model suites green Rust+TS; golden vectors | kernel scope creep |
| M0b | Schema IR / epochs / query profile | open | M0a, DQ-10 | M | M0a vectors | epoch replay vectors incl. type change | migration DSL design |
| M0c | Merkle / sync state machine | open | M0a | M | M0a vectors | root vectors + mismatch transcript | canonical-tree edge cases |
| M0d | Datastore / identity / authorization | open | M0a, DQ-1..3 | L | M0a vectors | negative auth vectors; control-plane spec | identity-model rework |
| M0e.1 | Group/WAL reference model | open | M0a, DQ-4 | M | M0a vectors | crash-point transcripts green (model) | C8 model fidelity vs SQLite |
| M0e.2 | Delivery/ack/resume state machine | open | M0a | M | M0a vectors | loss/reorder/resume model suite | cursor semantics (HX-04) |
| M0e.3 | CBOR decode profile + limits + registry | open | VR | S | registry merged | negative decode fixtures | pre-auth resource limits |
| M0f | Frontiers / checkpoints / snapshots | open | M0c, M0e.2 | M | both green | retirement/late-op/root-comparison models | frontier compactness (O7) |
| M0 | Composite gate | open | M0a–M0f | — | all packages | cross-package fixtures green both runners | silent gate waivers |

## Post-M0

| ID | Work | Status | Depends | Effort | Release |
|----|------|--------|---------|--------|---------|
| M1 | Local durable core (SQLite+CLI, layer-2 crash tests, E1/E2/E4/E9) | open | M0a,M0b,M0e models | L | `v0.1.0-local` (MVP) |
| M2 | Node/NAPI SDK vertical (E11 provisional) | open | M1 | M | `v0.1.0-sdk` |
| M3a | Durable convergence: L2 relay, catch-up (E3) | open | M2, M0c/M0f | L | internal |
| M3b | Security: auth, envelope, negatives (E5–E8) | open | M3a, M0d | L | internal |
| M3c | Interop TS wire peer + release | open | M3b | M | `v0.1.0` |
| M4a | Browser/WASM/WebRTC/React | open | M3c | L | feature |
| M4b | Migration/snapshots/upgrade matrix (E10) | open | M3c | L | feature |
| M5a | Operability (backup/restore, SLOs) | open | M3c | M | GA program |
| M5b | Lifecycle safety (GC, rolling upgrade) | open | M4b, C7 | L | GA program |
| M5c | Release assurance (fuzz/soak/audit) | open | M5a, M5b | L | GA decision |
| M6 | Ecosystem portfolio (per-epic approval) | open | compat stability | — | epics |

## Resolved-issue records

Durable closure records (HX-10): issue ID, outcome, evidence, approver. Decision Log in [ISSUES.md](../doc/ISSUES.md) holds the one-line outcome; this table holds the audit trail.

| Issue | Resolved | Outcome | Evidence | Approver |
|-------|----------|---------|----------|----------|
| O5 | 2026-07-16 | won't do — clean break, no migration tooling | Decision Log entry; README expectation set (`1db205e`) | fingerskier |
