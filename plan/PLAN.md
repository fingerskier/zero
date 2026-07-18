# ZeroDB — Path-to-MVP Execution Plan

**Date:** 2026-07-16 (status refreshed 2026-07-18 — **composite M0 exit**)
**Status:** active delivery plan (pre-MVP); composite M0 contract-model gate closed
**Authority:** delivery/tracking plan only. [SPEC §10](../doc/SPEC.md) is the normative roadmap, [ISSUES.md](../doc/ISSUES.md) the normative issue ledger, [LEDGER.md](LEDGER.md) the live work tracker. Where this plan conflicts with SPEC, SPEC wins.

This plan was drafted from two independent reviews — [FINDINGS.GROK.md](FINDINGS.GROK.md) (2026-07-15) and [FINDINGS.CODEX.md](FINDINGS.CODEX.md) (2026-07-16), both now **historical**. Their finding→work-item mapping has been executed; live status is in [LEDGER.md](LEDGER.md).

---

## 1. Verdict & release meaning

Both reviews agreed and it still holds: the M0→M6 sequencing is sound; the project is ready for M0 contract work but **not** for a format freeze, product implementation, or MVP release until composite M0 exits.

| Term | Definition |
|------|------------|
| **MVP** | `v0.1.0-local` — M1 exit: offline single-peer Rust core + SQLite + CLI |
| **First shippable product** | `v0.1.0` — M3c exit: secure multi-peer sync with offline catch-up |

## 2. Blocker status

The eight blockers that gated starting the MVP path — **all cleared** (CX-02 closed with M0d 2026-07-18):

| # | Blocker | Status |
|---|---------|--------|
| 1 | CX-01 semantic kernel unowned | **done** — M0a / [KERNEL.md](../doc/KERNEL.md) |
| 2 | CX-02 datastore membership relay-checked only | **done** — M0d / [AUTH.md](../doc/AUTH.md) (C4 admission + C5; on-wire M3b) |
| 3 | CX-03 gate circularity | **done** — P0-6, SPEC §10 two-layer rule |
| 4 | CX-04 no offline catch-up in v0.1 | **resolved (design)** — M3a L2 relay required before `v0.1.0` (SPEC §10; DQ-9) |
| 5 | CX-05 freeze before format decisions | **done** — envelope/blob decided in M0a (DQ-5/DQ-6) |
| 6 | CX-06 acceptance artifacts missing | **done** — INVARIANTS, EXEMPLAR, `conformance/` + CI (P0-2/3/5) |
| 7 | CX-07 HLC durability | **done** — DQ-7, KERNEL §5 |
| 8 | A-01 workspace doesn't compile | **done** — P0-1 green baseline |

## 3. P0 readiness — complete

All P0 items (green baseline, INVARIANTS, EXEMPLAR, ledger, `conformance/`+CI, gate language, doc hygiene) are **done**. Evidence in [LEDGER.md](LEDGER.md) P0 table. This section is retained only as an index; no P0 work remains.

## 4. M0 — executable contracts

Package structure and normative content live in [SPEC §10](../doc/SPEC.md); per-package status in [LEDGER.md](LEDGER.md). Snapshot:

| Package | Owns | Status |
|---------|------|--------|
| M0a | operation kernel, encoding, HLC, CRDT semantics | **done** — [KERNEL.md](../doc/KERNEL.md), 24-vector corpus |
| M0b | schema IR, epochs, migration DSL, query subset | **done** — [SCHEMA.md](../doc/SCHEMA.md), 57-vector corpus (C2 2026-07-18); TS→IR ≤ M1 |
| M0c | Merkle tree + sync state machine | **done** — [MERKLE.md](../doc/MERKLE.md) (C3) |
| M0d | datastore / identity / authorization | **done** — [AUTH.md](../doc/AUTH.md) (C4+C5) |
| M0e | groups/WAL, delivery/resume, versions | **done** — WAL / DELIVERY / VERSIONS (C8, H4, H7, H11) |
| M0f | frontiers, checkpoints, snapshots | **done** — [FRONTIER.md](../doc/FRONTIER.md) (C7/O7); GC disabled |
| M0 composite | cross-package fixtures | **done** — COMP-001; 103 vectors; draft-1 only |

Dependency order (SPEC §10): `M0a → {M0b, M0c, M0d, M0e}`; `M0c + M0e.2 → M0f`; composite → M1.

## 5. Post-M0 path

Milestone splits (M3a/b/c, M4a/b, M5a/b/c) are now normative in [SPEC §10](../doc/SPEC.md) — promoted there 2026-07-18. Delivery tracking in [LEDGER.md](LEDGER.md) "Post-M0" table. Key gates:

- **M1** `v0.1.0-local` (MVP): Rust + SQLite + CLI, atomic oplog/state/HLC with crash injection, E1/E2/E4/E9.
- **M2** `v0.1.0-sdk`: Node/NAPI vertical; TS model runner stays separate, evolving toward the M3c wire peer.
- **M3a→b→c** `v0.1.0`: durable L2 relay + offline catch-up → security → interop TS wire peer. If the L2 relay can't ship, M3 becomes an online-sync preview, **not** the first offline-first release.
- **M4a/b**, **M5a/b/c**, **M6**: platform / evolution, GA program, ecosystem.

## 6. Decision queue

DQ-1..DQ-12 defined below; resolution tracked in [LEDGER.md](LEDGER.md). A decision closes only via the [SPEC §10](../doc/SPEC.md) approved-resolution checklist.

| ID | Decision | Blocks | Status |
|----|----------|--------|--------|
| DQ-1 | Identity: key, device, or principal-with-devices? | M0d | **resolved** — AUTH §1 |
| DQ-2 | Datastore genesis + root authority? | M0d | **resolved** — AUTH §2 |
| DQ-3 | Per-op author-membership + historical authorization? | M0d | **resolved** — AUTH §4 |
| DQ-4 | Executable model closing C8 without SQLite? | M0e.1 | direction ratified |
| DQ-5 | Encryption unit + frozen envelope bytes? | M0a/M0b | **resolved** — KERNEL §7 |
| DQ-6 | Extension/blob strategy? | M0a | **resolved** — KERNEL §8 |
| DQ-7 | Durable HLC across restart/restore/rollback? | M0a/M1 | **resolved** — KERNEL §5 (backend re-verified M1) |
| DQ-8 | Equal-timestamp equivocation vs tie-break? | M0a | **resolved** — KERNEL §4.5 |
| DQ-9 | L2 durable catch-up mandatory for v0.1? | M3a | plan default: **yes** |
| DQ-10 | `unique` removed from v0.1 profile? | M0b | plan default: **removed** (SCHEMA §2) |
| DQ-11 | Who approves resolutions; where records live? | — | plan default: **LEDGER** |
| DQ-12 | Team/capacity, DRIs, effort bands? | schedule | open |

DQ-1..DQ-4 gate M0d/M0e contract writing (directions in [DQ-PROPOSALS.md](DQ-PROPOSALS.md)). DQ-5..DQ-8 are closed and normative in KERNEL.

## 7. Ground rules (normative in SPEC §10 / ISSUES.md)

- **No wire or persistent format freeze before composite M0 exit**; freezes name a versioned profile.
- **GC disabled** until C7 partition/rejoin, forgotten-peer, late-op, and restore tests pass (M5b).
- **Pre-M0 implementation policy**: `zerodb-core`/`zerodb-storage` are experimental; their types are not normative.
- **Approved-resolution checklist** (SPEC §10) is the only way an issue closes.
- Lean proofs do not gate M0 or v0.1.

## 8. Immediate next actions (ordered)

1. **M1** — local durable core: Rust + SQLite + CLI (`v0.1.0-local`); layer-2 crash injection for WAL contracts; E1/E2/E4/E9.
2. **TS→IR compiler** (≤ M1) — standalone npm tool emitting SCHEMA §2 IR.
3. **M2** — Node/NAPI SDK vertical.
4. **M3a→b→c** — L2 relay, security, interop wire peer → `v0.1.0`.

**Composite M0 closed 2026-07-18** (contract-model layer). M1 may begin; format freezes still require an explicit Decision Log freeze naming a versioned profile.
