# ZeroDB — Technical Specification

**Version:** 0.2.1-draft
**Date:** 2026-07-15
**Author:** Matt / Turing Automations
**Status:** Draft — seeking contributors and co-architects
**Normative authority:** This document is authoritative for core semantics; [RELAY-SPEC.md](RELAY-SPEC.md) is authoritative for the relay wire protocol.  Open specification issues are tracked by ID in [ISSUES.md](ISSUES.md); no wire or persistent format is frozen until the composite **M0** exit gate (§10) — packages **M0a–M0f**.

---

## 1. Executive Summary

ZeroDB is an offline-first, peer-to-peer, CRDT-powered property graph database designed to succeed GunDB.
It combines the accessibility of GunDB's vision — a database that runs anywhere with zero configuration — with the correctness guarantees that GunDB never delivered.

**The thesis:** GunDB proved the appetite for a zero-config, offline-first, decentralized database.
But its HAM conflict resolution algorithm is fundamentally broken under clock drift, it has no operation log for delta sync after offline periods, its JavaScript-only implementation prevents cross-platform correctness guarantees, and its everything-is-a-register CRDT model is too primitive for real applications.

ZeroDB fixes all of these while keeping what made GunDB compelling: instant local reads, automatic sync, and a developer experience that feels like a local database.

### Key Architectural Bets

| Decision | Replaces (GunDB) | Rationale |
|----------|-------------------|-----------|
| Hybrid Logical Clocks (HLC) | HAM (wall-clock timestamps) | Causality-preserving ordering without clock synchronization. A device with a skewed clock cannot block the network. |
| Oplog with causal graph + Merkle sync tree | No operation log | Enables efficient delta sync after arbitrary offline periods. Operations form a causal graph (for correctness); a derived Merkle sync tree enables O(log N) divergence detection. |
| Rust core / WASM bindings | Pure JavaScript | Identical behavior across browser, Node.js, mobile, and CLI. Follows Automerge and Loro's proven path. |
| Column-level CRDT selection | Everything is an LWW register | A `name` field is LWW, a `tags` field is an OR-Set, a `view_count` is a Counter. Schema declares intent; the engine enforces merge semantics. Inspired by CR-SQLite. |
| Lean formal proofs | No formal verification | Machine-checked proofs of CRDT convergence properties. The trust differentiator for safety-critical and financial applications. |

### Target Environments

- **Browser:** IndexedDB + OPFS storage, WASM core, WebSocket/WebRTC transport
- **Node.js:** SQLite storage, native Rust or WASM core, WebSocket transport
- **CLI:** First-class `zerodb` command for administration, migration, inspection, and scripting
- **Mobile (future):** Swift and Kotlin bindings from the Rust core via FFI

---

## 2. Architecture & Data Model

### 2.1 System Architecture

```
┌──────────────────────────────────────────────────────┐
│                   Application Layer                  │
│          TypeScript SDK  ·  React Hooks  ·  CLI      │
├──────────────────────────────────────────────────────┤
│                   Binding Layer                      │
│       WASM (browser)  ·  NAPI (Node)  ·  FFI (Swift) │
├──────────────────────────────────────────────────────┤
│                   ZeroDB Core (Rust)                 │
│  ┌────────┐ ┌──────────┐ ┌────────┐ ┌─────────────┐  │
│  │  HLC   │ │  CRDT    │ │ Merkle │ │   Crypto    │  │
│  │ Clock  │ │  Engine  │ │  Sync  │ │   Layer     │  │
│  └────────┘ └──────────┘ └────────┘ └─────────────┘  │
│  ┌────────────────────┐ ┌──────────────────────────┐ │
│  │    Oplog Store     │ │     State Materializer   │ │
│  └────────────────────┘ └──────────────────────────┘ │
├──────────────────────────────────────────────────────┤
│                   Storage Adapter                    │
│    IndexedDB  ·  OPFS  ·  SQLite  ·  (pluggable)     │
├──────────────────────────────────────────────────────┤
│                   Transport Layer                    │
│    WebSocket  ·  WebRTC  ·  (pluggable)              │
└──────────────────────────────────────────────────────┘
```

### 2.2 The Rust Core

The Rust core is the single source of truth for all CRDT logic, clock management, and merge semantics.
Every platform — browser, Node.js, CLI, mobile — runs identical compiled code.
This eliminates the class of bugs where "sync works differently on iOS than in Chrome" that plagues pure-JavaScript implementations.

**Crate structure:**

```
zerodb/
├── zerodb-core/          # CRDT engine, HLC, oplog, Merkle DAG
├── zerodb-storage/       # Storage adapter trait + implementations
│   ├── zerodb-idb/       # IndexedDB adapter (wasm-bindgen)
│   ├── zerodb-opfs/      # OPFS adapter (wasm-bindgen)
│   └── zerodb-sqlite/    # SQLite adapter (rusqlite)
├── zerodb-transport/     # Transport trait + implementations
│   ├── zerodb-ws/        # WebSocket
│   └── zerodb-webrtc/    # WebRTC (browser P2P)
├── zerodb-crypto/        # Signing, encryption, key management
├── zerodb-wasm/          # wasm-bindgen entry point
├── zerodb-napi/          # napi-rs entry point for Node.js
├── zerodb-cli/           # CLI binary
└── zerodb-proofs/        # Lean 4 formal proofs (separate build)
```

### 2.3 Property Graph Model

ZeroDB stores a **property graph**: nodes and edges, each carrying typed properties.
Unlike GunDB's flat key-value nodes with link references, ZeroDB edges are first-class citizens with their own properties and CRDT-managed fields.

#### Node

```typescript
interface Node {
  id: NodeId;             // UUIDv7 — sortable, globally unique, embeds timestamp (RFC 9562)
  label: string;          // e.g. "User", "Document", "Session"
  properties: Record<string, CRDTValue>;  // each property has a CRDT type
  _meta: {
    created: HLCTimestamp;
    updated: HLCTimestamp;
    tombstone: boolean;   // soft delete — GC'd after causal stability
    origin: PeerId;       // peer that created this node
  };
}
```

Materialized entities are multi-author aggregates and carry no signature of their own; signatures live on **operations** (§6.1), where authorship is well-defined.

#### Edge

```typescript
interface Edge {
  id: EdgeId;             // UUIDv7
  label: string;          // e.g. "FOLLOWS", "AUTHORED", "MEMBER_OF"
  source: NodeId;
  target: NodeId;
  properties: Record<string, CRDTValue>;
  _meta: {
    created: HLCTimestamp;
    updated: HLCTimestamp;
    tombstone: boolean;
    origin: PeerId;
  };
}
```

#### Why Property Graph (not GunDB-style)

GunDB represents everything as flat nodes with string-valued link properties.
This creates ambiguity: is `user.friends` a property or a relationship?
You can't attach metadata to relationships (e.g., "friended_at", "blocked").
And traversal requires chaining `.get()` calls that each trigger a separate async lookup.

A property graph makes relationships queryable and annotatable.
You can ask "find all FOLLOWS edges created after date X where blocked = false" in a single traversal, not a chain of reactive callbacks.

### 2.4 Hybrid Logical Clock (HLC)

Every operation in ZeroDB is timestamped with an HLC.  The HLC is a tuple:

```
(physical_time: u64, logical_counter: u16, peer_id: PeerId)
```

**Properties:**

- **Monotonic on a single peer:** Every timestamp from a given peer is strictly greater than the previous, even if the wall clock jumps backward.
- **Causal ordering across peers:** After peer A sends operations to peer B, all subsequent operations on B will have timestamps greater than A's sent operations.
- **Close to wall-clock time:** Unlike pure Lamport clocks, HLC timestamps are interpretable as approximate real time — useful for queries like "show me changes from the last hour."
- **Bounded drift:** If a peer's physical clock is wildly off, the HLC still advances but caps the logical counter to prevent runaway timestamps.  Peers with clocks skewed beyond a configurable threshold (default: 60 seconds) trigger a drift warning but are never blocked from writing.
- **Logical counter overflow:** The `u16` logical counter supports up to 65,535 operations per physical-clock millisecond.  If exhausted (extreme burst throughput), the peer advances `physical_time` by 1ms and resets the counter.  This preserves monotonicity at the cost of a slight forward drift.

**Comparison with GunDB's HAM:**

| Property | HAM | HLC |
|----------|-----|-----|
| Clock skew tolerance | Defers updates until local clock catches up — effectively blocks writes | Absorbs skew into logical counter; never blocks |
| Causal ordering | Not guaranteed across peers | Guaranteed after any message exchange |
| Tie-breaking | `JSON.stringify` lexicographic comparison | Deterministic: physical → logical → peer_id |
| Future-dated attacks | A peer can set clock to far future and block all others | Drift detection warns; logical counter caps prevent counter runaway.  A far-future `physical_time` can still win LWW conflicts until the acceptance/quarantine rule ships ([ISSUES H1](ISSUES.md)) |

### 2.5 Oplog & Causal Graph

Every mutation to the graph produces an **operation** appended to a local, append-only log:

```typescript
interface Operation {
  id: OpId;               // content hash (BLAKE3) of this operation — globally unique
  hlc: HLCTimestamp;      // HLC timestamp when this operation was created
  peer: PeerId;           // originating peer
  deps: OpId[];           // causal dependencies (OpIds of last-seen ops per peer)
  entity: NodeId | EdgeId;
  field: string;          // property name, or "__tombstone" for deletes
  payload: CRDTPayload;   // type-specific operation payload (CRDT type resolved via schema)
  group?: GroupId;         // optional operation group for atomic batches (see §2.8)
  signature: Signature;    // mandatory for any operation that leaves the local peer (§6.1)
}
```

Operations form a **causal graph** — each operation references its causal dependencies by `OpId` (content hash).  This structure provides:

1. **Causal ordering:** The `deps` field captures the happens-before relationship.  Any peer can reconstruct the causal partial order from the operation set.
2. **Integrity verification:** Each `OpId` is the content hash of the operation.  Tampered operations produce a different hash and are detectable.
3. **Deduplication:** Content-addressed operations are naturally idempotent — receiving the same operation twice is a no-op.

**Note:** The CRDT type for each operation is not stored in the operation itself.  It is resolved from the schema by looking up `(entity.label, field)`.  This avoids redundancy between the schema and the oplog, and prevents inconsistencies where an operation claims a different CRDT type than the schema declares.  Because the schema can evolve, this lookup must be bound to a **schema epoch**, not the mutable current schema ([ISSUES C2](ISSUES.md)).

**Open (M0a):** The canonical byte encoding, the hash/signature preimages (including exclusion of `id`/`signature` from their own preimages), the full operation variant set (entity creation, migrations, capability grants, key rotation), and the binding of `DatastoreId` (and related context fields) into the signed/hashed preimage are **M0a** deliverables ([ISSUES C1, C4](ISSUES.md)).  Schema-epoch binding of the CRDT type is completed in **M0b** ([ISSUES C2](ISSUES.md)).

### 2.6 Merkle Sync Tree

Separate from the causal graph, ZeroDB maintains a **Merkle sync tree** — a time-bucketed hash tree used exclusively for efficient delta synchronization between peers.

The oplog is partitioned into time-based buckets (configurable granularity, default: 1 minute).
Each bucket has a Merkle hash computed from its operations.
Buckets are organized in a balanced tree where:
- Leaf nodes = per-bucket operation hashes
- Internal nodes = hash of children
- Root = single hash representing the entire oplog state

Sync protocol walks from root downward: if a subtree matches, skip it entirely.  This gives O(log N) sync negotiation where N is the number of time buckets since divergence.

The Merkle sync tree is a **derived structure** — it is computed from the oplog and can be rebuilt at any time.  It is not part of the causal graph and carries no semantic meaning beyond enabling efficient sync.

**Open (M0c):** Canonical bucket boundaries, leaf ordering, empty-node hashes, tree shape, and the subtree-traversal messages needed to actually execute the root-down walk are **M0c** deliverables ([ISSUES C3](ISSUES.md)).  Wire shipping of the traversal protocol is M3.

### 2.7 State Materialization

ZeroDB maintains two views of data:

1. **Oplog (source of truth):** The append-only log of all operations, organized as a Merkle DAG.  This is what syncs between peers.
2. **Materialized state (read cache):** The computed current state of the graph, derived by replaying the oplog through CRDT merge functions.  This is what applications query.

The materializer is an incremental engine:
when new operations arrive (locally or via sync), it applies only the new operations to the existing materialized state rather than replaying from scratch.

**Consistency guarantee:** The materialized state is always a deterministic function of the oplog.  Given the same set of operations (in any order), every peer computes the identical materialized state.  This is the Strong Eventual Consistency (SEC) guarantee that CRDTs provide.

### 2.8 Operation Groups

CRDTs do not support traditional transactions, but many graph mutations are logically atomic — creating a node and its edges should arrive together.  ZeroDB supports **operation groups**: a set of operations tagged with a shared `GroupId` that are treated as a unit for sync and materialization.

```typescript
type GroupId = string;  // UUIDv7

// Operations in a group share the same group field
await db.batch((tx) => {
  const user = tx.create(User, { name: 'Alice', email: 'alice@example.com' });
  const post = tx.create(Post, { title: 'Hello', published: true });
  tx.link(Authored, user, post, { role: 'author' });
});
// All three operations get the same GroupId
```

**Guarantees:**

- **Local atomicity:** All operations in a group are appended to the local oplog and materialized together.  If the process crashes mid-group, none are persisted.
- **Sync atomicity:** During sync, a group is transmitted as a unit.  The receiving peer buffers operations until the full group arrives before materializing.
- **No cross-peer atomicity:** Operation groups do not provide distributed transaction semantics.  Two peers can independently create conflicting groups; CRDT merge rules still apply per-field.

**Open (M0e):** Group completion detection (a signed manifest or member count/index) and abort/expiry semantics, plus the storage transaction boundary that makes local atomicity real, are **M0e** deliverables ([ISSUES C8](ISSUES.md)).  Local implementation is M1; sync-side group completion is M3.

### 2.9 Referential Integrity

Edges reference source and target nodes by `NodeId`.  ZeroDB enforces referential integrity through **cascading tombstones**:

- **On node tombstone:** All edges where the tombstoned node is the `source` or `target` are automatically tombstoned.  This generates additional operations in the oplog (one per affected edge), causally dependent on the node's tombstone operation.
- **On edge creation:** If either the `source` or `target` node does not exist (or is tombstoned) in the materialized state, the edge is accepted into the oplog but marked as **dangling** in the materialized state.  If the referenced node later appears (e.g., arrives via sync), the edge becomes live.
- **Dangling edge queries:** By default, queries exclude dangling edges.  The query API provides an `includeDangling` option for debugging and sync diagnostics.

This design respects eventual consistency — operations can arrive in any order, and the materialized state converges regardless of whether a node or its edges arrive first.

**Open (M1):** Cascade authority (which peer emits the edge tombstones, and how it is authorized to delete edges authored by others), late-dangling-edge tombstoning, resurrection policy, and the CRDT governing `__tombstone` need a deterministic delete state machine — possibly derived visibility rather than generated cascades ([ISSUES H3](ISSUES.md)).

---

## 3. CRDT Type System & Schema DSL

### 3.1 Column-Level CRDT Selection

ZeroDB's key expressiveness advantage over GunDB is that each property on a node or edge declares its own CRDT merge strategy.
The schema is a contract: "when concurrent edits happen to this field, here's how they merge."

**Available CRDT types:**

| Type | Merge Semantics | Use Case |
|------|----------------|----------|
| `LWW<T>` | Last-Writer-Wins register. Latest HLC timestamp wins. | Names, titles, settings — any field where "most recent edit wins" is correct. |
| `GCounter` | Grow-only counter. Concurrent increments are summed, never overwritten. Value is always ≥ 0. | View counts, login counts, event counters — monotonically increasing values. |
| `PNCounter` | Positive-Negative counter. Supports both increment and decrement. Concurrent operations are summed. | Inventory quantities, vote tallies, balances — values that can go up or down. |
| `ORSet<T>` | Observed-Remove Set. Add and remove are both tracked causally. Concurrent add+remove = element is present. | Tags, labels, members lists, permission sets. |
| `MVRegister<T>` | Multi-Value Register. Concurrent writes produce multiple values; application resolves via `db.resolve()`. | Fields where conflicts must be surfaced to the user (e.g., conflicting title edits). |
| `LWWMap<K, V>` | Map where each key is an independent LWW register. Values must be scalar types (string, number, boolean, null). | Metadata bags, preferences, configuration. |
| `RGA<T>` | Replicated Growable Array. Ordered sequence with positional insert/delete. | Ordered lists, playlists, task orderings. |
| `Richtext` | Peritext-style rich text CRDT. Character-level insert/delete + formatting ranges. *(post-v0.1 — M5 feature track)* | Document content, comments, descriptions. |
| `Flag` | Enable-Wins Flag. Concurrent enable + disable = enabled. | Feature flags, active/inactive status where enabling should win. |

### 3.2 Schema Definition

Schemas are **authored in TypeScript** and compiled deterministically to the canonical **schema IR** ([SCHEMA.md](SCHEMA.md) §1–§2) — the only representation the core evaluates, replicates, or hashes (ISSUES O2, decided 2026-07-16; the earlier `.zerodb` DSL input format is dropped). The CLI consumes IR files emitted by the standalone TS→IR compiler (ships ≤ M1).

**TypeScript SDK:**

```typescript
import { z, schema, LWW, GCounter, PNCounter, ORSet, MVRegister, RGA } from 'zerodb';

const User = schema.node('User', {
  name:        LWW(z.string()),          // last write wins
  email:       LWW(z.string().email()),   // with validation
  bio:         LWW(z.string().optional()),
  tags:        ORSet(z.string()),          // add/remove tracked causally
  loginCount:  GCounter(),                 // grow-only: concurrent increments merge
  settings:    LWWMap(z.string(), z.string()),  // values are LWW scalars
});

const Post = schema.node('Post', {
  title:       MVRegister(z.string()),     // surface conflicts to user
  body:        LWW(z.string()),            // Richtext is post-v0.1 (M5 feature track)
  viewCount:   GCounter(),                 // grow-only
  score:       PNCounter(),                // upvote/downvote: can increment and decrement
  tags:        ORSet(z.string()),
  published:   LWW(z.boolean()),
});

const Authored = schema.edge('AUTHORED', {
  source: User,
  target: Post,
  role:   LWW(z.enum(['author', 'editor', 'contributor'])),
});
```

**Canonical IR (identity layer):** the compiled form is a canonical-CBOR map with `SchemaId = BLAKE3(domain("schema_ir") ‖ IR bytes)`; same logical schema ⇒ same bytes regardless of TS formatting. Structure, validation outcomes, and the `unique`/`encrypted` constraints are normative in [SCHEMA.md §2](SCHEMA.md).

### 3.3 Schema Evolution

Schemas evolve through **schema epochs** carrying **migrations as data, not code** ([SCHEMA.md §3–§4](SCHEMA.md)): a new epoch introduces a new immutable IR plus a list of migration steps (`add_prop`, `remove_prop`, `change_crdt`, `add_entity`, `remove_entity`) whose type-change transforms come from a **closed, versioned registry of total deterministic functions** — never JavaScript closures, which cannot replicate deterministically across implementations.

```typescript
// Authoring surface (compiled to SchemaEpoch operation + migration steps)
migration('001_add_avatar', {
  addProperty: { node: 'User', name: 'avatar', type: LWW(z.string().url().optional()), default: null }
});

// CRDT type change: transform is a registry tag, not a closure
migration('002_title_to_lww', {
  changeType: { node: 'Post', name: 'title', from: 'MVRegister', to: 'LWW', transform: 'keep_text' }
});
```

**Rules:**

- Adding a property is always safe — existing entities materialize the declared `default`.
- Removing a property is visibility-only — operations and history are untouched; late ops apply to shadow state.
- Changing a CRDT type names a registry transform (total and deterministic; `reset_to` is the catch-all — silent partial transforms do not exist).
- Epochs are themselves operations in the oplog (`SchemaEpoch`, KERNEL kind 5), causally ordered and bound into every data operation's signed context, so historical replay across type changes is deterministic ([ISSUES C2](ISSUES.md); executable model in SCHEMA §4.1).  Cross-peer mixed-version migration shipping is M4.

### 3.4 Schemaless Mode

To prioritize onboarding speed, ZeroDB supports a **schemaless mode** where no schema declaration is required.  This lets developers start prototyping immediately and add type safety incrementally.

**Default CRDT type:**  Any property written without a schema entry defaults to `LWW<any>`.  This is the safest general-purpose default — it resolves conflicts deterministically (latest HLC wins) and imposes no structural constraints.

**Warnings:**  Schemaless operation emits warnings at two levels:

- **Client console** — On startup when no schema is provided: `"No schema defined — all fields default to LWW. Define a schema for type safety and richer CRDT semantics."`  Additionally, the first write to each undeclared `(label, field)` pair logs: `"Field 'User.score' has no schema entry — defaulting to LWW."`
- **Relay-side warnings are not possible** — relays are schema-blind by design ([RELAY-SPEC.md](RELAY-SPEC.md)) and have no schema-registration channel.  Schema warnings are strictly a client-side concern.

**Strict mode:**  For production deployments, `ZeroDB.open({ schema, strict: true })` rejects writes to any field not declared in the schema, throwing a `SchemaViolationError` instead of falling back to LWW.  This is the recommended setting once a schema is defined.

**Migration from schemaless:**  When a developer adds a schema after prototyping without one, existing `LWW` data is inherently compatible — no migration is needed.  If a field's CRDT type should change (e.g., from the default LWW to `PNCounter`), a standard epoch migration with a registry transform is required (see §3.3).

---

## 4. Sync Protocol

### 4.1 Sync Lifecycle

```
Peer A                          Peer B
  │                               │
  ├── SyncRequest ───────────────►│  (Merkle root)
  │                               │
  │◄── SyncResponse ──────────────┤  (Merkle root)
  │                               │
  │  [Compare Merkle trees]       │  [Compare Merkle trees]
  │                               │
  ├── DeltaRequest ──────────────►│  (list of missing subtree hashes)
  │                               │
  │◄── DeltaBatch ────────────────┤  (operations, chunked)
  │                               │
  ├── DeltaBatch ────────────────►│  (operations, chunked)
  │                               │
  │◄── SyncAck ───────────────────┤  (new Merkle root confirms convergence)
  │                               │
  ├── SyncAck ───────────────────►│
  │                               │
  │  [Subscribe to live ops]      │  [Subscribe to live ops]
  │                               │
  ├── OPS ◄─────────────────────► │  (bidirectional real-time stream)
```

*Informative sketch; message names follow the relay protocol registry ([RELAY-SPEC](RELAY-SPEC.md) 0.2 — live streaming is the bidirectional `OPS` message).*

### 4.2 Sync Modes

| Mode | Trigger | Behavior |
|------|---------|----------|
| **Merkle sync** | Peer reconnects after offline period | Merkle tree comparison → delta exchange |
| **Live sync** | Peers already synchronized | Real-time bidirectional operation streaming |
| **Snapshot sync** | New peer joins with no history | Download a compressed state snapshot + recent oplog tail |

### 4.3 Transport Agnosticism

The sync protocol operates on abstract streams of operations. The transport layer is pluggable:

- **WebSocket:** Primary for browser-to-relay and Node-to-Node.
- **WebRTC DataChannel:** Browser-to-browser direct P2P.
- **Custom:** Implement the `Transport` trait for exotic environments (Bluetooth, serial, carrier pigeon).

TCP/QUIC bindings were removed with relay protocol 0.2; a server-to-server transport profile is post-v0.1 and currently unscheduled.

### 4.4 Relay Servers

ZeroDB is peer-to-peer, but relay servers exist for:

- **Peer discovery:** A lightweight signaling server for WebRTC negotiation.
- **Always-on persistence:** A relay that stores the full oplog serves as a backup and catch-up point for peers that go offline.
- **Fan-out efficiency:** Rather than every peer connecting to every other, relays aggregate and redistribute operations.

Relays are untrusted — they cannot forge operations (signatures verify origin), and censorship is detectable **only when a peer compares Merkle roots against a second independent source** ([RELAY-SPEC §12.3](RELAY-SPEC.md)): a sole relay can present a self-consistent censored view. A peer can use any relay, run their own, or operate without one in direct P2P mode.

The relay protocol is drafted in the companion [Relay Protocol Specification](RELAY-SPEC.md), which defines conformance levels, wire format, message types, and operational requirements for third-party relay implementations.  The draft is **not yet implementation-ready**: no relay conformance may be claimed until composite **M0** (§10, packages M0a–M0f) resolves the Critical contracts in [ISSUES.md](ISSUES.md) — notably op encoding (C1/M0a), Merkle traversal (C3/M0c), datastore admission (C4/M0a+M0d), author-key resolution (C5/M0d), and delivery/replay semantics (H4/M0e).

---

## 5. CLI & Developer Experience

### 5.1 Design Philosophy

The `zerodb` CLI is the primary developer interface for administration, debugging, and scripting. It follows the ergonomic patterns of tools like `git`, `wrangler`, and `turso`:

```bash
# Initialize a new ZeroDB project
zerodb init my-app

# Apply a compiled schema IR (emitted by the TS→IR compiler)
zerodb schema apply schema.ir

# Open an interactive REPL
zerodb repl

# Inspect the local graph
zerodb query 'MATCH (u:User)-[:AUTHORED]->(p:Post) WHERE u.name = "Alice" RETURN p.title'

# Sync status
zerodb sync status
zerodb sync connect wss://relay.example.com

# Inspect the oplog
zerodb oplog tail --follow
zerodb oplog range --after "2026-03-01" --before "2026-03-15"
zerodb oplog export --format json > backup.jsonl

# Peer management
zerodb peers list
zerodb peers trust <peer-id>
zerodb peers block <peer-id>

# Key management
zerodb keys generate
zerodb keys export --public > my-key.pub
zerodb keys import peer-key.pub --trust

# Debug / inspect
zerodb inspect node <node-id>
zerodb inspect edge <edge-id>
zerodb inspect merkle --depth 3
zerodb health
```

### 5.2 Query Language

ZeroDB uses a Cypher-inspired query syntax for the CLI REPL and programmatic queries. The **v0.1 subset is normative in [SCHEMA.md §5](SCHEMA.md)** (ISSUES O3, decided 2026-07-16): `MATCH` / `WHERE` / `RETURN` / `ORDER BY` / `LIMIT`, one hop max, parameterization via `$name` placeholders only, deterministic null/cross-type/conflict semantics.

```cypher
-- v0.1 subset: filter, order, project
MATCH (u:User)-[a:AUTHORED]->(p:Post)
WHERE u.name = $name AND p.published = true
RETURN u.name, p.title, a.role
ORDER BY p.viewCount DESC
LIMIT 20
```

Inline property predicates (`{name: "Alice"}`), multi-hop patterns, aggregation, and mutation-in-query are **post-v0.1**; the v0.1 parser rejects them rather than partially executing.
Queries are read-only; mutations go through the SDK or CLI mutation commands.

### 5.3 TypeScript SDK

```typescript
import { ZeroDB } from 'zerodb';

// Initialize
const db = await ZeroDB.open({
  name: 'my-app',
  schema: [User, Post, Authored],
  storage: 'indexeddb',         // or 'opfs', 'sqlite'
  relay: 'wss://relay.example.com',
});

// Create nodes
const alice = await db.create(User, {
  name: 'Alice',
  email: 'alice@example.com',
  tags: ['admin', 'early-adopter'],
  loginCount: 0,
});

const post = await db.create(Post, {
  title: 'Hello World',
  viewCount: 0,
  tags: ['intro'],
  published: true,
});

// Create edges
await db.link(Authored, alice, post, { role: 'author' });

// Queries
const alicePosts = await db.query(Post)
  .where(p => p.published.eq(true))
  .through(Authored, { direction: 'incoming', source: alice })
  .orderBy('viewCount', 'desc')
  .limit(10)
  .exec();

// Reactive subscriptions
const unsubscribe = db.subscribe(
  db.query(Post).where(p => p.tags.contains('breaking')),
  (posts) => { console.log('Breaking posts updated:', posts); }
);

// CRDT-aware mutations
await db.mutate(post, (p) => {
  p.viewCount.increment(1);     // GCounter: concurrent increments merge
  p.score.increment(1);          // PNCounter: supports increment and decrement
  p.score.decrement(1);
  p.tags.add('featured');        // ORSet: concurrent add+remove = present
  p.title.set('Updated Title'); // MVRegister: concurrent sets = multi-value
});

// Inspect CRDT state directly
const titleState = await db.crdtState(post, 'title');
// MVRegister: { values: ['Updated Title'], conflicts: false }
// If two peers set title concurrently:
// { values: ['Title A', 'Title B'], conflicts: true }

// Resolve MVRegister conflicts
if (titleState.conflicts) {
  // Application picks a winner (or merges, or prompts the user)
  await db.resolve(post, 'title', titleState.values[0]);
  // This writes a new LWW-style set that supersedes all concurrent values
}

// Sync control
await db.sync.connect('wss://relay.example.com');
await db.sync.connectPeer(peerId);  // direct P2P
db.sync.status; // 'synced' | 'syncing' | 'offline'

// Cleanup
db.close();
```

### 5.4 React Hooks (optional package)

```typescript
import { useQuery, useNode, useMutation, useSyncStatus } from 'zerodb/react';

function PostList() {
  const posts = useQuery(
    db.query(Post).where(p => p.published.eq(true)).orderBy('viewCount', 'desc')
  );
  const syncStatus = useSyncStatus();

  return (
    <div>
      <span>Sync: {syncStatus}</span>
      {posts.map(post => <PostCard key={post.id} post={post} />)}
    </div>
  );
}

function PostCard({ post }) {
  const node = useNode(post.id);
  const mutate = useMutation();

  return (
    <div>
      <h2>{node.title}</h2>
      <button onClick={() => mutate(post.id, p => p.viewCount.increment(1))}>
        👁 {node.viewCount}
      </button>
    </div>
  );
}
```

### 5.5 Mutation Semantics

**Concurrent local mutations:** Multiple `db.mutate()` calls on the same entity are serialized through the HLC — each call generates a new operation with a strictly greater HLC timestamp than the previous.  Mutations are never interleaved at the operation level.  From the application's perspective, `db.mutate()` returns a promise that resolves once the operation is appended to the local oplog and materialized.

**Mutation → operation mapping:** A single `db.mutate()` call produces one operation per field touched.  Mutating three fields produces three operations, all sharing the same `GroupId` (see §2.8) to ensure they are applied atomically.

---

## 6. Cryptography & Auth

### 6.1 Identity Model

Each peer generates an **Ed25519 keypair** on first run. The public key hash serves as the `PeerId`. Every operation that syncs to another peer **MUST be signed** — unsigned operation is permitted only for explicitly local-only databases and is non-interoperable.

Because `PeerId` is a hash, a receiving peer cannot recover the author's public key from it.  Forwarded operations (peer bridging, relay-to-relay sync) therefore require the author's key to be carried alongside or resolvable via an authenticated lookup; that distribution/rotation contract is an **M0d** deliverable ([ISSUES C5](ISSUES.md)).  On-wire enforcement ships M3.

```
PeerId = BLAKE3(Ed25519PublicKey)  // full 32 bytes stored; truncated to 16 hex chars for display
```

### 6.2 Built-in Crypto Layer

The default crypto layer provides:

- **Operation signing:** Each operation includes an Ed25519 signature.  Peers can verify any operation's authorship.
- **End-to-end encryption:** X25519 key exchange + XChaCha20-Poly1305 for encrypted properties.  A node can have both public and encrypted fields.
- **Key rotation:** Peers can rotate keys.  The old key signs a "key rotation" operation that delegates trust to the new key.
- **Web of trust:** Peers can sign each other's public keys, forming a trust graph.  Access control policies reference trust relationships.

### 6.3 Pluggable Auth

The crypto layer implements a trait:

```rust
pub trait CryptoProvider: Send + Sync {
    fn sign(&self, message: &[u8]) -> Result<Signature>;
    fn verify(&self, message: &[u8], signature: &Signature, pubkey: &PublicKey) -> Result<bool>;
    fn encrypt(&self, plaintext: &[u8], recipient: &PublicKey) -> Result<Vec<u8>>;
    fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>>;
    fn peer_id(&self) -> PeerId;
}
```

Applications can swap in their own provider — e.g., hardware security modules or WebAuthn-backed keys — but interoperable peers **MUST produce the fixed wire suite**: Ed25519 signatures, BLAKE3 hashing and `PeerId` derivation, X25519 + XChaCha20-Poly1305 encryption.  Custom providers change key custody, not algorithms or identifier derivation.

---

## 7. Storage Layer

### 7.1 Storage Adapter Traits

The storage layer is decomposed into focused traits.  Backends implement each trait independently, and the core composes them.

```rust
/// Append-only operation log storage.
#[async_trait]
pub trait OplogStore: Send + Sync {
    async fn append_ops(&self, ops: &[Operation]) -> Result<()>;
    async fn read_ops(&self, range: OpRange) -> Result<Vec<Operation>>;
    async fn ops_since(&self, after: &HLCTimestamp) -> Result<Vec<Operation>>;  // Merkle sync is the cross-peer delta mechanism; this is a local read primitive
}

/// Materialized graph state: read and write the computed current state.
#[async_trait]
pub trait StateStore: Send + Sync {
    async fn get_node(&self, id: &NodeId) -> Result<Option<Node>>;
    async fn get_edge(&self, id: &EdgeId) -> Result<Option<Edge>>;
    async fn query_nodes(&self, label: &str, filter: &Filter) -> Result<Vec<Node>>;
    async fn query_edges(&self, label: &str, filter: &Filter) -> Result<Vec<Edge>>;
    async fn traverse(&self, start: &NodeId, pattern: &TraversalPattern) -> Result<Vec<Path>>;
    async fn put_materialized(&self, entity: &Entity) -> Result<()>;
}

/// Merkle sync tree storage (derived structure, rebuildable from oplog).
#[async_trait]
pub trait MerkleStore: Send + Sync {
    async fn merkle_root(&self) -> Result<Hash>;
    async fn merkle_subtree(&self, depth: usize) -> Result<MerkleTree>;
    async fn rebuild(&self) -> Result<()>;
}

/// Housekeeping: compaction, snapshots, garbage collection.
#[async_trait]
pub trait Maintenance: Send + Sync {
    async fn compact(&self) -> Result<CompactionStats>;
    async fn snapshot(&self) -> Result<Snapshot>;
    async fn gc_tombstones(&self, stable_before: &HLCTimestamp) -> Result<usize>;
}
```

A complete storage backend bundles all four traits.  The `StorageBackend` type alias composes them:

```rust
pub trait StorageBackend: OplogStore + StateStore + MerkleStore + Maintenance {}
```

### 7.2 Backend Implementations

| Backend | Environment | Oplog Storage | State Storage | Notes |
|---------|-------------|---------------|---------------|-------|
| **IndexedDB** | Browser | `ops` object store, keyed by HLC | `nodes` + `edges` stores with indexes on label | Broadest browser compat. Async API. |
| **OPFS** | Browser (modern) | Append-only binary file | SQLite-over-OPFS via `wa-sqlite` | Higher throughput than IDB. Origin-private. |
| **SQLite** | Node.js / CLI | `operations` table with B-tree on HLC | `nodes` + `edges` tables with indexes | Full SQL power for complex queries. |

**Auto-selection:** The SDK detects the environment and picks the best available backend.  Browser prefers OPFS where available, falls back to IndexedDB. Node.js uses SQLite.

### 7.3 Compaction & Garbage Collection

The oplog grows indefinitely without intervention. Compaction strategies:

- **Causal stability pruning:** Once an operation has been acknowledged by all known peers (its HLC is below every peer's known minimum timestamp), the raw operation can be discarded and only its effect on materialized state is retained.
- **Tombstone GC:** Deleted nodes/edges (tombstoned) are fully removed after all peers have seen the delete.
- **Snapshot checkpointing:** Periodically, the materializer writes a full state snapshot.  The oplog can be truncated to only contain operations after the snapshot.
- **CRDT metadata pruning:** ORSet and RGA maintain internal metadata (tombstone markers, vector clocks per element) that grows with the history of add/remove operations.  After causal stability, internal CRDT metadata for acknowledged operations is compacted — e.g., ORSet tombstones for elements that all peers agree are removed can be dropped.

**GC granularity: time-bucket.**  Garbage collection operates on time buckets (ranges of HLC timestamps) rather than per-entity.  All CRDT metadata, tombstones, and compactable oplog entries within a bucket become eligible for collection once the bucket's upper bound is causally stable — i.e., below every known peer's acknowledged minimum.  This may retain metadata slightly longer for long-lived entities whose last mutation falls in a recent bucket, but it eliminates the need for per-entity causal stability tracking.  The trade-off is acceptable because the approach is eventually consistent: all reclaimable metadata is collected eventually, just not at the earliest possible moment for every individual entity.

**Status: disabled until specified.**  "Acknowledged by all known peers" is not yet a sound causal-stability rule — it needs per-peer durable acknowledgement frontiers, a peer-membership lifecycle (retirement/leases for departed peers), checkpoint identity for comparing compacted and uncompacted histories, and anti-replay commitments that survive compaction.  Contracts are **M0f** deliverables; compaction and GC remain disabled until those contracts exist **and** partition/rejoin, forgotten-peer, late-operation, and restore tests pass at M5 ([ISSUES C7](ISSUES.md)).

### 7.4 Indexing

The materialized state supports **secondary indexes** to avoid full scans during queries.

**Automatic indexes:**

- **Label index:** All nodes and edges are indexed by label.  `query_nodes("User", ...)` never scans non-User nodes.
- **Edge endpoint index:** Edges are indexed by `(source, label)` and `(target, label)` for efficient traversal in both directions.

**Schema-declared indexes:**

```typescript
const User = schema.node('User', {
  name:   LWW(z.string()),
  email:  LWW(z.string().email()),
}, {
  indexes: [
    { fields: ['email'], unique: true },    // unique secondary index
    { fields: ['name'] },                   // non-unique secondary index
  ],
});
```

```
node User {
  name   LWW<string>
  email  LWW<string>

  @index(email, unique)
  @index(name)
}
```

**Implementation:** In SQLite backends, indexes map directly to SQL indexes.  In IndexedDB, they map to IDB indexes on object stores.  The `StateStore` trait's `query_nodes` and `query_edges` methods use indexes when the filter matches an indexed field, falling back to scan otherwise.

---

## 8. Formal Verification Strategy

### 8.1 What We Prove

Using Lean 4, ZeroDB aims to provide machine-checked proofs of:

1. **CRDT convergence:** For each CRDT type (LWW, Counter, ORSet, MVRegister, RGA), prove that the merge function is commutative, associative, and idempotent — guaranteeing that any order of operation application produces the same result.

2. **HLC correctness:** Prove that the HLC maintains monotonicity on a single peer and preserves causal ordering after message exchange.

3. **Merkle DAG integrity:** Prove that the delta sync protocol is complete — after sync, both peers have identical oplog content (modulo compacted operations).

4. **Schema migration safety:** Prove that additive migrations preserve convergence — adding a new property to an existing schema doesn't break CRDT guarantees for existing properties.

### 8.2 Scope & Pragmatism

Formal verification is scoped to the **core algorithms**, not the entire system. The proofs verify:

- The mathematical properties of each CRDT merge function
- The HLC algorithm
- The sync protocol's convergence guarantee

They do **not** verify:

- Storage adapter correctness (trusted; tested by integration tests)
- Transport layer reliability (trusted; TCP/WS provide their own guarantees)
- FFI binding correctness (trusted; tested by cross-platform integration tests)

### 8.3 Proof Extraction

The Lean proofs serve double duty:

- **Trust signal:** The proofs are published alongside the codebase and can be independently verified.
- **Reference implementation:** The Lean code serves as an executable specification.  The Rust implementation is tested against the Lean reference for conformance.

---

## 9. Security Model

### 9.1 Threat Model

| Threat | Mitigation |
|--------|------------|
| **Malicious peer injects forged operations** | Mandatory Ed25519 signatures on all synced operations; unsigned ops rejected |
| **Relay censors or drops operations** | Merkle root comparison against a **second independent source** detects omissions; a sole relay can present a self-consistent censored view ([ISSUES H8](ISSUES.md)) |
| **Replay attack** | Operation deduplication by OpId; a durable anti-replay commitment must survive dedup-state compaction ([ISSUES H4](ISSUES.md)) |
| **Clock manipulation** | HLC drift detection; acceptance/quarantine rule for far-future timestamps under design ([ISSUES H1](ISSUES.md)) |
| **Data exfiltration from relay** | Only **encrypted** properties are confidential; public properties are visible to relays ([ISSUES H10](ISSUES.md)) |
| **Sybil attack (fake peers flood network)** | Datastore-membership admission + per-PeerId rate limiting; per-identity limits alone do not stop Sybils ([ISSUES H8](ISSUES.md)) |

### 9.2 Access Control

> **Status: post-v0.1 design.**  v0.1 ships **datastore-level access control only**: mandatory operation signatures plus datastore-membership capabilities verified at admission ([ISSUES C4](ISSUES.md)).  The entity-level declarative ACLs below are deferred until a causal authorization model is specified — root authority, grant/revoke ordering, deterministic accept/reject/quarantine, and reevaluation ([ISSUES C6](ISSUES.md)).  Read-ACL filtering is a convenience, **not a confidentiality boundary**: any peer holding replicated plaintext can read it outside the SDK.  Confidentiality comes from replication boundaries (separate datastores) and encryption (§6.2).

ZeroDB supports **declarative, capability-based access control** at the schema level.  Policies are expressed as data (not closures) so they can be serialized, replicated through the oplog, and evaluated consistently across all peers.

**Policy rules** are declarative predicates over the peer, entity, and operation:

```typescript
const Post = schema.node('Post', {
  title:  LWW(z.string()),
  body:   LWW(z.string()),
}, {
  acl: {
    write: [
      { rule: 'origin' },                    // creator can always write
      { rule: 'capability', cap: 'post:edit' }, // peers with this capability
    ],
    read: [
      { rule: 'origin' },                    // creator can always read
      { rule: 'field_equals', field: 'published', value: true }, // anyone if published
    ],
  }
});
```

```
node Post {
  title  LWW<string>
  body   LWW<string>

  @acl write: origin | cap("post:edit")
  @acl read:  origin | published == true
}
```

**Built-in rule types:**

| Rule | Semantics |
|------|-----------|
| `origin` | Peer is the entity's creator (`_meta.origin === peer.id`) |
| `capability` | Peer holds a named capability token |
| `field_equals` | A field on the entity matches a value |
| `peer_in_set` | Peer's ID is in a named ORSet on the entity (e.g., `editors` field) |
| `always` | Unconditional allow (public access) |

**Capability tokens** are operations in the oplog — a peer with the `grant` capability can issue a `GrantCapability` operation that gives another peer a named capability.  Revocation is also an oplog operation.  Because capabilities travel through the oplog, all peers converge on the same access control state.

**Enforcement:** ACL evaluation happens at the storage layer.  When a peer receives an operation via sync, it evaluates the write ACL.  Operations that fail the check are **quarantined** — stored separately, not applied to materialized state, and flagged for the application to review.  Note that quarantine does **not** by itself guarantee convergence: the originating peer and receivers can materialize different accepted sets, which is exactly the unresolved problem in [ISSUES C6](ISSUES.md); this section stays non-normative until C6 defines deterministic accept/reject/quarantine and reevaluation.

**Composing richer policies:** The built-in rules are intentionally minimal primitives.  Applications express complex access patterns — role hierarchies, time-based expiry, approval workflows — by designing their data model around capabilities and set-membership fields, then referencing those fields in declarative ACL rules.  This keeps ACL evaluation deterministic across all peers without requiring custom code execution at the engine level.

---

## 10. Roadmap

Development proceeds in **milestones**, each ending in a runnable outcome and beginning with failing contract/acceptance tests for its stated behavior (red/green).  The [Exemplar ToDo app](EXEMPLAR.md) supplies end-to-end acceptance scenarios from M1 onward (scenario IDs **E1–E11**, mapped to [INVARIANTS](INVARIANTS.md) IDs).  Blocking issues are tracked by ID in [ISSUES.md](ISSUES.md).

**v0.1 commitments (decided 2026-07-13):**

- First runtime: **Rust core + SQLite + CLI** — no binding layer in the first slice (ships at **M1**; multi-peer trust/sync at **M3**).
- Trust model: **mandatory operation signatures + datastore-membership capabilities**; entity-level distributed ACLs deferred (ISSUES C6).
- Non-goals for v0.1: distributed entity ACLs, mobile bindings, Richtext, hosted relay, GunDB migration tooling.

**Release naming (informational):**

| Label | Milestone exit | Meaning |
|-------|----------------|---------|
| `v0.1.0-local` | M1 | Offline single-peer core + CLI (**MVP**) |
| `v0.1.0-sdk` | M2 | Node/NAPI vertical, byte-identical fixtures |
| `v0.1.0` | M3c | First multi-peer secure product slice **with offline catch-up** |

### Cross-platform product acceptance (2026-09-05)

The product goal is **authorized data sharing between independent applications and agents across browser, CLI and desktop**, with durable offline edits and recovery. M1/M2/M3 release labels remain as defined above; M3c alone does not establish cross-platform product readiness.

The first cross-platform pilot adds **XP-1**, after M3c and the necessary M4a platform slice. It requires two distinct applications plus a non-interactive agent, exercising browser, native CLI and one desktop shell against the same datastore through the durable relay, with separate authorized identities. They must agree on datastore/schema identity, supported CRDT semantics, accepted operations and materialized state after offline concurrent edits, client/relay restart and reconnection. Tests must cover unauthorized/wrong-datastore writes, revocation, encrypted-property confidentiality, duplicate command retries, unknown epochs/versions, and real browser persistence failures. Scope and evidence: [cross-platform review](../plan/CROSS-PLATFORM-REVIEW.md); work rows: [LEDGER](../plan/LEDGER.md).

Prioritize an authenticated RELAY/WSS adapter path, enrollment/grant/revoke UX, browser durable persistence, structured CLI/Node agent API and one packaged desktop shell. Agent tooling uses the same datastore authorization as applications; a local query filter is not a security boundary. A thin MCP adapter may reuse that API. No founder-seed sharing or unauthenticated public agent bridge.

Split **M4a-share** (browser RELAY + tested IndexedDB durability and XP-1 platform support) from **M4a-extended** (OPFS, React hooks, WebRTC/direct-peer parity). Extended features still belong to full M4a but do not block the relay-backed pilot. Preserve M4b migration/upgrade and M5 GA gates. This does not freeze a format, waive M3b/H10, or declare any new track complete. Security-relevant pinned remainders require an explicit supported-profile disposition before release, not silent exemption.

### Pre-M0 implementation policy

Until composite **M0** exits:

| May land now | Must not freeze yet |
|--------------|---------------------|
| Pure CRDT merge math, HLC algorithms, exploratory tests | Wire codecs (CBOR op encoding), content-addressed `OpId` preimages |
| Throwaway prototypes under explicit `experimental` docs | Persistent on-disk layouts, SQLite schemas intended as stable |
| Fixture harness scaffolding (empty / red tests) | `PeerId` derivation, Merkle bucket rules, membership capability bytes |
| Documentation and conformance directory layout | RELAY message extensions claiming completeness |

Existing `zerodb-core` / `zerodb-storage` crates are **experimental** until M0a golden vectors exist.  Types there must not be treated as normative.

### Approved resolution checklist

A Critical (or High-contract) issue is **resolved for M0 package exit** only when all of the following exist:

1. **Normative prose** in SPEC.md and/or RELAY-SPEC.md (not only a direction line in ISSUES.md)
2. **Machine-readable artifact** where applicable (schema, registry, or fixture set)
3. **Golden positive + negative vectors** checked by at least one automated harness
4. **Decision Log** entry in [ISSUES.md](ISSUES.md)
5. Issue **removed** from the open list per ISSUES policy (outcome preserved in the Decision Log)

Directions marked “direction decided” (e.g. C4, C6) are **not** resolutions until steps 1–5 complete.  **C6** is deferred past v0.1 and is **not** part of the composite M0 exit.

### M0 — Executable contracts (packages M0a–M0f)

**Composite outcome:** independent encoders can produce and validate the same operation, schema-epoch, Merkle, auth, group/delivery, and snapshot/frontier fixtures.  Second implementation for package golden vectors is a **TypeScript pure encoder/decoder** under `conformance/` (no storage, not the full SDK).

**Composite exit gate:** Critical issues **C1–C5, C7–C8** have approved resolutions (checklist above) with red→green conformance tests for each package.  **No wire or persistent format freezes before this composite gate.**  Individual packages may land fixtures and experimental codecs; freezes require composite M0 exit (or an explicit Decision Log exception naming which format subset is frozen).

**Two conformance layers (auditable gate rule):**

1. **M0 contract-model conformance** — every package exit above means its *executable reference model* (pure state machines, fixtures, transcripts — no production backend, no SQLite) is green in Rust and the TypeScript conformance runner.  This and only this closes an M0 package and the composite gate.
2. **M1 backend conformance** — the SQLite/production backend re-runs the same contracts as implementation tests (crash injection at every named boundary, durability, recovery).  These tests belong to M1's exit gate and are **not** required for any M0 package to close.

A milestone may never both require and defer the same artifact: if a test needs a production backend, it is layer 2 and gates M1, not M0.

**Package order and dependencies:**

```
M0a (ops/encoding) ──► M0b (schema epochs) ──► M0c (Merkle SM)
        │                      │
        └──────────► M0d (keys + membership)
                              │
M0a ──► M0e (groups, delivery, versions)
M0c + M0e ──► M0f (frontiers / snapshot contracts)
```

Lean 4: **proof statements / model sketches** may be drafted anytime during M0 while formats are still changeable.  **Machine-checked proofs do not gate M0** (they track under M5 / assurance).

#### M0a — Operation algebra & canonical encoding

**Outcome:** two encoders (Rust + TypeScript conformance) produce identical `OpId` hashes and signature preimages for the same logical operations.

> **Contract draft in progress:** [KERNEL.md](KERNEL.md) owns the M0a normative text (operation algebra, deterministic CBOR profile, preimages, HLC state machine, CRDT semantic kernel, encrypted-value envelope, `BlobRef`); machine-readable constants in [`conformance/registry.json`](../conformance/registry.json). First `hlc-transition`/`crdt-apply` vectors are in the xfail lane.

- [x] Versioned operation algebra: all variants (entity creation, property ops, tombstones; migration/capability/key-record control tags reserved with preimage participation fixed — bodies land with their owning packages) — [KERNEL §4](KERNEL.md)
- [x] Fixed identifier encodings/lengths (`OpId`, `PeerId`, `NodeId`, `EdgeId`, `DatastoreId`, `GroupId`, keys, signatures) — KERNEL §2 + registry
- [x] Deterministic CBOR rules, duplicate-key rejection, domain-separated hash/signature preimages (`id`/`sig` excluded from their own preimages) — KERNEL §3–§4.4
- [x] `DatastoreId`, `operation_format_version`, and schema epoch inside the signed/hashed operation context ([ISSUES C4](ISSUES.md) context half) — KERNEL §4.1
- [x] Provisional operation/batch size limits ([ISSUES O6](ISSUES.md)) — registry `limits`
- [x] Seed [INVARIANTS.md](INVARIANTS.md) with encoding, content-addressing, and SEC statements — I-1..I-17
- [x] Golden byte-level + negative fixtures in `conformance/vectors/required` (typed-binding generation deferred as impractical pre-M2; harnesses build from vector descriptions)

**Exit gate:** C1 normative + fixtures green in Rust and TS conformance.  C4 context fields specified (admission credential format completes in M0d).  **Resolved 2026-07-16** (Decision Log; 24-vector corpus CI-blocking in both runners; corpus grows with later packages; draft-1 profile — byte freeze remains gated on composite M0).

#### M0b — Schema IR, epochs & query subset

**Outcome:** every data operation binds to an immutable schema epoch; migrations are a deterministic DSL (no JS closures); M1 has a frozen minimal query grammar.

> **Contract:** [SCHEMA.md](SCHEMA.md) owns the M0b normative text (schema IR §2, epochs §3, migration DSL + segmented-replay model §4, v0.1 query subset §5). All five vector families (`schema-ir`, `epoch-replay`, `migration-transform`, query parse/eval) are promoted and CI-blocking in both runners. The TS→IR compiler ships as a standalone tool ≤ M1 (O2) and is not an M0b gate.

- [x] One canonical schema IR with immutable IDs/versions — TS authoring-canonical, IR identity-canonical, `.zerodb` DSL dropped ([ISSUES O2](ISSUES.md) decided 2026-07-16; SCHEMA §1–§2)
- [x] Causally ordered schema epoch on every data operation; CRDT type bound to the op's own epoch's immutable IR ([ISSUES C2](ISSUES.md); SCHEMA §3)
- [x] Serializable migration DSL; mixed-version buffering/rejection and rollback rules (SCHEMA §4 + §3 mixed-version rule; cross-peer shipping M4)
- [x] Minimal query subset frozen for M1 CLI: grammar + null/conflict semantics for MATCH/WHERE/RETURN/ORDER BY/LIMIT ([ISSUES O3](ISSUES.md) decided; SCHEMA §5); aggregation/paths deferred

**Exit gate:** C2 normative; O2/O3 decided; epoch-bound replay vectors (including a type-change migration) red→green.  **Resolved 2026-07-18** (Decision Log; 57-vector corpus CI-blocking in both runners; draft-1 profile — byte freeze remains gated on composite M0; TS→IR ≤ M1; cross-peer migration shipping M4).

#### M0c — Merkle tree & sync state machine

**Outcome:** equal oplogs hash to equal roots; unequal roots are traversable to a concrete delta via a published state machine (transcript fixtures), independent of the eventual WebSocket framing.

> **Contract:** [MERKLE.md](MERKLE.md) — 1-minute buckets, leaf/node/empty domain separation, power-of-two pad, abstract mismatch-recovery walk. Root + transcript vectors promoted.

- [x] Canonical authenticated tree: bucket boundaries, leaf ordering, empty-node hashes, shape, internal encoding ([ISSUES C3](ISSUES.md); MERKLE §3; MERKLE-001..004)
- [x] Path/range subtree walk (NodeRequest/Response, LeafRequest/Response) — abstract model MERKLE §4; wire framing M3
- [x] Concurrent-write rule: freeze snapshot at walk start, final root re-compare (MERKLE §4)
- [x] Complete mismatch-recovery transcript (MERKLE-T-001..004; wire ships M3)

**Exit gate:** C3 normative; published root vectors; at least one full mismatch-recovery transcript green in both conformance encoders.  **Resolved 2026-07-18** (Decision Log; 8 Merkle vectors CI-blocking in both runners within 95-vector corpus; draft-1 profile).

#### M0d — Author keys & datastore membership

**Outcome:** author signatures are verifiable under forwarding; datastore admission has a capability format relays can check without schema.

> **Contract:** [AUTH.md](AUTH.md) owns the M0d normative text (two-level identity + device certs §1, genesis/root authority §2, membership capabilities + admission token §3, per-op authz predicate §4, relay vs peer roles §5). Directions DQ-1/2/3 resolved via the M0d checklist. All four vector families promoted (AUTH-CERT / AUTH-GEN / AUTH-AUTHZ / AUTH-ADM).

- [x] Device certificate format + root-sig verify + PrincipalId/PeerId bind ([ISSUES C5](ISSUES.md); AUTH §1; AUTH-CERT vectors)
- [x] Genesis body + self-certifying `DatastoreId` (AUTH §2; AUTH-GEN vectors)
- [x] Per-op authz predicate: causal grant-time, concurrent revoke, founder synthetic (AUTH §4; AUTH-AUTHZ vectors)
- [x] Relay-verifiable **datastore-membership capabilities** / admission token (AUTH §3.3; AUTH-ADM vectors)
- [x] Author key resolution contract: transport sender ≠ author; inline or prior cert; named `AUTHOR_UNRESOLVED`/`AUTHOR_UNKNOWN` (AUTH §1.3–§1.4); on-wire enforcement M3b
- [x] Negative vectors: forged/wrong-ds/missing membership/revoked (AUTH-AUTHZ + AUTH-ADM + AUTH-CERT)

**Exit gate:** C4 admission + C5 normative; negative auth vectors green.  On-wire enforcement remains M3.  **Resolved 2026-07-18** (Decision Log; 18 auth vectors CI-blocking in both runners within 75-vector corpus; draft-1 profile; closes CX-02).

#### M0e — Groups, delivery, ack & version policy

**Outcome:** group completeness and crash/recovery boundaries are specified; delivery is at-least-once with durable anti-replay intent; version authority is named.

> **Contracts:** [WAL.md](WAL.md) (M0e.1), [DELIVERY.md](DELIVERY.md) (M0e.2), [VERSIONS.md](VERSIONS.md) (M0e.3).

- [x] Signed group manifest (model form) + incomplete/abort outcomes ([ISSUES C8](ISSUES.md); WAL.md; WAL-004..008)
- [x] Atomic storage / WAL/replay with named crash points — layer 1 (WAL-001..012); SQLite layer 2 at M1
- [x] Delivery/dedup/replay: at-least-once, anti-replay, batch outcomes, resume ([ISSUES H4](ISSUES.md); DELIVERY.md; DELIV-001..004)
- [x] Receipt vs durable ack contract ([ISSUES H11](ISSUES.md); DELIVERY §5 — impl M3+)
- [x] Per-format version authority + decode limits ([ISSUES H7](ISSUES.md); VERSIONS.md + registry)
- [x] Identifier/hash encoding registry seed ([ISSUES H9](ISSUES.md); registry.json)

**Exit gate:** C8 + H4/H7 contracts normative; suites green at layer 1.  **Resolved 2026-07-18.** SQLite layer 2 gates M1.

#### M0f — Causal frontiers & snapshot contracts

**Outcome:** GC and snapshot bootstrap have implementable contracts **without enabling GC**.  No compaction ships until M5 tests pass.

> **Contract:** [FRONTIER.md](FRONTIER.md).

- [x] Causal frontiers independent of wall-clock; peer acks + retirement/lease rules ([ISSUES C7](ISSUES.md); FRONTIER §1–§3; FRONT-001)
- [x] Authenticated snapshot identity, tail boundaries ([ISSUES C7](ISSUES.md); FRONTIER §5; FRONT-002)
- [x] L2 stores peer-produced authenticated snapshots (not schema-blind materialization) — recorded FRONTIER §5
- [x] Compact frontier for `deps` scale ([ISSUES O7](ISSUES.md); FRONTIER §1)
- [x] **GC remains disabled** until M5; late-op rule FRONT-003

**Exit gate:** C7/O7 contracts normative; snapshot identity fixtures; no GC implementation required.  **Resolved 2026-07-18.**

### M1 — Local durable core (Rust + SQLite + CLI)

**Depends on:** composite M0 at the **contract-model layer** (layer 1 above) for M0a, M0b, and M0e; M0c/M0d/M0f model suites may close in parallel with early M1 work since local-only builds omit sync/auth wire.  M1 owns the **backend layer** (layer 2): SQLite crash-injection versions of the M0e storage/group contracts become green here.

**Outcome:** offline exemplar CRUD and deterministic restart/replay on a single peer (`v0.1.0-local`).

- [x] Graph entities, HLC, oplog, incremental materializer, operation groups (per M0a/M0e) — `zerodb-storage` (`atomic_group`, `m1_wave1` HLC suites)
- [x] CRDTs: LWW, GCounter, PNCounter, ORSet, Flag — store + NAPI parity (`m2-parity.test.mjs`)
- [x] Deterministic delete/referential-integrity state machine (ISSUES H3) — **derived visibility** (no cascade ops): node/edge tombstones set-derived, edge visible iff not tombstoned and both endpoints live (`e9_delete_machine`, `r0_stabilize`)
- [x] Schema pin (JSON) + type-pin reject; strict + schemaless modes — *canonical CBOR IR and secondary indexes re-scoped to M2/M3 (Decision Log 2026-07-25)*
- [x] CLI (M1 subset): `init`, `schema-apply`, `query` (O3), `inspect` — *interactive `repl` re-scoped to M2 (Decision Log 2026-07-25)*
- [x] Atomic oplog+state transaction and crash recovery (ISSUES C8, local half of M0e) — named-failpoint crash matrix at every commit boundary (`e4_crash_matrix`)

**Exit gate:** property/model tests, randomized replay equivalence, crash atomicity at every commit boundary, storage contract tests, duplicate/replay tests, offline exemplar acceptance (**E1, E2 model-level, E4, E9**).  **Resolved 2026-07-25** (Decision Log; E1 `e1_e2_acceptance` + `e1_kill_clock` kill/clock-rollback, E2 model-level `e1_e2_acceptance` e2_*, E4 `e4_crash_matrix`, E9 `e9_delete_machine`). Format freeze remains a separate, still-open Decision Log act — `v0.1.0-local` is an experimental-format release.

### M2 — One SDK vertical (Node/NAPI + SQLite)

**Outcome:** the TypeScript SDK runs the same exemplar and produces byte-identical core fixtures.

- [x] NAPI binding; `open`/`create`/`query`/`mutate`/`subscribe` — sync `Database` + thin `zerodb.mjs` facade (`zerodb-napi/test/m2-*.test.mjs`)
- [ ] MVRegister + `resolve` flow, RGA, LWWMap — *deferred (app-trigger); not required for `v0.1.0-sdk` (Decision Log 2026-07-25 / 2026-08-14)*
- [x] Schema IR + `SchemaId` + `applySchema` in the store (narrowed) — *full TS→IR / O2 compiler and secondary indexes remain open*

**Exit gate:** binding parity vectors, subscription/mutation/conflict lifecycle tests, artifact-size and baseline performance budgets (**E11 provisional**).  **Resolved 2026-08-14** (Decision Log; narrowed: schema IR + `applySchema`, edges/`listNodes`, facade, `applyCrdtVector` replay of `required/crdt/*`; subscribe/mutate/conflict covered by `m2-subscribe`/`m2-parity`/`m2-sync`. **E11 not claimed.**). Format freeze remains a separate, still-open Decision Log act — `v0.1.0-sdk` is an experimental-format release.

### M3 — Secure multi-peer sync (gates M3a → M3b → M3c)

**Outcome:** three peers converge through partition/reorder/retry over one WebSocket profile and a **durable (L2) reference relay**, then harden, then release `v0.1.0`.

Delivered as three independently auditable gates (amended 2026-07-18 from the delivery plan; resolves review findings CX-04/HX-08). **If the L2 relay cannot ship, M3 is renamed an online-sync preview and is not the first offline-first release** (plan DQ-9: durable catch-up is mandatory for `v0.1.0`).

#### M3a — Durable convergence (internal)

- [ ] **L2 reference relay**: durable persistence, receipt vs durable ack, full-oplog catch-up, GC off (ISSUES H11)
- [ ] Complete Merkle/delta wire protocol + delivery/ack/resume semantics (ISSUES C3, H4)
- [ ] Loss/reorder/partition/rejoin, three-peer offline catch-up, crash/restart — pre-provisioned signed test identities only

**Exit gate:** three-peer convergence with offline catch-up through the durable relay; exemplar **E2 live, E3**.

#### M3b — Security (internal)

- [ ] Mandatory signing policy, author-key resolution, datastore-membership admission (ISSUES C4, C5)
- [ ] Handshake hardening: fixed encoding through auth, transcript signature binding version/limits/transport (ISSUES H5; session resumption was removed in relay 0.2); signed peer handshake shared by direct P2P and relay participation (ISSUES H6)
- [ ] E2E encrypted-property envelope (M0-frozen bytes); recipient/group key distribution, rotation, revocation (ISSUES H10)
- [ ] Future-clock acceptance/quarantine rule (ISSUES H1); resource limits enforced pre-auth

**Exit gate:** malicious relay/peer negatives (forged ops, wrong datastore, clock abuse, auth bypass, resource limits); exemplar **E5–E8**.

#### M3c — Interop & release (`v0.1.0`)

- [ ] Independent TypeScript **wire peer** (evolved from the conformance model runner, still not NAPI-backed)
- [ ] Reference relay + conformance harness with golden/negative vectors in two languages (ISSUES H9)
- [ ] Version/upgrade matrix, packaging, support profile

**Exit gate:** two-language interoperability; partition/rejoin; duplicate/loss/reorder; release `v0.1.0`.

### M4 — Browser, P2P & evolution (tracks M4a / M4b)

**Outcome:** browser storage, WebRTC direct sync, cross-peer schema migration, snapshot bootstrap, and adjacent-version sync — without data loss. Two independent tracks:

**M4a — platform:**

- [ ] IndexedDB + OPFS adapters; WASM build within size budget (ISSUES O4); React hooks
- [ ] WebRTC direct sync using the shared peer protocol

**M4b — evolution:**

- [ ] Schema epochs across mixed-version peers (ISSUES C2, cross-peer half)
- [ ] Snapshot sync with authenticated snapshot format (ISSUES C7, snapshot half)
- [ ] Adjacent-version rollback/upgrade matrix
- [ ] Large-payload (`BlobRef`) transfer/storage implemented under the M0 reservation (ISSUES O1)

**Exit gate:** upgrade/downgrade/rollback matrix, mixed-schema peers (**E10**), snapshot + tail recovery, direct/relay parity, browser restart/offline tests.

### M5 — Production readiness & GA (program M5a / M5b / M5c)

**Outcome:** a deployable, observable, recoverable system with evidence for its published guarantees. Delivered as a focused three-part program:

**M5a — operability:**

- [ ] Backup/restore with restore drills; packaging/configuration; metrics/logs/traces; SLOs

**M5b — lifecycle safety:**

- [ ] Compaction & GC with causal frontiers and peer lifecycle (ISSUES C7 implementation — GC stays disabled until its partition/rejoin, forgotten-peer, late-op, and restore tests pass)
- [ ] Rolling upgrades across adjacent versions

**M5c — release assurance:**

- [ ] Fuzzing (decoders, ops, queries, migrations, sync state machines), load/soak, failure injection
- [ ] External security audit with **severity-based sign-off** (not "all findings closed")

**Parallel tracks (not GA gates):** unique-index conflict semantics or a durable won't-do decision (ISSUES H2); Richtext CRDT (Peritext-based, after O1 + M4b); Lean 4 proof artifacts (LWW, Counters, ORSet, MVRegister, RGA, HLC, sync completeness, migration safety) + Rust conformance to the Lean reference; performance benchmarks vs. GunDB, Automerge, Loro, Yjs.

**Exit gate:** restore drill, forgotten/offline-peer GC cases, rolling upgrade, published conformance suite, performance budgets, security sign-off.

### M6 — Ecosystem

Only after format and compatibility commitments are stable:

- [ ] Swift / Kotlin / Dart (Flutter) bindings over a shared stable C ABI from the Rust core
- [ ] Custom CRDT plugin policy
- [ ] Entity-level distributed ACLs (successor design to ISSUES C6)
- [ ] Hosted relay service (optional, SaaS)
- [ ] Visual graph inspector / admin UI
- [ ] Cypher query optimizer

---

## 11. Competitive Landscape

*The ZeroDB column states design goals, not demonstrated properties; the other columns describe shipped systems.*

| | GunDB | Automerge | Yjs | Loro | CR-SQLite | **ZeroDB** |
|---|---|---|---|---|---|---|
| **Data model** | Flat graph | JSON document | Shared types | JSON document | Relational (SQLite) | **Property graph** |
| **Conflict resolution** | HAM (wall-clock) | Causal CRDT | Causal CRDT | Eg-walker CRDT | Column CRDT | **Column CRDT + HLC** |
| **Clock** | Wall clock | Lamport | Lamport | Lamport | Lamport | **HLC** |
| **Offline sync** | Full state re-merge | Oplog delta | Oplog delta | Oplog delta | Changeset delta | **Merkle DAG delta** |
| **Storage** | localStorage/RAD | In-memory + save | In-memory + save | In-memory + save | SQLite | **IDB/OPFS/SQLite** |
| **Core language** | JavaScript | Rust/WASM | JavaScript | Rust/WASM | Rust/C | **Rust/WASM** |
| **CRDT variety** | LWW only | Doc-level | Doc-level | Doc-level | Column-level | **Column-level** |
| **Formal proofs** | None | None | None | None | None | **Lean 4 (planned)** |
| **Query language** | Chained .get() | None | None | None | SQL | **Cypher-inspired** |
| **Auth/crypto** | SEA (built-in) | None | None | None | None | **Built-in, swappable** |
| **Graph traversal** | Manual chaining | N/A | N/A | N/A | JOINs | **First-class traversal** |

---

## 12. Open Questions

All specification issues and open decisions are tracked by ID in **[ISSUES.md](ISSUES.md)** — Critical (C1–C8) and High (H1–H11) findings gate the roadmap milestones in §10 (M0 is packages **M0a–M0f**); resolved items move to the Decision Log there.  The open design questions, in brief:

| ID | Question | Decide by |
|----|----------|-----------|
| O1 | Large operation payload **transfer/storage protocol** (encoding reserved in M0a: caps + `BlobRef`, KERNEL §8; blocks Richtext) | M4 |
| O4 | WASM size budget; optional modules for RGA/Richtext | M4 |
| O6 | Protocol-level rate limiting (provisional size limits set in **M0a**, registry `limits`) | M3 |
| O7 | Causal `deps` scale — compact causal frontier + checkpoint translation | **M0f** contract; scale tests M5 |

O2 (TS authoring-canonical → IR identity-canonical) and O3 (minimal v0.1 query subset) were decided 2026-07-16 — see the Decision Log and [SCHEMA.md](SCHEMA.md).

---

## Appendix A: Glossary

| Term | Definition |
|------|------------|
| **Causal Graph** | The partial order of operations defined by their `deps` fields — captures happens-before relationships |
| **CRDTs** | Conflict-free Replicated Data Types — data structures that merge deterministically without coordination |
| **GCounter** | Grow-only counter CRDT — supports increment only, value is always ≥ 0 |
| **GroupId** | Identifier for an operation group — a set of operations applied atomically (see §2.8) |
| **HAM** | Hypothetical Amnesia Machine — GunDB's conflict resolution algorithm |
| **HLC** | Hybrid Logical Clock — combines physical time with logical counters for causality-preserving timestamps |
| **LWW** | Last-Writer-Wins — CRDT where the value with the latest timestamp wins |
| **Merkle Sync Tree** | Time-bucketed hash tree derived from the oplog, used for efficient delta sync negotiation |
| **MVRegister** | Multi-Value Register — CRDT register that preserves all concurrent values |
| **OpId** | Content hash (BLAKE3) of an operation — globally unique, used for deduplication and causal references |
| **Oplog** | Operation log — append-only record of all mutations |
| **ORSet** | Observed-Remove Set — CRDT set where add/remove are causally tracked |
| **Peritext** | Rich text CRDT algorithm that correctly handles concurrent formatting |
| **PNCounter** | Positive-Negative counter CRDT — supports both increment and decrement |
| **Quarantine** | Storage area for operations that fail ACL checks — kept but not materialized |
| **RGA** | Replicated Growable Array — ordered sequence CRDT |
| **SEC** | Strong Eventual Consistency — given the same set of operations, all peers converge to identical state |
| **UUIDv7** | UUID version 7 — time-ordered, globally unique identifier (RFC 9562) |


## Appendix B: References

- Kulkarni, S. et al. "Logical Physical Clocks and Consistent Snapshots in Globally Distributed Databases" (2014) — HLC paper
- Shapiro, M. et al. "Conflict-free Replicated Data Types" (2011) — foundational CRDT paper
- Litt, G. et al. "Peritext: A CRDT for Collaborative Rich Text Editing" (2021) — Ink & Switch
- Gentle, J. "Eg-walker: An Event Graph Walker for CRDTs" (2023) — Diamond Types / Loro foundation
- Kleppmann, M. "Making CRDTs Byzantine Fault Tolerant" (2022)
- CR-SQLite: https://vlcn.io — column-level CRDT selection in SQLite
- Loro: https://loro.dev — Rust/WASM CRDT library, Eg-walker architecture
- Automerge: https://automerge.org — Rust/WASM CRDT library
- GunDB: https://gun.eco — the predecessor this project aims to succeed