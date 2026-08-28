# ZeroDB performance review

**Review date:** 2026-08-27  
**Scope:** written against `bec091c`; current `main` is `979ac65` (set-before-create follow-on). Local SQLite/materialization/query paths, direct peer sync, relay sync, and relay persistence.  
**Status:** static code review, not a benchmark report. The repository has strong conformance and end-to-end coverage, but no repeatable performance benchmark suite yet. Claims below distinguish observed algorithmic work from hypotheses that still need measurement.

## Disposition

**Stage 0+1** is current work (this PR): small deterministic fixtures/phase counters, advertised relay limit enforcement, prove import≡replay then drop redundant push `replay_all`, batched relay SQLite inserts, request-id-bound NAPI drain, clone-free batch chunking, cheap order indexes + bulk property reads, and one ordered export in relay client sync.

**Stage 2** is pinned (targeted projections): derived `op_targets` columns, AUTH control projection, single-pass replay rewrite, persisted CRDT accumulators.

**Stage 3** is pinned (bounded reconciliation): replace full OpId manifests, missing-only relay upload protocol, compact Merkle snapshot cache (beyond the free one-export win in Stage 1).

**H10 leftovers** stay pinned (offline revoke, two-key wrap, wrap-body freeze). Handshake/TLS/resource hardening beyond advertised-limit enforcement, connection quotas, datastore-creation policy, and two-key principal/device split stay pinned.

No Stage 2 derived-column rewrite. No Stage 3 new sync protocol. **Not claimed:** M3b exit, format freeze, H10 closed. Do not invent benchmark numbers here.

## Executive summary

Yes—there is substantial room to improve both data wrangling and over-the-wire behavior.

The largest local cost is that projection updates repeatedly scan and JSON-decode broad sections of the oplog. A write to one property scans all property operations twice; replay repeats broad scans for every distinct node, edge, and property. AUTH evaluation also repeatedly reloads and decodes the full wire history. These paths will grow poorly with history and can make imports/pushes superlinear.

The largest wire cost is full-history reconciliation. Direct peer sync sends every OpId as hex JSON. Relay sync uploads every local operation on every connection—even when the relay already has all of them—and then performs another full export to construct the frontier. Relay-side Merkle walking is logically sound, but each node or leaf request rebuilds the complete tree, while each session retains a full-body frozen snapshot.

Before major redesign, add deterministic scaling fixtures and phase counters. Several low-risk wins can then land first: remove redundant replay after import (after proving equivalence), enforce the relay limits it advertises, batch relay SQLite inserts, eliminate repeated whole-frame cloning during chunking, bulk-load properties/ops, and cache one immutable Merkle tree per datastore generation.

## Priority findings

| Priority | Finding | Impact | Confidence |
|---|---|---:|---:|
| P0 | Targeted writes and replay repeatedly scan/parse broad oplog sets | Critical at scale | High |
| P0 | Relay limits are advertised but not enforced; expensive work is globally serialized | Availability / scale blocker | High |
| P0 | Direct and relay sync perform full-history upload/manifests for small or zero deltas | Critical at scale | High |
| P0 | Relay Merkle walk rebuilds the whole tree per request and retains full-body snapshots | Critical at scale | High |
| P1 | Push sync calls full `replay_all` after an import that already materialized accepted ops | High | High |
| P1 | AUTH repeatedly reloads and transforms complete operation history | High | High |
| P1 | Query graph construction is N+1 SQL and fully in-memory | High for reads | High |
| P1 | Relay SQLite does one duplicate query and autocommit insert per op | High under concurrency | High |
| P2 | Batch sizing repeatedly clones and re-encodes growing CBOR vectors | Medium | High |
| P2 | JSON/hex and duplicate `body_json` + `wire_json` amplify bytes and allocations | Medium; measure first | Medium |

## Data-wrangling findings

### P0 — Projection maintenance is broad-scan rather than target-indexed

Observed in `zerodb-storage/src/lib.rs`:

- `rematerialize_node` calls `op_scan_node_kinds()` and JSON-decodes every create/tombstone body for one node (`rematerialize_node`).
- `rematerialize_edge` calls `op_scan()` and JSON-decodes the oplog for one edge (`rematerialize_edge`).
- `rematerialize_prop` calls `load_prop_ops`, which scans and parses every property operation, then calls `infer_crdt_from_ops`, which scans and parses the property history again (`rematerialize_prop`, `load_prop_ops`, `infer_crdt_from_ops`).
- `apply_wire` invokes one of these rematerializers after each accepted op.
- `replay_all` first discovers unique targets, then invokes the broad-scan rematerializers once per target.

The SQLite `ops` table stores target fields only inside `body_json` (`zerodb-storage/src/sqlite_backend.rs`). It has no derived `entity`, `path`, or edge target columns, so SQL cannot fetch only the history relevant to one projection. The scan queries also order by timestamp without supporting indexes beyond the `id` primary key.

**Expected behavior:**

- A hot property write grows at least linearly with all property history, not merely that property's history.
- Replaying many distinct targets multiplies broad scans and can approach quadratic work.
- JSON parsing and SQLite temporary sorting add substantial constants.

**Recommended direction:**

1. Add validated, derived target columns or a normalized `op_targets` table populated only after wire validation. Never treat these columns as signed authority.
2. Index target and deterministic order, e.g. `(kind, entity, path, physical_ms, logical, id)` and equivalent node/edge keys.
3. Make rematerializers query only one target's history.
4. Rewrite replay as one ordered scan that groups/reduces each target once.
5. Keep full replay as the correctness oracle; consider persisted incremental CRDT accumulators only if targeted-history reduction remains too slow.

Stage 1 adds only supporting *order* indexes (not derived target columns). Targeted-history rematerialization remains Stage 2.

### P1 — AUTH work scales with all operations, not control history

`load_applied_wires` loads and JSON-decodes every `wire_json` row (`zerodb-storage/src/authz.rs`). `authorize_wire` then converts the applied vector to AUTH representations for each candidate. This occurs in local commit, single-op ingest, bundle import, atomic batches, quarantine release, and key-record handling (`zerodb-storage/src/lib.rs`).

For a bundle of `m` candidates over `n` prior ops, repeated history conversion can dominate signature and SQLite work. Non-control application history unnecessarily inflates authorization cost.

**Recommended direction:** maintain a replay-verifiable AUTH projection over genesis/grant/revoke/key control operations, with indexes by subject, grant ID, datastore, scope, expiry, and revocation. Build the authorization view once per transaction/batch, then apply candidates incrementally in deterministic order.

Pinned for Stage 2.

### P1 — Query assembly performs N+1 SQL and full graph materialization

`LocalStore::to_query_graph` lists all nodes, then calls `prop_list` once per live node before loading all visible edges. NAPI `list_nodes` has the same shape. Query evaluation then operates over in-memory vectors (`zerodb-storage/src/lib.rs`, `zerodb-napi/src/lib.rs`, `zerodb-core/src/queryeval.rs`).

**First improvements:**

- Add a bulk property read for all requested/live node IDs.
- Build node/edge ID maps once instead of repeated vector searches.
- Apply label filtering and result limits before parsing every property where semantics permit.
- Defer a full SQL query planner until measurements show the simpler bulk-load path is insufficient.

Stage 1 lands the bulk property read and ID maps. Label/limit pushdown stays deferred.

### P1 — Import/push does redundant full replay

`import_bundle` validates, inserts, and rematerializes each accepted operation through `apply_wire`. The persistent direct-sync paths then call `replay_all` whenever any op was accepted (`zerodb-storage/src/sync.rs`: `serve_push`, `pull_push`, and push-loop ingress).

This appears redundant and converts every non-empty push into a whole-database rebuild. Before removal, add a property-based/equivalence test proving that `import_bundle` alone produces the same nodes, edges, properties, AUTH state, encryption state, quarantine, and HLC as import followed by replay—across arrival permutations and datastore adoption.

Stage 1 proves equivalence and removes the success-path `replay_all`. `replay_all` remains the oracle and recovery API.

### P2 — Open/query paths need supporting indexes and bulk APIs

`op_max_hlc` orders the oplog descending on every open, while scan/export methods order by `(physical_ms, logical, id)`. Add a supporting order index after checking `EXPLAIN QUERY PLAN`. `export_ops_by_id` currently performs one lookup and JSON parse per requested ID; use a bounded bulk query or temporary ID table for large deltas.

Stage 1 adds `ops_order` / `ops_kind_order` after checking the plans below. Bulk `export_ops_by_id` stays deferred (not required for the surgical list).

#### EXPLAIN QUERY PLAN (after Stage 1 indexes)

Paste of actual `EXPLAIN QUERY PLAN` output lives in `zerodb-storage/tests/perf_s0.rs` (not timings). Recorded after `ops_order` / `ops_kind_order` migrate on an empty store.

## Over-the-wire findings

### P0 — Direct peer sync sends a full OpId manifest

Protocol v2 `Hello` carries every OpId as a hex string in JSON (`zerodb-storage/src/sync.rs`). Both peers load complete ID sets and compute differences. Persistent push repeats `list_op_ids` and a complete set difference whenever the local generation changes.

A one-op delta therefore costs CPU and bytes proportional to complete history. The generic 64 MiB frame ceiling also becomes a hard history ceiling for the manifest before payload transfer is considered.

**Recommended direction:** replace full manifests with bounded reconciliation:

1. exchange datastore generation/root plus a compact frontier or bucket manifest;
2. descend only mismatched Merkle/range nodes;
3. page leaf IDs and deltas;
4. stream bounded op batches with explicit continuation, ACK, and backpressure.

Until that protocol lands, cap manifest IDs separately from frame bytes and send IDs/ops in bounded pages.

Pinned for Stage 3.

### P0 — Relay sync uploads all local operations before reconciliation

`relay_client::sync` calls `export_all`, filters local ops, converts every op to a relay CBOR wrapper, and submits all of them on every connection. An equal replica receives only duplicate ACKs. The client then computes roots/frontier from the same history; `local_frontier` calls `export_all` again (`zerodb-storage/src/relay_client.rs`).

The current Merkle walk optimizes relay-to-client catch-up only. It does not optimize client-to-relay upload.

**Recommended direction:** make reconciliation symmetric. Exchange roots first, walk differences, then upload and download only missing IDs. At minimum, avoid the second full export by deriving the frontier and local Merkle input during the first ordered scan.

Stage 1 does the one-export derivation. Missing-only upload is Stage 3.

### P0 — Relay Merkle walking rebuilds whole-dataset state per round trip

`RelaySession::on_sync` loads every full `StoredOp`, builds a tree, and stores the full operation vector as the session's frozen walk snapshot. `on_merkle_node` and `on_merkle_leaf` rebuild the complete tree from that vector for every request (`zerodb-relay/src/session.rs`). `on_delta` scans the frozen vector and clones matched bodies.

Frozen snapshot semantics are correct and must be preserved, but the representation is expensive:

- CPU scales approximately with `whole tree × walk requests`.
- memory scales with `full history bodies × sessions × active datastores`.
- dense one-minute buckets can produce very large leaf ID responses.

**Recommended direction:** cache an immutable compact snapshot by `(datastore, generation, validated_root)`, containing the built tree, per-leaf IDs, and an ID-to-body lookup handle. Sessions retain a bounded reference, not cloned bodies. Add walk lifetime, count, snapshot-byte, leaf-page, and delta-ID limits. Cache invalidation must be generation/root based, not datastore-only.

Pinned for Stage 3.

### P0 — Relay resource limits are promises, not enforcement

WELCOME advertises payload, batch, subscription, operation-rate, and byte-rate limits (`default_limits` in `zerodb-relay/src/session.rs`), but `handle` decodes the full CBOR frame before a raw size check and `on_ops` does not enforce the advertised op/byte limits. The WebSocket server uses default acceptance without explicit application quotas (`zerodb-relay/src/bin/zerodb-relay.rs`).

This is both a performance and availability issue: any self-authenticated peer can submit expensive frames, hold the global relay mutex during admission, create many grantless datastore IDs, and consume durable storage.

**Required before network exposure:**

- reject oversized WebSocket messages before CBOR decode;
- enforce per-op, per-batch, subscription, rate, delta-ID, active-walk, and response limits;
- add connection/read/idle timeouts;
- define datastore creation/admission and per-principal, per-datastore, and global op/byte quotas;
- return overload/quota errors before acquiring the database lock where possible.

Stage 1 enforces advertised payload/batch limits (`0x303 PAYLOAD_TOO_LARGE`) before full decode / before insert. Connection quotas, TLS, rate-limit accounting, subscription caps, and datastore-creation policy stay pinned.

### P1 — Relay admission serializes all clients and autocommits each op

A single `Arc<Mutex<Inner>>` protects the store and subscriber state. `on_ops` holds it across authorization queries and SQLite work for the whole client-controlled batch. `SqliteStore::insert` performs `SELECT` then `INSERT` per op, with an autocommit each time and no relay WAL/busy configuration (`zerodb-relay/src/session.rs`, `zerodb-relay/src/store.rs`).

**Low-risk first step:** enforce small batch bounds, validate safe structural/cryptographic work outside the lock, then insert an admitted batch in one transaction using `INSERT ... ON CONFLICT DO NOTHING`. Keep grant evaluation and grant-op application atomically ordered. Do not simply remove the mutex; that risks authorization races.

Stage 1 batches inserts in one transaction and keeps the session mutex.

### P1 — NAPI relay response collection adds fixed latency

`collect_ws_replies` waits for replies and then uses a 200 ms read timeout as end-of-response signaling (`zerodb-napi/src/lib.rs`). The relay client invokes this abstraction for handshake messages, every upload batch, every serialized Merkle node/leaf request, and delta requests. Fixed drain waits can compound with walk depth.

Replace timeout-based completion with request-ID-bound protocol completion: known single-response types, `remaining` on paged responses, or an explicit end marker. Preserve timeouts as failure bounds, not normal framing.

### P2 — Batch sizing is allocation-amplifying

Client and relay chunkers repeatedly clone the growing CBOR vector and encode a complete trial envelope after every appended operation (`split_ops_batches` in `zerodb-storage/src/relay_client.rs`; `chunk_ops_frames` and `chunk_delta_frames` in `zerodb-relay/src/session.rs`). This produces quadratic copying/encoding within a batch.

Encode each op once, track exact encoded size plus envelope overhead, and append without cloning previous members. Add a single-op oversize rejection and tests at every boundary.

## Measurement plan

Performance changes should be accepted against deterministic fixtures, not intuition. Run both SQLite and memory backends where meaningful.

Stage 0 in this PR is a *minimum* fixture (1k ops, phase counters/timings printed by `perf_s0`), not a 100k farm. Do not treat those prints as published numbers.

### Local storage

1. **Hot write:** append to one property at 1k, 10k, and 100k prior ops.
2. **Wide write:** same totals spread across distinct `(entity, path)` targets.
3. **Replay:** vary ops and unique targets independently.
4. **AUTH:** vary total app ops and control ops independently; ingest one authorized op and one batch.
5. **Query:** vary nodes, properties/node, and edge density.

Capture p50/p95/p99 latency, SQLite statement count, rows scanned, JSON bytes parsed, transaction duration, allocations/peak RSS, and WAL/database growth. Record `EXPLAIN QUERY PLAN` for every hot query.

### Direct sync

Measure equal replicas, one-op delta, 1% divergence, and cold join at increasing histories. Capture bytes and frames each direction, IDs transmitted, diff CPU, export/import time, replay time, store-lock hold time, and peak RSS.

### Relay

Measure equal replicas, missing upload only, sparse missing buckets, dense same-bucket divergence, and cold join. Capture upload duplicate ratio, request count, drain-wait time, Merkle builds, snapshot bytes, delta bytes, SQLite commits, relay-lock wait/hold time, and end-to-end latency.

Run 1/4/16/64 concurrent cooperative and adversarial clients. Report legitimate-client tail latency alongside aggregate throughput. Boundary tests must prove over-limit input is rejected before expensive work and causes zero durable writes.

## Delivery plan

### Stage 0 — Baselines and containment

- Add benchmark fixtures and phase counters for validation, AUTH, scans, materialization, replay, export, diff, transport waits, Merkle build/walk, relay lock, and SQLite transaction time.
- Enforce raw transport and advertised semantic limits.
- Add datastore/storage quotas before exposing the relay beyond trusted development use.
- Add `EXPLAIN QUERY PLAN` checks for hot SQLite queries.

**Exit:** repeatable results for the scenarios above; limits demonstrably reject before expensive work.

**This PR (minimum):** 1k-op hot write / import / replay / one AUTH ingest counters; advertised payload/batch reject tests; EXPLAIN notes for the new order indexes. Quotas and the 100k farm stay pinned.

### Stage 1 — Surgical wins

- Prove import/materialization equivalence, then remove redundant push `replay_all`.
- Batch relay inserts transactionally with conflict-aware dedup.
- Replace timeout-based relay response completion.
- Eliminate repeated trial-frame cloning/encoding.
- Add ordered/kind indexes, bulk property loading, bulk op export, and query ID maps.
- Avoid duplicate full exports in relay client sync.

**Exit:** no conformance regression; equal and one-op-delta workloads materially improve without protocol format changes.

**This PR:** all of the above except bulk `export_ops_by_id` (still N lookups; not required for the surgical list).

### Stage 2 — Targeted projections

- Add validated derived target indexes/columns.
- Fetch only relevant operation history for node/edge/property reduction.
- Build AUTH from an indexed control projection once per transaction.
- Make replay a single grouped/streamed reduction pass.
- Retain old full replay in tests as an oracle during migration.

**Exit:** hot-write latency scales with target history rather than whole oplog; replay is near-linear in rows plus reduction work.

**Pinned.** Trigger: measurements from Stage 0 fixtures show hot-write/replay still dominated by broad oplog scans after Stage 1.

### Stage 3 — Bounded reconciliation

- Replace full direct-sync manifests with root/frontier/Merkle or range reconciliation.
- Make relay upload missing-only.
- Page leaves and deltas; stream bounded operation batches with explicit continuation, ACK, and backpressure.
- Cache immutable compact relay trees/snapshots by generation/root and bound their lifetime/resources.

**Exit:** equal sync is approximately constant-size; one-op delta cost is logarithmic reconciliation plus one bounded op transfer; memory is bounded independently of untrusted request size.

**Pinned.** Trigger: equal-replica / one-op-delta wire bytes still scale with full history after Stage 1.

## Correctness constraints

Optimization must preserve:

- order-independent tombstones and CRDT convergence;
- canonical OpId/signature/datastore validation;
- AUTH grant/revoke ordering and transactional visibility;
- encrypted-property rematerialization after key arrival;
- HLC/quarantine semantics;
- atomic groups and rollback behavior;
- frozen-snapshot consistency during multi-request Merkle walks;
- deterministic replay equivalence.

The current scan-based implementation is expensive but straightforward. Derived indexes and caches are acceleration structures only; signed wire data and deterministic full replay remain the source of truth and recovery path.
