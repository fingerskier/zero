# ZeroDB — Path-to-MVP Execution Plan

**Date:** 2026-09-12
**Status:** M3c exited (`v0.1.0` Decision Log act @ `177e247`; steward tags). M4a-a landed main #23 @ `56a3bad`. M4a-hooks landed main #24 @ `3f81d62`. H6 first-cut landed main #25 @ `671adba`. H6 fanout+roles landed main #26 @ `45a88b5`. H6 close-candidate reconnect/admission/profile landed main #27 @ `ef2fca9`. HELLO.datastore AUTH bind landed main #28 @ `bfe69b9`. **H6 closed** this act (Decision Log 2026-09-12). Stage 0+1 landed `9903280`. E5–E8 live. M3b remainder pinned. Formats draft-1/unfrozen. **Not** M4a complete (O4 open, no E10), **not** M3b exit, **not** format freeze.
**Authority:** delivery/tracking only. [SPEC §10](../doc/SPEC.md) is the normative roadmap; [ISSUES.md](../doc/ISSUES.md) the issue ledger; [LEDGER.md](LEDGER.md) the live work tracker. On conflict, SPEC wins.

---

## 1. Release meaning

| Term | Definition |
|------|------------|
| **MVP** | `v0.1.0-local` — M1 exit: offline single-peer Rust core + SQLite + CLI |
| **First shippable product** | `v0.1.0` — M3c exit: secure multi-peer sync with offline catch-up |

Roadmap M0–M6 (including M3a/b/c, M4a/b, M5a/b/c) is normative in [SPEC §10](../doc/SPEC.md).

---

## 2. Where we are

| Gate | Status |
|------|--------|
| P0 readiness | **done** |
| Composite M0 (contract-model, draft-1, 109 vectors) | **done** |
| M1 / `v0.1.0-local` | **done** (experimental) |
| M2 / M2a / `v0.1.0-sdk` | **done** (experimental; not SPEC-complete M2) |
| M3a L2 relay + E3 | **done** |
| E5–E8 live evidence | **done** |
| Stage 0+1 | **done** — landed `9903280` |
| Format freeze | **not done** — draft-1, unfrozen |
| M3b | **not done** — remainder pinned. **Not** M3b exit |
| M3c | **done** — Decision Log act @ `177e247` |
| `v0.1.0` | **Decision Log act** — git tag follows steward; not format freeze |
| M4a-a | **done** — WASM + IDB/OPFS persist/reopen (main #23 @ `56a3bad`) |
| M4a-hooks | **done** — optional `@zerodb/react` over live WASM (main #24 @ `3f81d62`). Not M4a complete |
| M4a-webrtc | **done** — H6 first-cut DataChannel + SIGNAL (main #25 @ `671adba`) |
| M4a-h6-fanout | **done** — live WS SIGNAL fanout + PeerId roles (main #26 @ `45a88b5`) |
| M4a-h6-close | **done** — reconnect/resume, admission, `h6-profile` (main #27 @ `ef2fca9`) |
| M4a-h6-auth-ds | **done** — optional HELLO.datastore in AuthTranscript (main #28 @ `bfe69b9`) |
| H6 | **closed** — Decision Log this act. Shared peer protocol over DataChannel. **Not** M4a complete |

Detailed evidence lives in the [LEDGER Closed index](LEDGER.md) and the [ISSUES Decision Log](../doc/ISSUES.md).

---

## 3. Preserve

Must not regress:

- **Signed wire is source of truth.** Derived columns, order indexes, Merkle snapshots, and AUTH projections are acceleration only.
- **`replay_all` remains the oracle / recovery API.** Success-path import must stay equivalent (see `import_replay_equiv`).
- **CRDT convergence**, order-independent tombstones, E9 derived visibility.
- **AUTH §4** on persist/import/ingest; honest relay REJECT/AUTHZ; colluding relay still peer-rejected (E5/E7).
- **Encrypted LWW:** KERNEL §7 seal before persist; relay/non-recipient stay blind; `ENCRYPTED_PLAINTEXT`; set-before-create missing-node path treated encrypted if any IR label marks the path; admin-only `kr=2` current-key adoption (E6).
- **`CLOCK_DRIFT` quarantine + release** (E8 / H1 closed).
- **Frozen-snapshot Merkle walk**; matching-subtree prune (M3a / E3).
- **Advertised session limits:** `max_payload_bytes` per-op, `max_batch_*` per OPS, pre-decode frame ceiling, plus per-session `max_subscriptions` / `ops_per_second` / `bytes_per_second` and 3 connections per PeerId. Transcript AUTH is draft (`zerodb-relay-auth-v2`).
- **Schema is a signed KERNEL kind 5 `SchemaEpoch`.** `apply_schema_json` is a helper that emits the op. Peers without the epoch fail closed (`EPOCH_UNKNOWN`). n=1 / empty migration landed on main; wrap-body unfrozen.
- **Formats draft-1 / unfrozen.** GC off until C7 (M5b). `zerodb-core` / `zerodb-storage` experimental until freeze.
- **Approved-resolution checklist** (SPEC §10) is the only way a C/H issue closes.
- Keep `e5`/`e6`/`e7`/`e8` + `import_replay_equiv` + `limits` + m3a suites green.

---

## 4. Open decisions

| ID | Decision | Blocks | Status |
|----|----------|--------|--------|
| DQ-9 | L2 durable catch-up mandatory for `v0.1.0`? | M3a | **ratified** (default yes; evidence M3a) |
| DQ-11 | Approver + records location | process | plan default: **LEDGER** + ISSUES Decision Log |
| DQ-12 | Capacity / effort bands | schedule | **open** |

2026-08-28 operating decision (not a DQ id): M3b remainder stays **pinned**, not closed. See ISSUES Decision Log.

Resolved DQ-1..DQ-8, DQ-10 live in AUTH / KERNEL / SCHEMA / WAL — not tracked here.

---

## 5. Path forward (ordered)

This is the only live action list.

1. **M3c-a `SchemaEpoch`** — landed on main (PR #17): signed KERNEL kind 5 persist/ingest/import (n=1, empty migration; `encrypted: true` rides the op; unknown `ep` is `EPOCH_UNKNOWN`). Codex P1s: same-batch kind-5 applies before epoch-bound data; late ops validate against their own epoch IR (ep=0 schemaless). Fork/quarantine and non-empty migration DSL not started. Do not freeze wrap-body.
2. **M3c-b TS wire peer** — landed on main (PR #18): independent TypeScript wire peer evolved from the conformance runner (`conformance/ts/peer/`), **not** NAPI-backed (SPEC M3c). Speaks live RELAY 0.2 HELLO/AUTH/WELCOME, signed KERNEL ops including kind 5, merkle-walk catch-up, `EPOCH_UNKNOWN` fail-closed, advertised WELCOME limits.
3. **M3c-c two-language harness** — landed on main (PR #19): golden/negative relay+peer vectors in Rust + independent TS (H9). Registry is the protocol definition; `conformance/schemas/` is generated from it. Evidence: `RELAY-OPS-001`, `RELAY-WALK-001`, `RELAY-LIMIT-001`, `PEER-EPOCH-001`, `PEER-REJECT-001..004` in `conformance/vectors/required/` (green in `conformance/ts/runner.mjs` and `zerodb-core` `conformance_relay` / `conformance_peer`). HELLO/AUTH/WELCOME already on main as `RELAY-HELLO-001..003`. H9 not removed; formats remain draft-1 / unfrozen.
4. **M3c-d packaging** — landed on main (PR #20): support profile ([SUPPORT.md](../doc/SUPPORT.md)), v0.1 window-size-1 upgrade matrix ([UPGRADE.md](../doc/UPGRADE.md)), changelog / crate version story (`0.1.0-alpha`, unpublished). Formats remain draft-1 / unfrozen. M4 adjacent-version / rolling-upgrade tests not started.
5. **`v0.1.0` Decision Log act** — landed @ `177e247` (#22): names `v0.1.0` as first multi-peer secure product slice with offline catch-up (SPEC M3c exit). Evidence is H9 two-language fixtures (#19), existing Rust E3, and TS smoke. Live Rust↔TS partition/rejoin is follow-on, not this tag, not format freeze. Steward creates the git tag. Formats remain draft-1 / unfrozen. **Not** M3b exit / H9 closed.
6. **M4a-a** — landed main #23 @ `56a3bad`: IndexedDB + OPFS adapters behind `zerodb-wasm`; persist/reopen of signed KERNEL ops. Occupied-IDB auto, versionless IDB open, serialized persist, fail-closed OPFS identity. O4 gzip 262.6 KiB vs ~250 KB stays open. Not M4a complete.
7. **M4a-hooks** — landed main #24 @ `3f81d62`: optional `@zerodb/react` (`ZeroDbProvider`, `useQuery` / `useNode` / `useMutation` / `useSyncStatus`) wrapping the live WASM API + `journal.persist`. No typed query DSL. `useSyncStatus` is local ready/offline. Not M4a complete.
8. **M4a-webrtc** — landed main #25 @ `671adba`: SIGNAL (0x42) + ordered `zerodb-relay` DataChannel carrying the shared peer protocol (HELLO / `zerodb-relay-auth-v2` / WELCOME / OPS). Reuses `handshake.rs` `AuthTranscript` — no second AUTH preimage. Evidence: `conformance/ts/webrtc/webrtc.test.mjs` + `zerodb-relay/tests/signal.rs`.
9. **M4a-h6-fanout** — landed main #26 @ `45a88b5`: live `zerodb-relay` WebSocket SIGNAL fanout + PeerId-order role negotiation. Same `AuthTranscript` / `zerodb-relay-auth-v2`.
10. **M4a-h6-close** — landed main #27 @ `ef2fca9`: reconnect/resume, session admission, named `h6-profile`.
11. **M4a-h6-auth-ds** — landed main #28 @ `bfe69b9`: optional `HELLO.datastore` bound into `AuthTranscript` / `zerodb-relay-auth-v2` (omit when absent). A swapped or malformed offer fails AUTH before OPS. Evidence: `H6-AUTH-003`, handshake unit tests, `webrtc.test.mjs` MITM swap.
12. **H6 Decision Log close** — landed main #29 @ `a4fc3b8`: **H6 closed.** Shared peer protocol over DataChannel ([RELAY-SPEC](../doc/RELAY-SPEC.md) §14.2). DC AUTH proves the HELLO client; CHALLENGE/WELCOME stay unsigned (H5). Evidence #25–#28. TURN/NAT parked as infra (no hosted TURN / `wrtc`). **Not** M4a complete. No E10.
14. **H5 slice — DTLS channel binding (this PR)** — `HELLO.channel_binding` over both DTLS fingerprints in the same `AuthTranscript`; DC server verifies its own derivation before CHALLENGE. Closes the bridged-DTLS MITM found in the 2026-09-12 review. Evidence: `H6-AUTH-004`, `webrtc.test.mjs` bridged-MITM, handshake unit tests. **H5 not closed** (server identity, signed CHALLENGE/WELCOME, TLS).

**Pinned (do not start):**
- **perf Stage 2** — trigger: Stage 0 still scan-dominated
- **perf Stage 3** — trigger: equal/one-op-delta still full-history
- **H10** remains open (leftovers implemented this pass: offline-revoke at `open`, bootstrap hold, principal/device wrap, wrap-shape draft). Not closed.
- M3b remainder stays pinned/open (this work is the pinned remainder, not a gate rename / not M3b exit)
- M2-crdts (until an app needs MVRegister/RGA/LWWMap); E11; query-scoped subscribe; interactive `repl`; CBOR wire (protocol v3); OPFS/sqlite-wasm
- Experimental browser-peer/IDB slice grew into M4a-a adapters (#23); hooks landed #24; H6 first-cut #25; fanout+roles #26; close candidate #27; HELLO.datastore AUTH bind #28; H6 closed this act on that track

Live rows: [LEDGER.md](LEDGER.md). Historical July reviews: [plan/archive/](archive/).
