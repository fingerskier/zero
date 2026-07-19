# ZeroDB — Path-to-MVP Execution Plan

**Date:** 2026-07-19 (refreshed after composite M0 + M1 prototype)  
**Status:** active — composite M0 **done** (contract-model); M1 **in progress** toward `v0.1.0-local`  
**Authority:** delivery/tracking only. [SPEC §10](../doc/SPEC.md) is the normative roadmap; [ISSUES.md](../doc/ISSUES.md) the issue ledger; [LEDGER.md](LEDGER.md) the live work tracker. On conflict, SPEC wins.

Completed P0 readiness, M0 packages, and dispositioned review findings are recorded in the [ISSUES Decision Log](../doc/ISSUES.md) and are **not** re-listed here.

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
| Composite M0 (contract-model, draft-1, 103 vectors) | **done** 2026-07-18 |
| Format freeze | **not done** — needs explicit Decision Log freeze |
| Experimental M1 local store + file/TCP peer exchange | **in progress** — [M1-LOCAL.md](../doc/M1-LOCAL.md); **not** M1 exit |
| M1 exit (`v0.1.0-local`) | **open** — [LEDGER.md](LEDGER.md) |

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

1. **M1 remaining** — E4 crash matrix, H3/E9, schema/query/TS→IR (or Decision-Log re-scope). Wave 1 + E1/E2 harness + partial E4 done.
2. **M2 in progress** — `@zerodb/node` NAPI vertical over experimental LocalStore (CRUD/export/import green). Still need subscribe, query, schema, extra CRDTs, parity vectors.
3. **Format freeze** only via Decision Log if retaining local DBs.
4. **M3a→b→c** — L2 relay, security, interop wire peer → `v0.1.0`.

Live rows and exit evidence: [LEDGER.md](LEDGER.md). Open review backlog: [FINDINGS.GROK.md](FINDINGS.GROK.md).
