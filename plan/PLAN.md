# ZeroDB — Path-to-MVP Execution Plan

**Date:** 2026-08-15 (refreshed after M3a-e2-live)  
**Status:** active — composite M0 **done** (contract-model, draft-1, unfrozen); M1 experimental exit **done** (`v0.1.0-local`); M2 experimental exit **done** (`v0.1.0-sdk`). Current: **M3a** still open — client + E2-live / E3-lite **done**; Merkle walk and full EXEMPLAR E3 still later.  
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
| Composite M0 (contract-model, draft-1, 109 vectors) | **done** 2026-07-18 — CX-03..06 amended 2026-08-14; CX-08 dual-root direction in RELAY 0.2.1-draft |
| Format freeze | **not done** — draft-1, unfrozen until an explicit Decision Log freeze names a versioned profile |
| Experimental M1 local store + file/TCP peer exchange | **done** (experimental) — [M1-LOCAL.md](../doc/M1-LOCAL.md) |
| M1 exit (`v0.1.0-local`) | **done** 2026-07-25 — experimental format |
| M2 Node/NAPI vertical | **done** 2026-08-14 — experimental `v0.1.0-sdk`; not SPEC-complete M2 |
| M2a stabilize + schema | **done** — [LEDGER.md](LEDGER.md) |
| M3a L2 relay | **in progress** — process **done** (`9603a6c`); client **done** (`73a6b6d`); E2-live / E3-lite **done** (this slice). Merkle walk and full EXEMPLAR E3 still open. |

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

**Current work package:** M3a leftovers. Do **not** start M3b.

1. ~~Golden two-language frames~~ **done**.
2. ~~L2 relay process (`zerodb-relay`)~~ **done** (`9603a6c`).
3. ~~RELAY 0.2.2 client (`connectRelay`)~~ **done** (`73a6b6d`).
4. ~~E2-live / E3-lite on that client~~ **done** (`relay_client` + `m3a-relay.test.mjs`). Concurrent LWW/ORSet/Flag/PNCounter; 3-peer C-offline catch-up; B sqlite close/reopen (not process death); resume `received=0`.
5. Merkle *walk* still later. Full EXEMPLAR E2 equal-ts and 1000-op E3 still later.

**Shipped already (do not re-open as next work):** M1 experimental exit; M2a; M2-parity; `v0.1.0-sdk` (CI [run 31836860347](https://github.com/fingerskier/zero/actions/runs/31836860347) @ `b352ca4`); M3a-client; this E2-live / E3-lite slice.

**Deferred with triggers:** M2-crdts (until an app needs MVRegister/RGA/LWWMap); E11 budgets; query-scoped subscribe; interactive `repl`; CBOR wire (protocol v3); OPFS/sqlite-wasm; WebRTC. Format freeze remains a separate Decision Log act.

Live rows: [LEDGER.md](LEDGER.md). Historical July reviews: [plan/archive/](archive/).
