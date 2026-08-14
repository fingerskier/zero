# ZeroDB — Path-to-MVP Execution Plan

**Date:** 2026-08-14 (refreshed after M2-parity)  
**Status:** active — composite M0 **done** (contract-model, draft-1, unfrozen); M1 experimental exit **done** (`v0.1.0-local`); M2a **done**; M2-parity **done**. Next: Decision Log whether `v0.1.0-sdk` closes. **Do not start M3.**  
**Authority:** delivery/tracking only. [SPEC §10](../doc/SPEC.md) is the normative roadmap; [ISSUES.md](../doc/ISSUES.md) the issue ledger; [LEDGER.md](LEDGER.md) the live work tracker. On conflict, SPEC wins.

Completed P0 readiness, M0 packages, M1 experimental exit, and dispositioned review findings are recorded in the [ISSUES Decision Log](../doc/ISSUES.md) and are **not** re-listed here. July 2026 review files live in [plan/archive/](archive/).

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
| Composite M0 (contract-model, draft-1, 109 vectors) | **done** 2026-07-18 — CX-03..06 amended 2026-08-14; CX-08 still M2a-relay |
| Format freeze | **not done** — draft-1, unfrozen until an explicit Decision Log freeze names a versioned profile |
| Experimental M1 local store + file/TCP peer exchange | **done** (experimental) — [M1-LOCAL.md](../doc/M1-LOCAL.md) |
| M1 exit (`v0.1.0-local`) | **done** 2026-07-25 — experimental format; EXEMPLAR E1/E4 honesty leftovers in M2a |
| M2 Node/NAPI vertical | **in progress** — experimental sync NAPI + WS v2 + schema + fixture parity shipped; `v0.1.0-sdk` is a Decision Log / tag act |
| M2a stabilize + schema | **done** — [LEDGER.md](LEDGER.md) |
| M3a L2 relay | **not started** — blocked on M2a contract repair + RELAY rewrite |

---

## 3. Ground rules

- **No wire or persistent format freeze** without a Decision Log entry naming a versioned profile (composite M0 closed draft-1 only).
- **GC disabled** until C7 safety tests pass (M5b).
- **`zerodb-core` / `zerodb-storage` experimental** until freeze; types are not normative.
- **Approved-resolution checklist** (SPEC §10) is the only way a C/H issue closes.
- Lean proofs do not gate M0 or v0.1.
- Experimental multi-process TCP is **non-gating** for M1 exit unless SPEC/LEDGER say otherwise ([M1-LOCAL.md](../doc/M1-LOCAL.md)).

---

## 4. Open decisions

| ID | Decision | Blocks | Status |
|----|----------|--------|--------|
| DQ-9 | L2 durable catch-up mandatory for `v0.1.0`? | M3a | plan default **yes** (in SPEC §10) |
| DQ-11 | Approver + records location | process | plan default: **LEDGER** + ISSUES Decision Log |
| DQ-12 | Capacity / effort bands | schedule | **open** |

Resolved DQ-1..DQ-8, DQ-10 live in AUTH / KERNEL / SCHEMA / WAL — not tracked here.

---

## 5. Immediate next actions (ordered)

**Current work package: M2-exit Decision Log.** M2a (adopted 2026-08-14) and M2-parity are done. Do not start M3a against RELAY-SPEC 0.2.0-draft.

1. **Honesty** — PLAN/LEDGER/M1-LOCAL/ISSUES aligned with evidence; FINDINGS archived; M2 exit scope Decision-Logged.
2. **Store leftovers** — conflicting/same-id CreateNode, HLC-meta-ahead recover, import-time schema pin, E1/E4 claim wording, shuffle/replay identity.
3. **M0 implementability** — CX-03 envelope AAD cycle, CX-04 frontier late-op, CX-05 resume cursor, CX-06 WAL MUSTs + vectors. Slot-context AAD (recommended).
4. **Relay contract (docs only)** — CX-08 accepted-set vs peer Merkle; RELAY rewrite. No relay binary.
5. **M2-schema** — persist M0b CBOR IR + `SchemaId`; stamp `ep` / enforce `deps`; NAPI `applySchema`.
6. **Binding parity** — edges + `listNodes` + query params on NAPI/wasm; thin promise/typed JS facade.
7. **M2-parity** — NAPI `applyCrdtVector` replays `conformance/vectors/required/crdt/*` through the shared Rust kernel.

**Shipped already (do not re-open as next work):** LAN hardening, clean-checkout NAPI CI (ubuntu+windows green on `3d5ae48`, [run 30178840377](https://github.com/fingerskier/zero/actions/runs/30178840377)), E1 kill/clock suites, E4 single-txn failpoint matrix, v2 push, experimental wasm peer.

**Deferred with triggers:** M2-crdts (until an app needs MVRegister/RGA/LWWMap); E11 budgets; query-scoped subscribe; interactive `repl`; CBOR wire (protocol v3); OPFS/sqlite-wasm; WebRTC. Format freeze remains a separate Decision Log act.

**After M2a + M2-parity:** Decision Log whether `v0.1.0-sdk` closes. **Then and only then** M3a against the rewritten relay draft.

Live rows: [LEDGER.md](LEDGER.md). Historical July reviews: [plan/archive/](archive/).
