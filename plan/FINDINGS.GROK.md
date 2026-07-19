# ZeroDB open findings — Grok (2026-07-19)

**Baseline:** `f9cc660`  
**Role:** open review backlog only. Work is tracked in [LEDGER.md](LEDGER.md). Implemented M1 behavior is documented in [M1-LOCAL.md](../doc/M1-LOCAL.md).  
**Historical** plan/spec reviews (2026-07-15 Grok plan review, 2026-07-16 Codex) were dispositioned into the ISSUES Decision Log and removed from `plan/`.

**Verdict:** Composite M0 model gate is solid. Experimental M1 local/TCP slice is real progress. **Not** M1 exit.

---

## Open items → LEDGER

| ID | Severity | One-liner | LEDGER |
|----|----------|-----------|--------|
| G-01 | bug | ~~UUID-tiled seed~~ → OS CSPRNG | **done** M1-fix-rng |
| G-02 | bug | ~~meta-only HLC~~ → oplog max on open/replay | **done** M1-fix-hlc |
| G-03 | bug | No groups / WAL layer-2 crash matrix → E4 not closable | M1-e4 |
| G-04 | bug | Node tombstone only; E9/H3 edges missing | M1-e9 |
| G-05 | bug | Plaintext `serve` — fail-closed non-loopback | **done** M1-fix-serve (still plaintext by design) |
| G-06 | bug | No CRDT type pin / deps policy under multi-writer | M1-schema |
| G-07 | suggestion | `storage_format_version=1` written; freeze still open | **partial** M1-fmtver |
| G-08 | suggestion | SPEC M1 CLI: schema apply, repl, query missing | M1-schema, M1-query |
| G-09 | suggestion | E1/E2 store harness (50-todo + equal-ts) | **done** M1-e1, M1-e2-store |
| G-10 | suggestion | Dead `HelloOk.need` (one-way pull) | document in M1-LOCAL (done); drop or implement later |
| G-11 | suggestion | Signing seed plaintext in SQLite | M1-LOCAL threat note (done); optional wrap later |
| G-12 | suggestion | Rematerialize scans all kind=3 ops | defer until scale pressure |
| G-13 | suggestion | Local genesis ≠ AUTH genesis | M1-LOCAL (done); bridge at M3b |
| G-14 | suggestion | TS→IR compiler absent | M1-tsir |

---

## Intentionally out of M1 product scope (labeled experimental)

- Plaintext TCP multi-machine exchange (LAN only, disposable DBs).
- AUTH membership enforcement on ingest (M3b).
- Format freeze of JSON bundle / Hello / SQLite layout.

---

## M1 exit gap (summary)

| Criterion | Status |
|-----------|--------|
| Local SQLite + signed props CRDTs | partial (experimental) |
| Ingress / arrival-order hardening | met for current path |
| E1 full / E2 store equal-ts | partial / missing |
| E4 groups + crash injection | missing |
| E9 / H3 | missing |
| Schema + query + TS→IR | missing |
| DQ-7 backend HLC | met (open/replay) |
| LEDGER M1-exit | blocked on above |

Full narrative review was reduced to this index on 2026-07-19 plan cleanup. For code cites, use LEDGER row evidence and `git show f9cc660`.
