# ZeroDB

Offline-first, peer-to-peer, CRDT-powered **property graph database** — a successor to GunDB that keeps its zero-config, local-first developer experience while addressing necessary improvements (wall-clock conflict resolution, no oplog, JS-only core, LWW-everything).

**Status:** **Composite M0 contract-model gate closed 2026-07-18** (C1–C5, C7–C8; C6 deferred). Draft-1 profiles only — **no wire/persistent format freeze** until an explicit freeze Decision Log entry. Two-language conformance corpus **103** required vectors, CI-blocking. Next: **M1** (`v0.1.0-local`). TS→IR compiler trails ≤ M1.
`zerodb-core` / `zerodb-storage` crates remain **experimental** until a format freeze; M1 may begin on the model contracts.


## Documents

| Doc | What it is |
|-----|------------|
| [Technical Specification](doc/SPEC.md) | Core architecture, data model, CRDT type system, sync, storage, security — and the milestone roadmap (§10) |
| [Kernel Specification](doc/KERNEL.md) | M0a contract (resolved): operation algebra, canonical encoding, preimages, HLC state machine, CRDT semantic kernel |
| [Schema Specification](doc/SCHEMA.md) | M0b contract (draft-1): schema IR, epochs, migration DSL, v0.1 query subset |
| [Authorization Specification](doc/AUTH.md) | M0d contract (draft-1): identity, genesis, membership, authz predicate |
| [WAL Specification](doc/WAL.md) | M0e.1 (draft-1): WAL crash model, group seal (C8) |
| [Delivery Specification](doc/DELIVERY.md) | M0e.2 (draft-1): delivery, anti-replay, resume (H4/H11) |
| [Version Policy](doc/VERSIONS.md) | M0e.3 (draft-1): version namespaces + decode limits |
| [Frontier Specification](doc/FRONTIER.md) | M0f (draft-1): frontiers, snapshots; GC disabled (C7) |
| [Merkle Specification](doc/MERKLE.md) | M0c (draft-1): canonical sync tree + mismatch walk (C3) |
| [Relay Protocol Specification](doc/RELAY-SPEC.md) | Draft wire protocol for relay servers (not yet implementation-ready — see ISSUES) |
| [Issues & Decisions](doc/ISSUES.md) | Tracked specification issues (C/H/O IDs), M0 package map, and the decision log |
| [Exemplar](doc/EXEMPLAR.md) | Distributed ToDo app used as the end-to-end acceptance target |
| [Execution Plan](plan/PLAN.md) | Path-to-MVP delivery status: blocker rollup, M0 package status, decision queue (live tracking in [LEDGER.md](plan/LEDGER.md)) |
| [Delivery Ledger](plan/LEDGER.md) | Canonical work tracker — per-item status, dependencies, exit evidence, resolved-issue audit trail |
| [Findings (Codex)](plan/FINDINGS.CODEX.md) | Historical review that drove the P0/M0 corrections, now executed (2026-07-16) |
| [Findings (Grok)](plan/FINDINGS.GROK.md) | Historical review that motivated the M0 package-split (2026-07-15) |

## v0.1 scope

- **Runtime:** Rust core + SQLite + CLI (M1 → `v0.1.0-local`); Node/NAPI TypeScript SDK (M2 → `v0.1.0-sdk`); first multi-peer secure product at M3 (`v0.1.0`); browser/WASM later (M4)
- **Trust model:** mandatory Ed25519 operation signatures + datastore-membership capabilities
- **Non-goals for v0.1:** entity-level distributed ACLs, mobile bindings, Richtext, hosted relay
- **GunDB migration tooling: won't do** (clean break — "successor to GunDB" means the developer experience, not data portability)

## Roadmap (SPEC §10)

- **M0** — executable contracts as packages **(composite exit 2026-07-18)**:
  - **M0a–M0f** closed at contract-model layer (C1–C5, C7–C8; C6 deferred)
  - Draft-1 profiles; format freeze still requires an explicit Decision Log freeze
  - **103** two-language conformance vectors CI-blocking
- **M1** — local durable core: Rust + SQLite + CLI (`v0.1.0-local`)
- **M2** — Node/NAPI TypeScript SDK vertical, byte-identical to the core (`v0.1.0-sdk`)
- **M3** — secure multi-peer sync in three gates: **M3a** durable convergence (L2 relay + offline catch-up), **M3b** security (signatures, admission, E2E encryption), **M3c** interop TS wire peer + release (`v0.1.0`)
- **M4** — browser storage, WebRTC P2P, cross-peer schema migration, snapshot bootstrap
- **M5** — production readiness & GA: compaction/GC, backup/restore, fuzzing, Lean 4 proofs, external audit
- **M6** — ecosystem: mobile/Flutter bindings over a shared C ABI, plugins, hosted relay, tooling

## Contributing

Start with [ISSUES.md](doc/ISSUES.md).
Composite M0 model contracts are closed; highest-value work is **M1** (SQLite durable core) and M3 wire enforcement. Format freezes still need an explicit Decision Log freeze.
