# ZeroDB — Delivery Ledger

Canonical work tracker. A gate closes only with **Exit evidence** (commit, fixture path, or CI run).

**DRI:** `fingerskier` unless a row names someone else. Effort bands (rough solo): S ≤ 1 wk, M ≤ 1 mo, L > 1 mo — informational; DQ-12 capacity ratification was dropped 2026-09-12.

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
| M3c Interop + release (`v0.1.0`) | 2026-09-11 | Decision Log act #22 @ `177e247`; **tagged `v0.1.0`**. #17 SchemaEpoch n=1, #18 TS wire peer, #19 H9 two-language fixtures (H9 not removed), #20 SUPPORT/UPGRADE, #21 WELCOME `protocol_version` reject (`0x102`), existing Rust E3, TS smoke. Live Rust↔TS partition/rejoin is follow-on, not this tag, not format freeze. Not M3b exit / H9 closed. |
| M4a-browser-slice / wasm-events / push-driver / idb-journal | experimental shipped | Grew into M4a-a (#23, `zerodb-wasm/js/storage.mjs`). Not M4a complete. |
| M4a-a WASM + IDB/OPFS | 2026-09-11 | #23 @ `56a3bad`; `zerodb-wasm/test/persist-reopen.test.mjs`. Not M4a complete. |
| M4a-hooks `@zerodb/react` | 2026-09-11 | #24 @ `3f81d62`; `zerodb-react/test/hooks.test.mjs`. `useSyncStatus` local only. |
| H6 direct P2P protocol | 2026-09-12 | Decision Log; RELAY-SPEC §14.2. #25 @ `671adba` first-cut, #26 @ `45a88b5` WS SIGNAL fanout + PeerId roles, #27 @ `ef2fca9` reconnect/admission/`h6-profile`, #28 @ `bfe69b9` HELLO.datastore bind, #29 @ `a4fc3b8` close. Fake ordered DataChannel; TURN is infra. |
| O4 pinned | 2026-09-12 | Decision Log; #30 @ `0e90aef`. Out of the M4a gate; CI ceiling 300 KiB gzip; RGA/Richtext modules ride M2-crdts. Not closed. |
| M4a-h5-binding DTLS channel binding | 2026-09-12 | #31 @ `a322c4d`; `H6-AUTH-004` both runners; `webrtc.test.mjs` bridged-MITM + control; entrypoints require binding. H5 not closed. |
| M3b-relay-harden transport hardening + in-process TLS | 2026-09-12 | #32 @ `175784f`; `zerodb-relay/tests/hardening.rs` (9). Issue "no wss listener" closed. Not M3b exit. |
| O6 resolved | 2026-09-12 | Decision Log; advertised limits, rate windows, subscription/connection caps, pre-buffer ceiling all enforced (`limits.rs`, `hardening.rs`). Quotas/datastore-creation policy stay M3b remainder. |
| M3b-c5-attest kr=0 self-attestation | 2026-09-15 | AUTH §4.1; `e6_well_signed_foreign_root_does_not_rebind`. Not C5/PKI closed, not M3b exit. |

Detailed resolved-issue audit prose lives in the [ISSUES Decision Log](../doc/ISSUES.md) only (no second copy here).

---

## Decisions still open

| ID | Status | Blocks |
|----|--------|--------|
| DQ-9 L2 catch-up mandatory for v0.1 | **ratified** (M3a; default yes) | — |
| DQ-11 approver + records | plan default: this ledger + Decision Log | — |

DQ-12 (capacity / effort bands) dropped 2026-09-12 — no owner, blocked nothing.

---

## Live work

### M3c — done (see Closed index)

`v0.1.0` tagged @ `177e247` (#22). Slice rows #17–#21 are indexed above; nothing live.

### Pinned / remainder

| ID | Work | Status | Notes |
|----|------|--------|-------|
| M3b | Security remainder | open/pinned | E5–E8 live. H5 transcript AUTH + DTLS binding, session limits, transport hardening + TLS, H10 leftovers, and kr=0 self-attestation (`M3b-c5-attest`) landed. **Not** M3b exit — outstanding: C5/PKI device-cert trust store (data-op membership still solo-device), H10 close (rotation, wrap-body), H8 direction (undecided 2026-09-12), quotas / datastore-creation policy. H5 remainder is handshake-server identity. |
| M3b-h5 | Transcript AUTH (draft) | done(handshake + RELAY-HELLO-001 + limits H5 negatives) | `zerodb-relay-auth-v2` ‖ HELLO+nonce+intended WELCOME. v1 nonce-only `AUTH_FAILED`. Not a format freeze. |
| M3b-limits | Session rate/sub/conn + plaintext listen | done(`zerodb-relay/tests/limits.rs`) | `0x305 TOO_MANY_SUBS`, `0x304 RATE_EXCEEDED` / `TOO_MANY_CONNECTIONS`, `--allow-insecure`. No global quota. |
| M3b-h10-remain | H10 leftovers | done(`e6_encrypted_notes` H10 cases) | Offline-revoke at open, key-before/after-data hold, principal+device wrap, wrap-shape draft. **H10 not closed.** |
| M3b-relay-harden | Relay transport hardening + in-process TLS | done(landed main #32 @ `175784f`) | WS message ceiling before buffering; handshake deadline (`0x100 HANDSHAKE_TIMEOUT`); idle timeout (`GOODBYE IDLE_TIMEOUT`; PING/PONG keepalive); global `--max-connections` (`0x304`); GOODBYE; `--tls-cert`/`--tls-key` wss via rustls. Evidence: `zerodb-relay/tests/hardening.rs`. Pinned: datastore-creation policy, op/byte quotas, walk/response limits, CA/rotation. **Not** M3b exit. |
| M3b-quotas | Datastore-creation policy; per-principal / per-datastore / global op-byte quotas; walk/response limits | pinned | Remaining "before network exposure" items (PERF). No trigger set. |
| perf-s2 | Stage 2 targeted projections | pinned | derived `op_targets`, AUTH control projection, single-pass replay rewrite, persisted CRDT accumulators. Trigger: Stage 0 still scan-dominated after Stage 1. |
| perf-s3 | Stage 3 bounded reconciliation | pinned | replace full OpId manifests; missing-only relay upload; compact Merkle snapshot cache. Trigger: equal/one-op-delta wire still full-history after Stage 1. |
| perf-bench | Benchmark harness (1k/10k/100k) for the four P0 findings | first slice done (`bench/relay-chat/`); remainder pinned | Relay path: cold / reconnect / delta / sparse / chat scenarios, wire bytes via counting proxy, `RelayStats` (`--stats-interval-secs`), `/proc` RSS+CPU, delivery latency. Baselines in `bench/results/`, not published numbers. Missing: local 10k/100k Stage 0 fixture (P0-1), direct-peer scenario set (P0-2), 100k relay history, DataChannel path. CI runs `smoke.test.mjs` (shape only). |

### M4a — Browser / WASM / WebRTC / React (open)

Depends: M3c done. Landed slices (#23–#32) are in the Closed index. **Not** M4a complete. Exit gate (SPEC §10 M4a, added 2026-09-12): real-browser DataChannel path, direct/relay parity, browser restart/offline tests. E10 is M4b.

| ID | Work | Status | Notes |
|----|------|--------|-------|
| M4a-browser-dc | Real-browser WebRTC DataChannel path | open | `conformance/ts/webrtc/` driven by an actual `RTCPeerConnection` (browser or `wrtc`-class runtime), SIGNAL via live `zerodb-relay`, `channelBindingFor(pc)` on real SDP, OPS converge. Fake channel stays the protocol oracle. TURN/NAT is infra. |
| M4a-parity | Direct/relay parity | open | Same op set converges identically over DataChannel and via relay, incl. reconnect / `resume-cursor`; one fixture set, both transports. |
| M4a-offline | Browser restart/offline tests | open | `openDurable` reload + offline edits + later sync over both transports; todo app or browser-peer example as harness. |
| M4a-todo-transport | Todo app cannot reach a LAN peer from GitHub Pages | todo (optional) | Direct NAPI `db.serve` peer has no TLS listener; relay TLS does not help. Either add wss to `db.serve` / `zerodb serve`, or route the app through `zerodb-relay`. Not scheduled. |
| O4 | WASM size budget | pinned | Pinned 2026-09-12 (#30): out of the M4a gate. CI ceiling 300 KiB gzip; artifact 262.6 KiB. RGA/Richtext modules ride M2-crdts. Not closed. |
| M4a | Browser/WASM/WebRTC/React | open | Exit claim only after the three rows above; Decision Log names it. |

### Later gates

| ID | Work | Status | Depends | Effort | Release |
|----|------|--------|---------|--------|---------|
| M4a | Browser/WASM/WebRTC/React | open | M3c | L | feature |
| M4b | Migration/snapshots/upgrade (E10) | open | M3c | L | feature |
| M5a | Operability (backup/restore, SLOs) | open | M3c | M | GA program |
| M5b | Lifecycle safety (GC, rolling upgrade) | open | M4b, C7 | L | GA program |
| M5c | Release assurance (fuzz/soak/audit) | open | M5a, M5b | L | GA decision |
| M6 | Ecosystem (per-epic) | open | compat stability | — | epics |
