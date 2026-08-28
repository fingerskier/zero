# ZeroDB

Offline-first, peer-to-peer, CRDT-powered **property graph database** — a successor to GunDB that keeps its zero-config, local-first developer experience while addressing necessary improvements (wall-clock conflict resolution, no oplog, JS-only core, LWW-everything).

**Status:** M0 contracts, M1 local core, M2 Node SDK, and experimental **M3a durable convergence are complete**. M3a includes the L2 relay/client, frozen-snapshot Merkle mismatch walk, and full 1,000-write three-peer E3 relay-only catch-up. Current gate: **M3b security**. **Browser peer (experimental):** `zerodb-wasm` in-memory store + WS sync driver. Conformance **115** vectors CI-blocking. *All formats draft-1, unfrozen until an explicit Decision Log freeze names a versioned profile.*


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
| [Relay Protocol Specification](doc/RELAY-SPEC.md) | Implemented experimental L2 wire protocol; security remains M3b |
| [Issues & Decisions](doc/ISSUES.md) | Tracked specification issues (C/H/O IDs), M0 package map, and the decision log |
| [Exemplar](doc/EXEMPLAR.md) | Distributed ToDo app used as the end-to-end acceptance target |
| [M1 local store](doc/M1-LOCAL.md) | Experimental M1 SQLite store + peer exchange (`v0.1.0-local`; not a format freeze) |
| [M1 LAN test](doc/M1-LAN-TEST.md) | Windows ↔ Pi trusted-LAN runbook for experimental TCP path |
| [Execution Plan](plan/PLAN.md) | Live path-to-MVP delivery plan |
| [Delivery Ledger](plan/LEDGER.md) | Live work tracker — M1 subtasks, post-M1 milestones |
| [Performance review](plan/PERF.md) | Static review + Stage 0+1 / pinned Stage 2/3 disposition (not a benchmark report) |
| [Archived reviews](plan/archive/) | Historical July 2026 FINDINGS (not the live backlog) |
| [Node SDK (M2)](zerodb-napi/) | Experimental `@zerodb/node` NAPI binding — `npm install && npm run build && npm test` |
| [TS→IR (M1)](tools/ts-to-ir/) | Minimal authoring JSON → schema pin IR for `schema-apply` |

## v0.1 scope

- **Runtime:** Rust core + SQLite + CLI (M1 → `v0.1.0-local`); Node/NAPI TypeScript SDK (M2 → `v0.1.0-sdk`); first multi-peer secure product at M3 (`v0.1.0`); browser/WASM later (M4)
- **Trust model:** mandatory Ed25519 operation signatures + datastore-membership capabilities
- **Non-goals for v0.1:** entity-level distributed ACLs, mobile bindings, Richtext, hosted relay
- **GunDB migration tooling: won't do** (clean break — "successor to GunDB" means the developer experience, not data portability)

## Roadmap (SPEC §10)

- **M0** — executable contracts as packages **(composite exit 2026-07-18)**:
  - **M0a–M0f** closed at contract-model layer (C1–C5, C7–C8; C6 deferred)
  - Draft-1 profiles; format freeze still requires an explicit Decision Log freeze
  - **115** two-language conformance vectors CI-blocking
- **M1** — local durable core: Rust + SQLite + CLI (`v0.1.0-local`)
- **M2** — Node/NAPI TypeScript SDK vertical, byte-identical to the core (`v0.1.0-sdk`)
- **M3** — secure multi-peer sync in three gates: **M3a durable convergence done**, **M3b** security (signatures, admission, E2E encryption), **M3c** interop TS wire peer + release (`v0.1.0`)
- **M4** — browser storage, WebRTC P2P, cross-peer schema migration, snapshot bootstrap
- **M5** — production readiness & GA: compaction/GC, backup/restore, fuzzing, Lean 4 proofs, external audit
- **M6** — ecosystem: mobile/Flutter bindings over a shared C ABI, plugins, hosted relay, tooling

## Local MVP (M1 slice) — multi-process / multi-machine

```bash
cargo build -p zerodb-cli
# Peer A
./target/debug/zerodb init --path ./a.sqlite
NODE=$(./target/debug/zerodb create-node --path ./a.sqlite --label Todo)
./target/debug/zerodb set --path ./a.sqlite --node $NODE --key title --value "milk"

# Peer B (empty DB adopts A's datastore id on first import)
./target/debug/zerodb init --path ./b.sqlite
./target/debug/zerodb export --path ./a.sqlite --out ./bundle.json
./target/debug/zerodb import --path ./b.sqlite --file ./bundle.json

# Concurrent edits then two-way merge (same machine, two DB files / processes)
./target/debug/zerodb set --path ./b.sqlite --node $NODE --key title --value "oat"
./target/debug/zerodb sync --path ./a.sqlite --peer ./b.sqlite

# Multi-machine (or second process): bind only the serving host's private LAN IP
./target/debug/zerodb serve --path ./a.sqlite --listen 192.168.1.12:7700 --allow-insecure-lan
# on other process/host:
./target/debug/zerodb pull --path ./b.sqlite --from 192.168.1.12:7700
```

`pull` is one-way; reverse the serving and pulling hosts to exchange writes in both directions. The M1 TCP transport is plaintext and has no peer authentication, so use it only with disposable test data on a trusted private LAN. Non-loopback binds require `--allow-insecure-lan`. Do not bind it to a wildcard or public interface. See the [Windows ↔ Raspberry Pi test runbook](doc/M1-LAN-TEST.md) for the complete two-machine acceptance procedure.

Smoke test: `powershell -File scripts/test-mvp.ps1`  
Ops are signed Ed25519; LWW merge uses the KERNEL §4.5 total order. This is an **experimental M1 path** toward `v0.1.0-local`, not a format freeze.

## Contributing

Start with [ISSUES.md](doc/ISSUES.md).
M3a durable convergence has exited experimentally. M3b-sig, AUTH membership + E5, E7 remainder, E8 clock quarantine (H1 closed), and E6 encrypted notes (I-10 live; KERNEL §7 on persist/ingest) are on. Current work is perf/DX Stage 0+1, then M3c (signed `SchemaEpoch` on that list). Pinned: Stage 2/3, H10 leftovers, handshake/TLS beyond advertised-limit enforcement, two-key split. All formats remain draft-1 and unfrozen. Do not claim M3b exit.
