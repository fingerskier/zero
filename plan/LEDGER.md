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
| P0-5 | `conformance/` + CI green baseline | fingerskier | done(`6500952` + required/xfail lanes) | P0-1 | S |
| P0-6 | SPEC two-layer gate language | fingerskier | done(`24b23c5`) | — | S |
| P0-7 | D-01..D-16 hygiene pass | fingerskier | done(`1db205e`) | — | S |

## Decisions (gate-shaping)

DQ-1..DQ-12 are defined in [PLAN.md §6](PLAN.md). Track resolution here; closure requires the SPEC §10 approved-resolution checklist.

| ID | Status | Blocks |
|----|--------|--------|
| DQ-1 identity model | **resolved 2026-07-18** (M0d exit; AUTH §1 + AUTH-CERT vectors) | — |
| DQ-2 datastore genesis/root authority | **resolved 2026-07-18** (M0d exit; AUTH §2 + AUTH-GEN vectors) | — |
| DQ-3 per-op membership verification + historical auth | **resolved 2026-07-18** (M0d exit; AUTH §4 + AUTH-AUTHZ/ADM vectors) | — |
| DQ-4 C8 executable model w/o SQLite | **resolved 2026-07-18** (M0e.1; WAL.md + WAL vectors) | — |
| DQ-5 encryption envelope scope + frozen bytes | **resolved 2026-07-16** (C1 closure; KERNEL §7 + ENV vectors); schema `encrypted` annotation constraints closed with M0b (SCHEMA §2 + SCHEMA-NEG-002) | — |
| DQ-6 extension/blob strategy | **resolved 2026-07-16** (C1 closure; KERNEL §8 + CRDT-BLOB-001) | — |
| DQ-7 durable HLC state rule | **resolved 2026-07-16** (contract; KERNEL §5 + HLC-002/005); backend layer re-verified at M1 | M1 layer 2 |
| DQ-8 equal-timestamp equivocation/tie-break | **resolved 2026-07-16** (C1 closure; KERNEL §4.5 + CRDT-LWW-002) | — |
| DQ-9 L2 catch-up mandatory for v0.1 | plan default: **yes** | M3a |
| DQ-10 `unique` removed from v0.1 profile | plan default: **removed** | M0b |
| DQ-11 resolution approver + records location | plan default: **this ledger** | — |
| DQ-12 capacity/effort ratification | open | schedule credibility |

## M0 — executable contracts

| ID | Work | Status | Depends | Effort | Entry gate | Exit evidence required | Top risk |
|----|------|--------|---------|--------|------------|------------------------|----------|
| VR | Version registry (5 namespaces) | done(`conformance/registry.json` + KERNEL §1) | P0-5 | S | P0 done | registry file + fixtures | naming churn |
| M0a | Semantic & operation kernel | **done(2026-07-16)** — C1 + C4-context resolved via SPEC §10 checklist; 24-vector corpus CI-blocking in both runners; draft-1 profile (byte freeze at composite M0). Evidence in resolved records below | VR, DQ-5..8 | L | registry merged | model suites green Rust+TS; golden vectors ✓ | corpus is a growing baseline, not exhaustive |
| M0b | Schema IR / epochs / query profile | **done(2026-07-18)** — C2 + O2/O3 resolved via SPEC §10 checklist; SCHEMA.md draft-1; 57-vector corpus (5 families) CI-blocking both runners. Evidence in resolved records below. TS→IR compiler trails ≤ M1 (not a gate); cross-peer migration shipping M4 | M0a ✓, DQ-10 | M | M0a vectors ✓ | epoch replay vectors incl. type change | — |
| M0c | Merkle / sync state machine | **done(2026-07-18)** — C3 resolved via SPEC §10 checklist; MERKLE.md draft-1; MERKLE-001..004 roots + MERKLE-T-001..004 transcripts CI-blocking both runners; wire framing M3. Evidence in resolved records below | M0a ✓ | M | M0a vectors | root vectors + mismatch transcript | — |
| M0d | Datastore / identity / authorization | **done(2026-07-18)** — C4 admission + C5 resolved via SPEC §10 checklist; AUTH.md draft-1; 18 auth vectors (+ full corpus 75) CI-blocking both runners; DQ-1/2/3 contract layer closed. On-wire enforcement M3b. Evidence in resolved records below | M0a ✓, DQ-1..3 | L | M0a vectors | negative auth vectors; control-plane spec | — |
| M0e.1 | Group/WAL reference model | **done(2026-07-18)** — C8; WAL.md draft-1; WAL-001..012 | M0a ✓, DQ-4 | M | M0a vectors | crash-point transcripts green (model) | — |
| M0e.2 | Delivery/ack/resume state machine | **done(2026-07-18)** — H4/H11 contract; DELIVERY.md; DELIV-001..004 | M0a ✓ | M | M0a vectors | loss/reorder/resume model suite | — |
| M0e.3 | CBOR decode profile + limits + registry | **done(2026-07-18)** — VERSIONS.md + registry + OP-NEG (M0a) | VR ✓ | S | registry merged | negative decode fixtures | — |
| M0f | Frontiers / checkpoints / snapshots | **done(2026-07-18)** — C7/O7 contract; FRONTIER.md; FRONT-001..003; GC disabled | M0c ✓, M0e.2 ✓ | M | both green | retirement/late-op/root-comparison models | — |
| M0 | Composite gate | **done(2026-07-18)** — C1–C5,C7–C8 resolved; COMP-001 smoke; 103 vectors both runners; draft-1 only (no format freeze) | M0a–M0f | — | all packages | cross-package fixtures green both runners | — |

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
| C1 (+C4 context, O6 provisional) | 2026-07-16 | M0a package exit: operation algebra/encoding/preimages normative in doc/KERNEL.md; draft-1 profile, byte freeze deferred to composite M0 | Contract commits `da48531`..exit commit; `conformance/registry.json`; 24 vectors in `conformance/vectors/required/` green in Rust harnesses + JS runner (CI blocking); DQ-5..8 directions ratified 2026-07-16 | fingerskier ("go ahead with the exit pass") |
| DQ-5..DQ-8 | 2026-07-16 | Contract layer resolved inside C1 closure (KERNEL §7, §8, §5, §4.5 respectively); DQ-7 backend durability layer re-verified at M1 | Same vector corpus (ENV-001/002, CRDT-BLOB-001, HLC-002/005, CRDT-LWW-002) | fingerskier |
| C2 (+O2/O3 residual) | 2026-07-18 | M0b package exit: schema IR / epochs / migration DSL / v0.1 query normative in doc/SCHEMA.md; draft-1 profile, byte freeze deferred to composite M0; TS→IR compiler ≤ M1; cross-peer migration shipping M4 | Contract commits `8cf044f`..`5c4ccdb` + exit pass; 57 vectors in `conformance/vectors/required/` (schema×6, epoch×14, migration×5, query×8 + M0a families) green in Rust harnesses + JS runner (CI blocking); O2/O3 Decision Log 2026-07-16 | fingerskier ("please continue with the plan") |
| C4 admission (+C5, DQ-1..3) | 2026-07-18 | M0d package exit: identity/genesis/membership/authz normative in doc/AUTH.md; draft-1 profile; on-wire enforcement deferred to M3b; closes CX-02 | AUTH.md + `zerodb-core/src/auth.rs` + JS model; 18 vectors under `conformance/vectors/required/auth/` (CERT×5, GEN×3, AUTHZ×6, ADM×4) green both runners; full required corpus 75 | fingerskier ("please continue") |
| C3 | 2026-07-18 | M0c package exit: canonical Merkle tree + mismatch-recovery walk normative in doc/MERKLE.md; draft-1; wire framing M3 | MERKLE.md + `merkle.rs`/`merkle.mjs`; MERKLE-001..004 + MERKLE-T-001..004 green both runners (corpus 95 at exit) | fingerskier ("please continue") |
| C8 (+DQ-4) | 2026-07-18 | M0e.1: WAL/group reference model | WAL.md; WAL-001..012 | fingerskier |
| H4/H11 | 2026-07-18 | M0e.2: delivery/ack/resume contract | DELIVERY.md; DELIV-001..004 | fingerskier |
| H7 (+H9 registry) | 2026-07-18 | M0e.3: version policy + limits | VERSIONS.md; registry | fingerskier |
| C7 (+O7) | 2026-07-18 | M0f: frontiers/snapshots; GC disabled | FRONTIER.md; FRONT-001..003 | fingerskier |
| Composite M0 | 2026-07-18 | All M0 packages closed at model layer; no format freeze | 103 required vectors both runners; COMP-001 | fingerskier ("lets finish M0, then commit") |
