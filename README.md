# ZeroDB

Offline-first, peer-to-peer, CRDT-powered **property graph database** — a successor to GunDB that keeps its zero-config, local-first developer experience while addressing necessary improvements (wall-clock conflict resolution, no oplog, JS-only core, LWW-everything).

**Status:** specification draft, pre-implementation.
Current work is **Milestone 0** — stabilizing the operation format, schema epochs, Merkle sync, and trust-model contracts before any code freezes a wire or persistent format.

## Documents

| Doc | What it is |
|-----|------------|
| [Technical Specification](doc/SPEC.md) | Core architecture, data model, CRDT type system, sync, storage, security — and the milestone roadmap (§10) |
| [Relay Protocol Specification](doc/RELAY-SPEC.md) | Draft wire protocol for relay servers (not yet implementation-ready — see ISSUES) |
| [Issues & Decisions](doc/ISSUES.md) | Tracked specification issues (C/H/O IDs) and the decision log |
| [Exemplar](doc/EXEMPLAR.md) | Distributed ToDo app used as the end-to-end acceptance target |

## v0.1 scope

- **Runtime:** Rust core + SQLite + CLI (M1); Node/NAPI TypeScript SDK follows (M2); browser/WASM later (M4)
- **Trust model:** mandatory Ed25519 operation signatures + datastore-membership capabilities
- **Non-goals for v0.1:** entity-level distributed ACLs, mobile bindings, Richtext, hosted relay, GunDB migration tooling

## Roadmap (SPEC §10)

- **M0** — decisions & executable contracts (canonical operation format, schema epochs, Merkle sync state machine, trust model); exit gate:  all Critical issues resolved with red conformance tests
- **M1** — local durable core:  Rust + SQLite + CLI runs the exemplar offline with deterministic restart/replay
- **M2** — Node/NAPI TypeScript SDK vertical, byte-identical to the core
- **M3** — secure multi-peer sync:  signatures, datastore admission, E2E encryption, reference relay + conformance harness
- **M4** — browser storage, WebRTC P2P, cross-peer schema migration, snapshot bootstrap
- **M5** — production readiness & GA:  compaction/GC, backup/restore, fuzzing, Lean 4 proofs, external audit
- **M6** — ecosystem:  mobile/Flutter bindings over a shared C ABI, plugins, hosted relay, tooling

## Contributing

Start with [ISSUES.md](doc/ISSUES.md).
The Critical (C-series) issues must be resolved before any format freezes — design proposals against those IDs are the highest-value contributions right now.
