# ZeroDB

Offline-first, peer-to-peer, CRDT-powered **property graph database** — a successor to GunDB that keeps its zero-config, local-first developer experience while addressing necessary improvements (wall-clock conflict resolution, no oplog, JS-only core, LWW-everything).

**Status:** specification draft, pre-implementation.
Current work is **Milestone 0** (packages **M0a–M0f**) — stabilizing the operation format, schema epochs, Merkle sync, trust-model, delivery, and frontier contracts before any code freezes a wire or persistent format.
Existing `zerodb-core` / `zerodb-storage` crates are **experimental** until M0a golden vectors exist.

## Documents

| Doc | What it is |
|-----|------------|
| [Technical Specification](doc/SPEC.md) | Core architecture, data model, CRDT type system, sync, storage, security — and the milestone roadmap (§10) |
| [Kernel Specification](doc/KERNEL.md) | M0a contract draft: operation algebra, canonical encoding, preimages, HLC state machine, CRDT semantic kernel |
| [Relay Protocol Specification](doc/RELAY-SPEC.md) | Draft wire protocol for relay servers (not yet implementation-ready — see ISSUES) |
| [Issues & Decisions](doc/ISSUES.md) | Tracked specification issues (C/H/O IDs), M0 package map, and the decision log |
| [Exemplar](doc/EXEMPLAR.md) | Distributed ToDo app used as the end-to-end acceptance target |
| [Execution Plan](plan/PLAN.md) | Path-to-MVP delivery plan: P0 readiness package, revised M0 packages, decision queue |
| [Findings (Codex)](plan/FINDINGS.CODEX.md) | Specification & plan review driving the current P0/M0 corrections (2026-07-16) |
| [Findings (Grok)](plan/FINDINGS.GROK.md) | Historical plan review that motivated the M0 package-split (2026-07-15) |

## v0.1 scope

- **Runtime:** Rust core + SQLite + CLI (M1 → `v0.1.0-local`); Node/NAPI TypeScript SDK (M2 → `v0.1.0-sdk`); first multi-peer secure product at M3 (`v0.1.0`); browser/WASM later (M4)
- **Trust model:** mandatory Ed25519 operation signatures + datastore-membership capabilities
- **Non-goals for v0.1:** entity-level distributed ACLs, mobile bindings, Richtext, hosted relay
- **GunDB migration tooling: won't do** (clean break — "successor to GunDB" means the developer experience, not data portability)

## Roadmap (SPEC §10)

- **M0** — executable contracts as packages:
  - **M0a** — operation algebra & canonical encoding (C1, C4 context)
  - **M0b** — schema IR, epochs, migration DSL, minimal query (C2, O2, O3)
  - **M0c** — Merkle tree & sync state machine (C3)
  - **M0d** — author keys & datastore membership (C4 admission, C5)
  - **M0e** — groups, delivery, version policy (C8, H4, H7, …)
  - **M0f** — causal frontiers & snapshot contracts (C7, O7) — GC still disabled
  Exit: C1–C5, C7–C8 resolved with conformance fixtures; **no format freezes before composite M0**
- **M1** — local durable core: Rust + SQLite + CLI (`v0.1.0-local`)
- **M2** — Node/NAPI TypeScript SDK vertical, byte-identical to the core (`v0.1.0-sdk`)
- **M3** — secure multi-peer sync: signatures, admission, E2E encryption, reference relay + harness (`v0.1.0`)
- **M4** — browser storage, WebRTC P2P, cross-peer schema migration, snapshot bootstrap
- **M5** — production readiness & GA: compaction/GC, backup/restore, fuzzing, Lean 4 proofs, external audit
- **M6** — ecosystem: mobile/Flutter bindings over a shared C ABI, plugins, hosted relay, tooling

## Contributing

Start with [ISSUES.md](doc/ISSUES.md).
The Critical issues mapped to **M0a–M0f** must be resolved before any format freezes — design proposals against those package IDs are the highest-value contributions right now.
