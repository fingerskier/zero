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

Current work package (adopted 2026-07-25; innate sync stages 1–4 shipped: `ce71349`, `4146f2a`, `a2b2377`):

1. **Sync LAN hardening** — session timeout (a slow peer currently holds the store lock for its session) + non-loopback `serve` behind an explicit unsafe flag (mirror CLI `--allow-insecure-lan`).
2. **CI completeness** — clean-checkout CI job building zerodb-napi and running the JS test suites; binding parity vectors for the 5 shipped CRDTs vs core fixtures.
3. **E1 hard evidence** — kill-not-shutdown and 1-hour clock-rollback store tests (durability claims the sync path now leans on).
4. **R0.2 wire stance** — decided (Decision Log 2026-07-25): JSON wire stays v2 experimental; canonical CBOR lands with protocol v3 (one wire migration).

Deferred with triggers: M2-crdts (until an app needs MVRegister/RGA), E4 crash matrix (formal M1 exit pass), CBOR wire + server-push (protocol v3), browser wasm/OPFS peer (after this package or on demand).

**M1 exit: resolved 2026-07-25** (Decision Log; tag `v0.1.0-local`, experimental format; CBOR IR/indexes/repl re-scoped to M2/M3; freeze stays a separate Decision Log act). Then: **M3a→b→c** — L2 relay, security, interop wire peer → `v0.1.0`.

Live rows and exit evidence: [LEDGER.md](LEDGER.md). Open review backlog: [FINDINGS.GROK.md](FINDINGS.GROK.md).
