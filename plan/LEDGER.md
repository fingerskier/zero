# ZeroDB — Delivery Ledger

Canonical work tracker. A gate closes only with **Exit evidence** (commit, fixture path, or CI run).

**DRI:** `fingerskier` unless a row names someone else. Effort bands (rough solo): S ≤ 1 wk, M ≤ 1 mo, L > 1 mo. DQ-12 capacity ratification remains open.

Status: `open` · `in-progress` · `blocked(<on>)` · `done(<evidence>)` · `pinned`.

---

## Closed (index only)

| Gate | Closed | Where remembered |
|------|--------|------------------|
| P0 readiness (P0-1..P0-7) | 2026-07-16 | ISSUES Decision Log; conformance + INVARIANTS + EXEMPLAR in `doc/` |
| Composite M0a–M0f (contract-model) | 2026-07-18 | [SPEC §10](../doc/SPEC.md), package docs (KERNEL…FRONTIER); **109** vectors after 2026-08-14 CX-03..06 amend |
| DQ-1..DQ-8, DQ-10 | 2026-07-16/18 | AUTH / KERNEL / SCHEMA / WAL + Decision Log |
| Historical plan reviews (Codex 07-16, Grok plan 07-15) | dispositioned | Decision Log |
| July 2026 state reviews (FINDINGS.GROK / FINDINGS.CODEX) | archived 2026-08-14 | [plan/archive/](archive/) — CX-01/CX-02 closed in tree; do not treat as live backlog |
| M1 local durable core (`v0.1.0-local`) | 2026-07-25 | Decision Log; [M1-LOCAL.md](../doc/M1-LOCAL.md); tag `v0.1.0-local`. Suites: `e1_e2_acceptance`, `e1_kill_clock`, `e4_crash_matrix`, `e9_delete_machine`, `m1_wave1`, `r0_stabilize`, `serve_bind`. Freeze still open. |
| M2 Node/NAPI (`v0.1.0-sdk`) | 2026-08-14 | Decision Log; `zerodb-napi/`; `m2-*.test.mjs`; `applyCrdtVector`; CI [run 31836860347](https://github.com/fingerskier/zero/actions/runs/31836860347) @ `b352ca4`. M2-crdts deferred (app-trigger). Not SPEC-complete M2. |
| M2a stabilize + schema | 2026-08-14 | Decision Log; `r0_stabilize`, `m2_schema`, CX-03..06 (109 vectors), CX-08 RELAY 0.2.2-draft |
| M3a L2 relay + E3 | 2026-08-15 | Decision Log; `zerodb-relay`; `relay_client`; `m3a-relay.test.mjs`; `merkle-walk-v1`; `full_exemplar_e3_1000_ops_hard_crash_and_relay_only_catchup`; `relay-transcript` 6 |
| M3b-sig | 2026-08-16 | `admit_experimental_op` + `m3b_admission`. Not M3b exit. |
| M3b-auth-e5 | 2026-08-19 | `membership_grants` + peer AUTH §4 + `e5_membership` (storage + relay). Not M3b exit. |
| M3b-e7 | 2026-08-27 | `e7_forged_replay` (storage + relay). Not M3b exit. |
| M3b-e8 | 2026-08-27 | `e8_clock_quarantine` (storage + relay). H1 closed. Not M3b exit. |
| M3b-e6 | 2026-08-27 | `e6_encrypted_notes` (storage + relay). Not H10-complete / M3b exit. |
| perf-doc | 2026-08-28 | [PERF.md](PERF.md). Stage 0+1 **landed** `9903280`; Stage 2/3 and H10 leftovers pinned. Not a benchmark report. |
| perf-s0 | 2026-08-28 | `perf_s0` fixtures/phase counters (1k). No invented README numbers. |
| perf-s1 | 2026-08-28 | `import_replay_equiv` + `limits` + chunk tests. Advertised payload/batch; import≡replay. Not Stage 2/3. |
| M4a-browser-slice / wasm-events / push-driver / idb-journal | experimental shipped | `zerodb-wasm`; `memory_backend.rs`; `sync-driver.test.mjs`; `examples/browser-peer`. M4a proper still waits on M3c. |

Detailed resolved-issue audit prose lives in the [ISSUES Decision Log](../doc/ISSUES.md) only (no second copy here).

---

## Decisions still open

| ID | Status | Blocks |
|----|--------|--------|
| DQ-9 L2 catch-up mandatory for v0.1 | **ratified** (M3a; default yes) | — |
| DQ-11 approver + records | plan default: this ledger + Decision Log | — |
| DQ-12 capacity/effort bands | **open** | schedule credibility |

---

## Live work

### M3c — Interop TS wire peer + release (`v0.1.0`)

Depends: M3a done; M3b remainder pinned (not a start-blocker). Release: `v0.1.0`.

| ID | Work | Status | Notes |
|----|------|--------|-------|
| M3c | Interop TS wire peer + release (include signed `SchemaEpoch`) | open | SPEC M3c exit + Decision Log act at tag time. **Not** claimed until that act. |
| M3c-epoch | Signed KERNEL kind 5 `SchemaEpoch` | in-progress | Kind 5 persist/ingest/import landed this PR (`m3c_schema_epoch`; n=1, empty migration). Schema including `encrypted: true` is an op. Unknown `ep` is `EPOCH_UNKNOWN`. Codex P1s: epoch-first in multi-op batches; own-epoch IR for pin/encrypt (ep=0 schemaless). Fork/quarantine + non-empty migration not this slice. Do not freeze wrap-body. **Not** M3c complete. |
| M3c-ts-peer | Independent TypeScript wire peer | in-progress | This PR: `conformance/ts/peer/` evolved from the runner, **not** NAPI-backed (SPEC M3c). HELLO/`zerodb-relay-auth-v2`/WELCOME; signed CreateNode, SetProperty LWW, SchemaEpoch n=1; merkle-walk catch-up; `EPOCH_UNKNOWN` fail-closed; advertised WELCOME limits. Smoke: `conformance/ts/peer/smoke.test.mjs`. **Not** M3c complete. |
| M3c-harness | Two-language golden/negative harness (H9) | open | Relay+peer vectors in two languages. |
| M3c-pack | Version/upgrade matrix, support profile | open | Packaging for the tag. |

### Pinned / remainder

| ID | Work | Status | Notes |
|----|------|--------|-------|
| M3b | Security remainder | open/pinned | E5–E8 live. H5 transcript AUTH, session limits/TLS-outside-dev, and H10 leftovers landed as `M3b-h5` / `M3b-limits` / `M3b-h10-remain` below. H6 parked M4. **Not** M3b exit. |
| M3b-h5 | Transcript AUTH (draft) | done(handshake + RELAY-HELLO-001 + limits H5 negatives) | `zerodb-relay-auth-v2` ‖ HELLO+nonce+intended WELCOME. v1 nonce-only `AUTH_FAILED`. Not a format freeze. |
| M3b-limits | Session rate/sub/conn + plaintext listen | done(`zerodb-relay/tests/limits.rs`) | `0x305 TOO_MANY_SUBS`, `0x304 RATE_EXCEEDED` / `TOO_MANY_CONNECTIONS`, `--allow-insecure`. No global quota. |
| M3b-h10-remain | H10 leftovers | done(`e6_encrypted_notes` H10 cases) | Offline-revoke at open, key-before/after-data hold, principal+device wrap, wrap-shape draft. **H10 not closed.** |
| perf-s2 | Stage 2 targeted projections | pinned | derived `op_targets`, AUTH control projection, single-pass replay rewrite, persisted CRDT accumulators. Trigger: Stage 0 still scan-dominated after Stage 1. |
| perf-s3 | Stage 3 bounded reconciliation | pinned | replace full OpId manifests; missing-only relay upload; compact Merkle snapshot cache. Trigger: equal/one-op-delta wire still full-history after Stage 1. |

### Later gates

| ID | Work | Status | Depends | Effort | Release |
|----|------|--------|---------|--------|---------|
| M4a | Browser/WASM/WebRTC/React | open | M3c | L | feature |
| M4b | Migration/snapshots/upgrade (E10) | open | M3c | L | feature |
| M5a | Operability (backup/restore, SLOs) | open | M3c | M | GA program |
| M5b | Lifecycle safety (GC, rolling upgrade) | open | M4b, C7 | L | GA program |
| M5c | Release assurance (fuzz/soak/audit) | open | M5a, M5b | L | GA decision |
| M6 | Ecosystem (per-epic) | open | compat stability | — | epics |
