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
| M1-e1 | Full E1 (restart, replay, larger load, HLC mono) | done(`e1_e2_acceptance` + `e1_kill_clock`) | M | 50-todo restart/import/replay; `tests/e1_kill_clock.rs` adds repeated hard-kill mid-write (child process, replay/state/sig re-verify each iteration) + 1h wall-clock rollback HLC monotonicity via `set_test_clock` hook | G-09 / G20-05a |
| M1-e2-store | Store-level equal-ts / equivocation suite | done(`e1_e2_acceptance` e2_*) | S | same-author exclude + cross-peer total order | G-09 |
| M1-e4 | Groups + WAL layer-2 crash injection | done(`e4_crash_matrix` + `atomic_group`) | L | named layer-2 crash matrix: 5 failpoints (before-txn / after-op-insert / before-hlc-persist / after-hlc-persist / before-commit, mapped to WAL §3) × 3 commit paths incl. mid-group seal rollback — `tests/e4_crash_matrix.rs` (`e4_commit_local_crash_matrix`, `e4_atomic_group_crash_matrix`, `e4_import_bundle_crash_matrix`); plus `e4_groups`, `m1_remainders`, `e1_kill_clock` random kills | G-03 |
| M1-e9 | H3 derived visibility + edges + late-edge E9 | done(`e9_delete_machine`) | L | set-derived edge tombstone (kind 4 `{edge}` ref) + derived visibility, no cascade; permutations (tombstone-before-edge, late edge to dead endpoint), replay/restart identity, hidden props/query exclusion, no resurrection — `tests/e9_delete_machine.rs` (5 tests); plus `m1_remainders`, `r0_stabilize` | G-04 / G20-02 |
| M1-schema | Schema apply / IR load; type pin; `ep` | done(JSON pin + CLI) | M | `apply_schema_json` + pin reject; `schema-apply` CLI; full CBOR IR/ep still draft | G-06/08 |
| M1-query | O3 minimal query + repl | done(eval + CLI) | M | `LocalStore::query` + `zerodb query`; no interactive repl yet | G-08 |
| M1-tsir | TS→IR compiler (≤ M1, O2) | done(minimal tool) | M | `tools/ts-to-ir` authoring JSON → pin IR; full CBOR SchemaId later | G-14 |
| M1-fmtver | `storage_format_version` meta + freeze discipline | done(meta v1 + backfill; freeze still open) | S | meta written at init/open; freeze still needs Decision Log | G-07 |
| M1-fix-init | Fail-closed init (no silent re-key) | done(`r0_stabilize` init tests) | S | re-init of empty-initialized and nonempty DB errors; identity+ops preserved | G20-03 |
| M1-exit | Close M1 when exit criteria met | open | — | R0.1 store safety landed; formal exit still needs R0.2 contracts, E1/E4 evidence honesty, SPEC checklist | — |

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
| M2-subscribe | `subscribe` / live change notifications | done (experimental) | `test/m2-subscribe.test.mjs` (3): op/import/replay events, unsubscribe |
| M2-query | O3 query via NAPI | done (experimental) | `test/m2-query.test.mjs` (2): match/where/return + parse reject |
| M2-ws-sync | `serve`/`connectPeer` WebSocket sync (protocol v2 two-way) | done (experimental) | `test/m2-sync.test.mjs` (3): converge both ways, stop/close, bad urls |
| M2-autosync | `autoConnect`/`disconnect` background re-sync (dirty flag + interval poll + backoff) | done (experimental) | `test/m2-autosync.test.mjs` (2): auto-converge both ways, disconnect stops, sync-error retry, clean close; `examples/webapp` two-process demo |
| M2-lan-hardening | sync session timeout + non-loopback serve behind unsafe flag | done | 30s socket timeouts on NAPI serve/connect/autoConnect sockets; per-connection serve threads (store lock only after WS handshake); `serve(port, allowInsecureLan?)` binds 0.0.0.0 only with explicit flag (CLI unchanged); tests: loopback-default, LAN bind, stalled-raw-socket recovery in zerodb-napi/test/m2-sync.test.mjs — 21/21 npm test green |
| M2-push | v2 push capability: persistent sessions (server streams new ops; client pushes on dirty) negotiated via `Hello.push`/`HelloOk.push` (serde defaults — old peers fall back to one-shot; CBOR wire still reserved for v3) | done | `zerodb_storage::sync::{serve_push,pull_push}` + `tests/sync_push.rs` (3: two-way push w/o new session, plain-serve fallback, capability field compat); NAPI `serve(port, lan?, push?)` + `autoConnect` upgrade, `test/m2-push.test.mjs` (2: push latency well under interval, push-disabled server poll fallback) |
| M2-ci | clean-checkout CI: NAPI build + JS suites; parity vectors for 5 shipped CRDTs | done-pending-first-CI-run | `ci.yml`: napi job (ubuntu+windows, `npm ci` + `napi build --platform --release --target <host>` + `npm test`), rust job now `--locked` + clippy `-D warnings` (5 pre-existing lints fixed in zerodb-core), ts-to-ir job; `test\m2-parity.test.mjs` (5): LWW/GCounter/PNCounter/ORSet/EWFlag single-store + cross-peer convergence via exportJson/importJson both ways; local: 21/21 napi tests, clippy clean, workspace tests green; actual GH Actions run not yet observed |
| M2-schema | schema apply / TS→IR pipeline (O2) | open | blocked(M1-schema/tsir) |
| M2-crdts | MVRegister + resolve, RGA, LWWMap | open | kernel + binding |
| M2-parity | binding parity vectors vs core fixtures | open | — |
| M2-exit | Close `v0.1.0-sdk` | blocked(above) | SPEC §10 M2 checklist |
| M3a | L2 relay + offline catch-up (E3) | open | M2, M0c/M0f | L | internal |
| M3b | Security: auth, envelope, negatives (E5–E8) | open | M3a, M0d | L | internal |
| M3c | Interop TS wire peer + release | open | M3b | M | `v0.1.0` |
| M4a | Browser/WASM/WebRTC/React | open | M3c | L | feature |
| M4a-browser-slice | Experimental wasm browser peer: `sqlite` feature gate + `MemoryBackend`, `zerodb-wasm` (`LocalStore<MemoryBackend>`), JS v2 sync driver + IndexedDB demo (`examples/browser-peer`) | done(memory_backend.rs 6 tests; sync-driver.test.mjs wasm<->NAPI convergence; wasm-pack build) | rides M1/M2 experimentally (M4a proper still needs M3c) | S | experiment |
| M4a-wasm-events | wasm `ZeroDb.onChange`/`offChange` change events (op/import/replay, NAPI subscribe shapes; synchronous — callbacks must defer re-entry) | done(sync-driver.test.mjs onChange test) | M4a-browser-slice | S | experiment |
| M4a-push-driver | JS `connectPush` persistent push session + `autoSync` push-preferred/poll-fallback | done(sync-driver.test.mjs pushed-op test: wasm receives NAPI push without re-sync) | M2-push | S | experiment |
| M4a-idb-journal | Incremental IndexedDB persistence: per-op journal keyed by op id appended on onChange (compact/rewrite on load; reset clears journal); true OPFS/sqlite-wasm backend remains future work | done(`examples/browser-peer/index.html`) | M4a-wasm-events | S | experiment |
| M4b | Migration/snapshots/upgrade (E10) | open | M3c | L | feature |
| M5a | Operability (backup/restore, SLOs) | open | M3c | M | GA program |
| M5b | Lifecycle safety (GC, rolling upgrade) | open | M4b, C7 | L | GA program |
| M5c | Release assurance (fuzz/soak/audit) | open | M5a, M5b | L | GA decision |
| M6 | Ecosystem (per-epic) | open | compat stability | — | epics |
