# ZeroDB — Path-to-MVP Execution Plan

**Date:** 2026-09-12
**Status:** M3c exited and tagged (`v0.1.0` @ `177e247`). M4a in progress: H6 closed (#29), O4 pinned (#30), DTLS channel binding (#31), relay transport hardening + in-process TLS (#32). M3b remainder pinned (**not** M3b exit). Stage 0+1 perf landed; Stage 2/3 pinned. All formats draft-1 / unfrozen. **Not** M4a complete, **not** format freeze.
**Authority:** delivery/tracking only. [SPEC §10](../doc/SPEC.md) is the normative roadmap; [ISSUES.md](../doc/ISSUES.md) the issue ledger; [LEDGER.md](LEDGER.md) the live work tracker. On conflict, SPEC wins. Working conventions: [AGENTS.md](../AGENTS.md).

---

## 1. Release meaning

| Term | Definition |
|------|------------|
| **MVP** | `v0.1.0-local` — M1 exit: offline single-peer Rust core + SQLite + CLI |
| **First shippable product** | `v0.1.0` — M3c exit: secure multi-peer sync with offline catch-up (tagged; experimental format) |

Roadmap M0–M6 (including M3a/b/c, M4a/b, M5a/b/c) is normative in [SPEC §10](../doc/SPEC.md).

---

## 2. Where we are

| Gate | Status |
|------|--------|
| P0 readiness, composite M0 (draft-1, 109 vectors) | **done** |
| M1 `v0.1.0-local`, M2 `v0.1.0-sdk` | **done** (experimental; M2 not SPEC-complete) |
| M3a L2 relay + E3 | **done** |
| M3b security | **pinned remainder** — E5–E8 live; transcript AUTH, session limits, transport hardening + TLS, H10 leftovers, kr=0 self-attestation landed. **Not** M3b exit (C5/PKI trust store, H10 close, H8 direction outstanding) |
| M3c `v0.1.0` | **done and tagged** @ `177e247` (#22); not a format freeze |
| M4a platform | **in progress** — WASM + IDB/OPFS (#23), React hooks (#24), H6 protocol closed over a fake DataChannel (#25–#29), DTLS binding (#31). Open: real-browser DataChannel path, direct/relay parity, browser restart/offline tests |
| O4 WASM size | **pinned** (#30) — CI ceiling 300 KiB gzip; not an M4a gate |
| M4b evolution (E10), M5a/b/c, M6 | **open** |
| Format freeze | **not done** — draft-1, unfrozen |

Evidence per gate: [LEDGER Closed index](LEDGER.md), [ISSUES Decision Log](../doc/ISSUES.md), [CHANGELOG](../CHANGELOG.md).

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
- **One AUTH preimage.** Relay WebSocket and DataChannel both sign `zerodb-relay-auth-v2` over HELLO + nonce + intended WELCOME; optional `HELLO.datastore` and `HELLO.channel_binding` ride the same hello map and are omitted when absent. No second domain. DataChannel entrypoints require the binding (RELAY-SPEC §14.2).
- **Relay limits and transport hardening:** `max_payload_bytes` per-op, `max_batch_*` per OPS, pre-decode / pre-buffer frame ceiling, per-session `max_subscriptions` / `ops_per_second` / `bytes_per_second`, 3 connections per PeerId, global `--max-connections`, handshake deadline from TCP accept, idle timeout with PING/PONG keepalive, `--tls-cert`/`--tls-key` in-process TLS (RELAY-SPEC §5.4, §8).
- **Schema is a signed KERNEL kind 5 `SchemaEpoch`.** `apply_schema_json` is a helper that emits the op. Peers without the epoch fail closed (`EPOCH_UNKNOWN`). n=1 / empty migration landed; wrap-body unfrozen.
- **Formats draft-1 / unfrozen.** GC off until C7 (M5b). `zerodb-core` / `zerodb-storage` experimental until freeze.
- **Approved-resolution checklist** (SPEC §10) is the only way a C/H issue closes.
- Keep `e5`/`e6`/`e7`/`e8` + `import_replay_equiv` + `limits` + `hardening` + m3a suites, the conformance required lane, and `webrtc.test.mjs` green.

---

## 4. Open decisions

| ID | Decision | Status |
|----|----------|--------|
| DQ-9 | L2 durable catch-up mandatory for `v0.1.0`? | **ratified** (default yes; evidence M3a) |
| DQ-11 | Approver + records location | plan default: **LEDGER** + ISSUES Decision Log |

DQ-12 (capacity / effort bands) was **dropped 2026-09-12** — no owner, blocked nothing; effort bands stay informational in the LEDGER header. Resolved DQ-1..DQ-8, DQ-10 live in AUTH / KERNEL / SCHEMA / WAL. 2026-08-28 operating decision: M3b remainder stays **pinned**, not closed.

---

## 5. Path forward (ordered)

This is the only live action list. Everything landed before 2026-09-12 is indexed in [LEDGER](LEDGER.md), not repeated here.

1. **M4a-browser-dc** — real-browser WebRTC DataChannel path: `conformance/ts/webrtc/` protocol code driven by an actual `RTCPeerConnection` (browser or `wrtc`-class runtime), SIGNAL through a live `zerodb-relay`, `channelBindingFor(pc)` on real SDP, OPS converge. Fake-channel tests stay as the protocol oracle. TURN/NAT remains a deployment choice.
2. **M4a-parity** — direct/relay parity: the same op set converges identically whether it travels peer↔peer over the DataChannel or peer↔relay↔peer, including reconnect/`resume-cursor`; one fixture set, both transports.
3. **M4a-offline** — browser restart/offline tests: `openDurable` reload + offline edits + later sync, over both transports, with the todo app or the browser-peer example as the harness.
4. **M4a exit claim** — Decision Log naming M4a complete against SPEC §10's M4a exit gate (added 2026-09-12). Not before 1–3.

**Pinned (do not start):**
- **O4 WASM size budget** — pinned 2026-09-12; trigger: an app needs RGA/Richtext in the browser (then decide optional modules). CI ceiling 300 KiB gzip stands meanwhile.
- **H5 remainder** — handshake-server identity (signed CHALLENGE/WELCOME or an explicit Decision Log exception). TLS on the relay path and DTLS binding on the DataChannel are the interim story.
- **M3b exit** — needs C5/PKI trust store (kr=0 self-attestation landed 2026-09-15; data-op membership still solo-device), H10 close, H8 direction; remainder rows in LEDGER. Not a gate rename.
- **perf Stage 2** — trigger: Stage 0 still scan-dominated. **perf Stage 3** — trigger: equal/one-op-delta still full-history. Relay-side evidence now comes from `bench/relay-chat/` (LEDGER `perf-bench`, first slice landed); the local 10k/100k fixture and a direct-peer scenario set are still missing.
- **Todo-app transport** — GitHub Pages todo app cannot reach a LAN peer (direct NAPI peer has no TLS listener). Optional either way: wss on `db.serve`, or route the app via the relay. Tracked in LEDGER; not scheduled.
- M2-crdts (until an app needs MVRegister/RGA/LWWMap); E11; query-scoped subscribe; interactive `repl`; CBOR wire (protocol v3); OPFS/sqlite-wasm.

Live rows: [LEDGER.md](LEDGER.md). Historical July reviews: [plan/archive/](archive/).
