# ZeroDB — Path-to-MVP Execution Plan

**Date:** 2026-08-28
**Status:** current work **M3c**. Stage 0+1 landed `9903280`. E5–E8 live. M3b remainder pinned. Formats draft-1/unfrozen. **Not** M3b exit, **not** `v0.1.0`.
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
| M3c | **open** — current work |
| `v0.1.0` | **not done** |

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
- **Schema is a signed KERNEL kind 5 `SchemaEpoch`.** `apply_schema_json` is a helper that emits the op. Peers without the epoch fail closed (`EPOCH_UNKNOWN`). n=1 / empty migration on this PR; wrap-body unfrozen.
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

1. **M3c-a `SchemaEpoch`** — signed KERNEL kind 5 persist/ingest/import landed this PR (n=1, empty migration; `encrypted: true` rides the op; unknown `ep` is `EPOCH_UNKNOWN`). Codex P1s: same-batch kind-5 applies before epoch-bound data; late ops validate against their own epoch IR (ep=0 schemaless). Fork/quarantine and non-empty migration DSL not started. Do not freeze wrap-body. **Not** M3c complete.
2. **M3c-b TS wire peer** — this PR: independent TypeScript wire peer evolved from the conformance runner (`conformance/ts/peer/`), **not** NAPI-backed (SPEC M3c). Speaks live RELAY 0.2 HELLO/AUTH/WELCOME, signed KERNEL ops including kind 5, merkle-walk catch-up, `EPOCH_UNKNOWN` fail-closed, advertised WELCOME limits. **Not** M3c complete.
3. **M3c-c two-language harness** — golden/negative vectors for relay+peer in two languages (H9).
4. **M3c-d packaging** — version/upgrade matrix, support profile.
5. **`v0.1.0` tag** — only after M3c-a..d and a Decision Log act at tag time. Still not format freeze unless that act says so.

**Pinned (do not start):**
- **perf Stage 2** — trigger: Stage 0 still scan-dominated
- **perf Stage 3** — trigger: equal/one-op-delta still full-history
- **H6 direct P2P** — parked to M4; reuse the handshake transcript helper
- **H10** remains open (leftovers implemented this pass: offline-revoke at `open`, bootstrap hold, principal/device wrap, wrap-shape draft). Not closed.
- M3b remainder stays pinned/open (this work is the pinned remainder, not a gate rename / not M3b exit)
- M2-crdts (until an app needs MVRegister/RGA/LWWMap); E11; query-scoped subscribe; interactive `repl`; CBOR wire (protocol v3); OPFS/sqlite-wasm; WebRTC
- Experimental browser-peer/IDB slice already shipped; M4a proper still waits on M3c

Live rows: [LEDGER.md](LEDGER.md). Historical July reviews: [plan/archive/](archive/).
