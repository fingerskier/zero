# ZeroDB — Delivery Ledger

Canonical work tracker. A gate closes only with **Exit evidence** (commit, fixture path, or CI run).

**DRI:** `fingerskier` unless a row names someone else. Effort bands (rough solo): S ≤ 1 wk, M ≤ 1 mo, L > 1 mo. DQ-12 capacity ratification remains open.

Status: `open` · `in-progress` · `blocked(<on>)` · `done(<evidence>)`.

---

## Closed (index only)

| Gate | Closed | Where remembered |
|------|--------|------------------|
| P0 readiness (P0-1..P0-7) | 2026-07-16 | ISSUES Decision Log; conformance + INVARIANTS + EXEMPLAR in `doc/` |
| Composite M0a–M0f (contract-model) | 2026-07-18 | [SPEC §10](../doc/SPEC.md), package docs (KERNEL…FRONTIER); **109** vectors after 2026-08-14 CX-03..06 amend |
| DQ-1..DQ-8, DQ-10 | 2026-07-16/18 | AUTH / KERNEL / SCHEMA / WAL + Decision Log |
| Historical plan reviews (Codex 07-16, Grok plan 07-15) | dispositioned | Decision Log |
| July 2026 state reviews (FINDINGS.GROK / FINDINGS.CODEX) | archived 2026-08-14 | [plan/archive/](archive/) — CX-01/CX-02 closed in tree; do not treat as live backlog |

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

**Overall:** `done` — exit resolved 2026-07-25 (Decision Log; tag `v0.1.0-local`, experimental format, freeze still open). Implementation notes: [M1-LOCAL.md](../doc/M1-LOCAL.md).

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
| M1-e1 | Full E1 (restart, replay, larger load, HLC mono) | done(experimental; not full EXEMPLAR E1) | M | 50-todo is clean reopen (`e1_e2_acceptance`); kill and 1h clock-rollback are **separate** tests in `e1_kill_clock` (kill asserts `ops > prev_ops`, not pre-kill projection). Honesty leftovers: M2a-e1 | G-09 / G20-05a |
| M1-e2-store | Store-level equal-ts / equivocation suite | done(`e1_e2_acceptance` e2_*) | S | same-author exclude + cross-peer total order | G-09 |
| M1-e4 | Groups + WAL layer-2 crash injection | done(single-txn failpoints; not EXEMPLAR E4 / WAL SEAL-TRUNCATE) | L | 5 in-process failpoints × 3 commit paths in one SQLite txn (`e4_crash_matrix`) — rollback ≡ process death **before COMMIT**. `CRASH_AFTER_SEAL` / `CRASH_AFTER_TRUNCATE` N/A on this backend. GroupBuilder has no `create_edge`. Honesty leftovers: M2a-e4 | G-03 |
| M1-e9 | H3 derived visibility + edges + late-edge E9 | done(derived visibility; same-id resurrection still M2a) | L | set-derived edge tombstone + derived visibility, no cascade; tombstone-before-edge, late edge to dead endpoint, replay/restart, query exclusion — `e9_delete_machine` (5). Recreate-under-**new**-id tested; same-id CreateNode after tombstone + conflicting labels: M2a-store | G-04 / G20-02 |
| M1-schema | Schema apply / IR load; type pin; `ep` | done(JSON pin + CLI) | M | `apply_schema_json` + pin reject; `schema-apply` CLI; full CBOR IR/ep still draft | G-06/08 |
| M1-query | O3 minimal query + repl | done(eval + CLI) | M | `LocalStore::query` + `zerodb query`; no interactive repl yet | G-08 |
| M1-tsir | TS→IR compiler (≤ M1, O2) | done(minimal tool) | M | `tools/ts-to-ir` authoring JSON → pin IR; full CBOR SchemaId later | G-14 |
| M1-fmtver | `storage_format_version` meta + freeze discipline | done(meta v1 + backfill; freeze still open) | S | meta written at init/open; freeze still needs Decision Log | G-07 |
| M1-fix-init | Fail-closed init (no silent re-key) | done(`r0_stabilize` init tests) | S | re-init of empty-initialized and nonempty DB errors; identity+ops preserved | G20-03 |
| M1-exit | Close M1 when exit criteria met | done(Decision Log 2026-07-25; tag `v0.1.0-local`) | — | SPEC §10 M1 checklist ticked with evidence (E1/E2/E4/E9 suites); scope narrowing Decision Log entry (CBOR IR + indexes + repl → M2/M3); format freeze deliberately still open | — |

---

## Post-M1

| ID | Work | Status | Depends | Effort | Release |
|----|------|--------|---------|--------|---------|
| M2 | Node/NAPI SDK vertical (E11 provisional) | done | M1 (rides experimental LocalStore) | M | `v0.1.0-sdk` |

### M2 subtasks

| ID | Work | Status | Evidence |
|----|------|--------|----------|
| M2-napi-scaffold | `zerodb-napi` crate + `@zerodb/node` package | done | `zerodb-napi/`; `napi build` |
| M2-napi-crud | Database init/open/mutate/get/inspect/export/import/close | done | `test/m2-basic.test.mjs` (3) |
| M2-subscribe | `subscribe` / live change notifications | done (experimental) | `test/m2-subscribe.test.mjs` (3): op/import/replay events, unsubscribe |
| M2-query | O3 query via NAPI | done (experimental) | `test/m2-query.test.mjs` (2): match/where/return + parse reject |
| M2-ws-sync | `serve`/`connectPeer` WebSocket sync (protocol v2 two-way) | done (experimental) | `test/m2-sync.test.mjs` (6): converge both ways, stop/close, bad urls, LAN bind, timeouts |
| M2-autosync | `autoConnect`/`disconnect` background re-sync (dirty flag + interval poll + backoff) | done (experimental) | `test/m2-autosync.test.mjs` (2): auto-converge both ways, disconnect stops, sync-error retry, clean close; `examples/webapp` two-process demo |
| M2-lan-hardening | sync session timeout + non-loopback serve behind unsafe flag | done | 30s socket timeouts; per-connection serve threads; `serve(port, allowInsecureLan?)` binds 0.0.0.0 only with explicit flag; tests in `m2-sync.test.mjs` |
| M2-push | v2 push capability: persistent sessions (server streams new ops; client pushes on dirty) negotiated via `Hello.push`/`HelloOk.push` (serde defaults — old peers fall back to one-shot; CBOR wire still reserved for v3) | done | `zerodb_storage::sync::{serve_push,pull_push}` + `tests/sync_push.rs` (3: two-way push w/o new session, plain-serve fallback, capability field compat); NAPI `serve(port, lan?, push?)` + `autoConnect` upgrade, `test/m2-push.test.mjs` (2: push latency well under interval, push-disabled server poll fallback) |
| M2-ci | clean-checkout CI: NAPI build + JS suites; CRDT semantic + fixture parity | done([run 31836860347](https://github.com/fingerskier/zero/actions/runs/31836860347) @ `b352ca4`) | rust `--locked` + clippy `-D warnings` + fmt; napi ubuntu+windows; ts-to-ir; conformance required. Prior green: [run 30178840377](https://github.com/fingerskier/zero/actions/runs/30178840377) @ `3d5ae48` |
| M2-schema | canonical CBOR IR + SchemaId + `ep`/`deps` in store + NAPI `applySchema` | done(`m2_schema` + `m2-schema.test.mjs`) | pin or IR JSON → persist IR/SchemaId/ep=1; local ops stamp ep; import rejects >64 or missing deps |
| M2-crdts | MVRegister + resolve, RGA, LWWMap | deferred(app-trigger) | Decision Log 2026-07-25 / 2026-08-14 — not required for `v0.1.0-sdk` |
| M2-parity | binding parity vectors vs core fixtures | done(`applyCrdtVector` + `m2-parity.test.mjs`) | NAPI replays all 9 `required/crdt` fixtures through `zerodb_core::apply_crdt_vector` (same runner as `conformance_crdt`); high-level semantic smoke kept |
| M2-facade | thin promise/typed JS facade over sync NAPI | done(`zerodb-napi/zerodb.mjs`) | `ZeroDB.open/create/mutate/query`; NAPI remains the sync binding |
| M2-exit | Close `v0.1.0-sdk` | done(Decision Log 2026-08-14; tag `v0.1.0-sdk`) | narrowed checklist + CI green; experimental format; not E11 / extra CRDTs / query-subscribe / repl |

### M2a — Stabilize and schema (done)

Blocks `v0.1.0-sdk` and any M3 start. Adopted 2026-08-14.

| ID | Work | Status | Evidence / remaining |
|----|------|--------|----------------------|
| M2a-honesty | Tracker alignment + M2 scope Decision Log + FINDINGS archive | done(this refresh) | PLAN/LEDGER/M1-LOCAL/ISSUES/README/registry; `plan/archive/` |
| M2a-store | Conflicting/same-id CreateNode; HLC meta-ahead; import pin; shuffle replay | done(`r0_stabilize` + `m1_wave1`) | same-id no resurrection; HLC-order labels; inflated meta rewritten; import pin; shuffle replay identity |
| M2a-e1 | E1 honesty: do not claim EXEMPLAR E1 | done(LEDGER M1-e1 wording) | kill + clock remain separate suites; no pre-kill projection |
| M2a-e4 | E4 honesty: SEAL/TRUNCATE N/A on single-txn backend | done(LEDGER M1-e4 wording) | failpoint matrix kept; not EXEMPLAR E4 |
| M2a-m0 | CX-03 AAD slot-hash; CX-04 frontier tip; CX-05 resume; CX-06 WAL MUSTs + vectors | done(109 required vectors) | KERNEL §7 slot AAD; FrontierTip+HLC; delivery frontier cursor; WAL n/unique/visible + WAL-013..016 |
| M2a-relay | CX-08 accepted-set + RELAY-SPEC §7.4 (docs only) | done(RELAY 0.2.2-draft + transcripts) | dual roots; frames in `M3a-transcripts`; no relay binary |
| M2a-schema | persist M0b IR + SchemaId; stamp `ep`; enforce `deps`; NAPI `applySchema` | done(`m2_schema` + NAPI `applySchema`) | pin still accepted; compiles to IR |
| M2a-bind | NAPI/wasm edges + `listNodes` props parity + query params + promise facade | done | `listNodes.props`; `createEdge`/`deleteEdge`; `query(q, params)`; `zerodb.mjs` |
| M3a | L2 relay + offline catch-up (E3) | done(`merkle-walk-v1` + full E3) | M2, M0c/M0f | L | internal |
| M3a-transcripts | RELAY 0.2.2 handshake / dual-root / resume / reject-ack frames | done(`relay-transcript` 6) | two-language vectors RELAY-HELLO-001/002/003, ROOT-001, RESUME-001, REJECT-001 |
| M3a-relay | L2 `zerodb-relay` process | done(`m3a_process` + Merkle handlers) | handshake, SQLite validated oplog, dual-root SYNC, frozen walk snapshots, node/leaf responses, OpId delta batches; no authz (M3b) |
| M3a-client | LocalStore + NAPI RELAY client | done(`relay_client` + `m3a-relay.test.mjs`) | handshake, signed-op push, aligned local tree, mismatch pruning, leaf OpId difference, delta apply; `Database.connectRelay` reports walk counts |
| M3a-e2-live | E2-live CRDT matrix + E3-lite 3-peer catch-up | done(`relay_client` concurrent_crdts + three_peer + `m3a-relay.test.mjs` 3) | Historical smaller slice: concurrent LWW/ORSet/Flag/PNCounter; C offline; B clean close/reopen; C catches up from R alone; resume `received=0`. Full E3 and Merkle walk are evidenced by the next row. |
| M3a-merkle-e3 | Merkle wire walk + full E3 | done(`merkle_walk_prunes_matching_subtrees` + `full_exemplar_e3_1000_ops_hard_crash_and_relay_only_catchup`) | `merkle-walk-v1`; frozen responder snapshot; matching-subtree pruning; 1,000 partition writes; B hard-abort/reopen; C catches up from R alone; roots equal; resume receives 0 |
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
