# ZeroDB — Technical Specification

**Version:** 0.1.0-draft
**Date:** 2026-03-19
**Author:** Matt / Turing Automations
**Status:** Draft — seeking contributors and co-architects

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
| Oplog with Merkle DAG | No operation log | Enables efficient delta sync after arbitrary offline periods. Peers exchange Merkle tree roots to identify divergence, then ship only missing operations. |
| Rust core / WASM bindings | Pure JavaScript | Identical behavior across browser, Node.js, mobile, and CLI. Follows Automerge and Loro's proven path. |
| Column-level CRDT selection | Everything is an LWW register | A `name` field is LWW, a `tags` field is an OR-Set, a `view_count` is a Counter. Schema declares intent; the engine enforces merge semantics. Inspired by CR-SQLite. |
| Lean formal proofs | No formal verification | Machine-checked proofs of CRDT convergence properties. The trust differentiator for safety-critical and financial applications. |

### Target Environments

- **Browser:** IndexedDB + OPFS storage, WASM core, WebSocket/WebRTC transport
- **Node.js:** SQLite storage, native Rust or WASM core, TCP/WebSocket transport
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
│  │ Clock  │ │  Engine  │ │  DAG   │ │   Layer     │  │
│  └────────┘ └──────────┘ └────────┘ └─────────────┘  │
│  ┌────────────────────┐ ┌──────────────────────────┐ │
│  │    Oplog Store     │ │     State Materializer   │ │
│  └────────────────────┘ └──────────────────────────┘ │
├──────────────────────────────────────────────────────┤
│                   Storage Adapter                    │
│    IndexedDB  ·  OPFS  ·  SQLite  ·  (pluggable)     │
├──────────────────────────────────────────────────────┤
│                   Transport Layer                    │
│    WebSocket  ·  WebRTC  ·  TCP  ·  (pluggable)      │
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
│   ├── zerodb-webrtc/    # WebRTC (browser P2P)
│   └── zerodb-tcp/       # TCP (server-to-server)
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
  id: NodeId;             // ULIDv2 — sortable, globally unique, embeds timestamp
  label: string;          // e.g. "User", "Document", "Session"
  properties: Record<string, CRDTValue>;  // each property has a CRDT type
  _meta: {
    created: HLCTimestamp;
    updated: HLCTimestamp;
    tombstone: boolean;   // soft delete — GC'd after causal stability
    origin: PeerId;       // peer that created this node
    signature?: Signature; // optional cryptographic signature
  };
}
```

#### Edge

```typescript
interface Edge {
  id: EdgeId;             // ULIDv2
  label: string;          // e.g. "FOLLOWS", "AUTHORED", "MEMBER_OF"
  source: NodeId;
  target: NodeId;
  properties: Record<string, CRDTValue>;
  _meta: {
    created: HLCTimestamp;
    updated: HLCTimestamp;
    tombstone: boolean;
    origin: PeerId;
    signature?: Signature;
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

**Comparison with GunDB's HAM:**

| Property | HAM | HLC |
|----------|-----|-----|
| Clock skew tolerance | Defers updates until local clock catches up — effectively blocks writes | Absorbs skew into logical counter; never blocks |
| Causal ordering | Not guaranteed across peers | Guaranteed after any message exchange |
| Tie-breaking | `JSON.stringify` lexicographic comparison | Deterministic: physical → logical → peer_id |
| Future-dated attacks | A peer can set clock to far future and block all others | Bounded drift detection; logical counter caps prevent runaway |

### 2.5 Oplog & Merkle DAG

Every mutation to the graph produces an **operation** appended to a local, append-only log:

```typescript
interface Operation {
  id: OpId;               // HLC timestamp of this operation
  peer: PeerId;           // originating peer
  deps: OpId[];           // causal dependencies (HLC of last-seen op per peer)
  entity: NodeId | EdgeId;
  field: string;          // property name, or "__tombstone" for deletes
  crdt_type: CRDTType;   // LWW | Counter | ORSet | MVRegister | ...
  payload: CRDTPayload;  // type-specific operation payload
  signature?: Signature;
}
```

Operations form a **Merkle DAG** — each operation references its causal dependencies by hash. This structure enables:

1. **Efficient delta sync:** Two peers compare Merkle roots. If they match, peers are in sync.  If not, they walk the DAG to find the divergence point and exchange only missing operations.
2. **Offline resilience:** A peer that's been offline for days/weeks/months can reconnect and efficiently sync by exchanging Merkle hashes — no need to replay the entire history.
3. **Integrity verification:** Any operation can be verified against its hash and its causal parents' hashes.  Tampered operations are detectable.

**Merkle Tree Structure:**

The oplog is partitioned into time-based buckets (configurable granularity, default: 1 minute).
Each bucket has a Merkle hash computed from its operations.
Buckets are organized in a balanced tree where:
- Leaf nodes = per-minute operation hashes
- Internal nodes = hash of children
- Root = single hash representing the entire oplog state

Sync protocol walks from root downward: if a subtree matches, skip it entirely.  This gives O(log N) sync negotiation where N is the number of time buckets since divergence.

**Why GunDB can't do this:**

GunDB has no oplog.
Each peer stores only the current materialized state.
After an extended offline period, the only sync mechanism is to re-exchange the entire state and re-run HAM conflict resolution.
There's no concept of "what changed since we last talked."

### 2.6 State Materialization

ZeroDB maintains two views of data:

1. **Oplog (source of truth):** The append-only log of all operations, organized as a Merkle DAG.  This is what syncs between peers.
2. **Materialized state (read cache):** The computed current state of the graph, derived by replaying the oplog through CRDT merge functions.  This is what applications query.

The materializer is an incremental engine:
when new operations arrive (locally or via sync), it applies only the new operations to the existing materialized state rather than replaying from scratch.

**Consistency guarantee:** The materialized state is always a deterministic function of the oplog.  Given the same set of operations (in any order), every peer computes the identical materialized state.  This is the Strong Eventual Consistency (SEC) guarantee that CRDTs provide.

---

## 3. CRDT Type System & Schema DSL

### 3.1 Column-Level CRDT Selection

ZeroDB's key expressiveness advantage over GunDB is that each property on a node or edge declares its own CRDT merge strategy.
The schema is a contract: "when concurrent edits happen to this field, here's how they merge."

**Available CRDT types:**

| Type | Merge Semantics | Use Case |
|------|----------------|----------|
| `LWW<T>` | Last-Writer-Wins register. Latest HLC timestamp wins. | Names, titles, settings — any field where "most recent edit wins" is correct. |
| `Counter` | Grow-only or PN-Counter. Concurrent increments are summed, not overwritten. | View counts, vote tallies, inventory quantities. |
| `ORSet<T>` | Observed-Remove Set. Add and remove are both tracked causally. Concurrent add+remove = element is present. | Tags, labels, members lists, permission sets. |
| `MVRegister<T>` | Multi-Value Register. Concurrent writes produce multiple values; application resolves. | Fields where conflicts must be surfaced to the user (e.g., conflicting title edits). |
| `LWWMap<K, V>` | Map where each key is an independent LWW register. | Metadata bags, preferences, configuration. |
| `RGA<T>` | Replicated Growable Array. Ordered sequence with positional insert/delete. | Ordered lists, playlists, task orderings. |
| `Richtext` | Peritext-style rich text CRDT. Character-level insert/delete + formatting ranges. | Document content, comments, descriptions. |
| `Flag` | Enable-Wins Flag. Concurrent enable + disable = enabled. | Feature flags, active/inactive status where enabling should win. |

### 3.2 Schema Definition

Schemas are defined in TypeScript (for the SDK) and a compact DSL (for the CLI and migration tooling):

**TypeScript SDK:**

```typescript
import { z, schema, LWW, Counter, ORSet, MVRegister, RGA } from 'zerodb';

const User = schema.node('User', {
  name:        LWW(z.string()),          // last write wins
  email:       LWW(z.string().email()),   // with validation
  bio:         LWW(z.string().optional()),
  tags:        ORSet(z.string()),          // add/remove tracked causally
  loginCount:  Counter(),                  // concurrent increments merge
  settings:    LWWMap(z.string(), z.any()),
});

const Post = schema.node('Post', {
  title:       MVRegister(z.string()),     // surface conflicts to user
  body:        Richtext(),                 // collaborative editing
  viewCount:   Counter(),
  tags:        ORSet(z.string()),
  published:   LWW(z.boolean()),
});

const Authored = schema.edge('AUTHORED', {
  source: User,
  target: Post,
  role:   LWW(z.enum(['author', 'editor', 'contributor'])),
});
```

**CLI DSL (`.zerodb` schema files):**

```
node User {
  name        LWW<string>
  email       LWW<string>
  bio         LWW<string?>
  tags        ORSet<string>
  loginCount  Counter
  settings    LWWMap<string, any>
}

node Post {
  title       MVRegister<string>
  body        Richtext
  viewCount   Counter
  tags        ORSet<string>
  published   LWW<bool>
}

edge AUTHORED (User -> Post) {
  role        LWW<"author" | "editor" | "contributor">
}
```

### 3.3 Schema Evolution

Schemas evolve through **migrations** — declarative operations that transform the graph:

```typescript
// Migration 001: Add 'avatar' to User
migration('001_add_avatar', {
  addProperty: { node: 'User', name: 'avatar', type: LWW(z.string().url().optional()) }
});

// Migration 002: Change 'title' from MVRegister to LWW
migration('002_simplify_title', {
  changeType: { node: 'Post', name: 'title', from: 'MVRegister', to: 'LWW', 
                resolver: (values) => values[0] }  // pick first on downgrade
});
```

**Rules:**

- Adding a property is always safe — existing nodes get the CRDT's zero value.
- Removing a property is soft — the field becomes invisible in the schema but persists in the oplog for sync compatibility.
- Changing a CRDT type requires a resolver function to convert existing state.
- Migrations are themselves operations in the oplog, so they propagate to all peers.

---

## 4. Sync Protocol

### 4.1 Sync Lifecycle

```
Peer A                          Peer B
  │                               │
  ├── SyncRequest ───────────────►│  (Merkle root + version vector)
  │                               │
  │◄── SyncResponse ──────────────┤  (Merkle root + version vector)
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
  ├── LiveOp ◄──────────────────► │  (bidirectional real-time stream)
```

### 4.2 Sync Modes

| Mode | Trigger | Behavior |
|------|---------|----------|
| **Cold sync** | Peer reconnects after offline period | Full Merkle tree comparison → delta exchange |
| **Warm sync** | Peer connects to a known-recent peer | Version vector comparison → delta exchange (skip Merkle walk) |
| **Live sync** | Peers already synchronized | Real-time bidirectional operation streaming |
| **Snapshot sync** | New peer joins with no history | Download a compressed state snapshot + recent oplog tail |

### 4.3 Version Vectors

Each peer maintains a **version vector** — a map of `PeerId → latest HLC timestamp seen from that peer`.
During warm sync, two peers exchange version vectors and each can immediately compute which operations the other is missing, without walking the Merkle tree.

```typescript
type VersionVector = Map<PeerId, HLCTimestamp>;

// Peer A's vector: { A: 1050, B: 1020, C: 990 }
// Peer B's vector: { A: 1030, B: 1050, C: 1000 }
// 
// A needs from B: B's ops after 1020, C's ops after 990
// B needs from A: A's ops after 1030
```

### 4.4 Transport Agnosticism

The sync protocol operates on abstract streams of operations. The transport layer is pluggable:

- **WebSocket:** Primary for browser-to-relay and Node-to-Node.
- **WebRTC DataChannel:** Browser-to-browser direct P2P.
- **TCP:** Server-to-server federation.
- **Custom:** Implement the `Transport` trait for exotic environments (Bluetooth, serial, carrier pigeon).

### 4.5 Relay Servers

ZeroDB is peer-to-peer, but relay servers exist for:

- **Peer discovery:** A lightweight signaling server for WebRTC negotiation.
- **Always-on persistence:** A relay that stores the full oplog serves as a backup and catch-up point for peers that go offline.
- **Fan-out efficiency:** Rather than every peer connecting to every other, relays aggregate and redistribute operations.

Relays are untrusted — they cannot forge operations (signatures verify origin) and they cannot censor without detection (Merkle roots must match). A peer can use any relay, run their own, or operate without one in direct P2P mode.

---

## 5. CLI & Developer Experience

### 5.1 Design Philosophy

The `zerodb` CLI is the primary developer interface for administration, debugging, and scripting. It follows the ergonomic patterns of tools like `git`, `wrangler`, and `turso`:

```bash
# Initialize a new ZeroDB project
zerodb init my-app

# Apply schema from .zerodb files
zerodb schema apply

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

ZeroDB uses a Cypher-inspired query syntax for the CLI REPL and programmatic queries:

```cypher
-- Find all posts by a user
MATCH (u:User {name: "Alice"})-[:AUTHORED]->(p:Post)
RETURN p.title, p.viewCount
ORDER BY p.viewCount DESC

-- Find mutual followers
MATCH (a:User {name: "Alice"})-[:FOLLOWS]->(b:User)-[:FOLLOWS]->(a)
RETURN b.name

-- Traverse with edge properties
MATCH (u:User)-[a:AUTHORED {role: "editor"}]->(p:Post)
WHERE p.published = true
RETURN u.name, p.title, a.role
```

The query engine compiles Cypher to optimized graph traversals over the materialized state.
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
  p.viewCount.increment(1);     // Counter: concurrent increments merge
  p.tags.add('featured');        // ORSet: concurrent add+remove = present
  p.title.set('Updated Title'); // MVRegister: concurrent sets = multi-value
});

// Inspect CRDT state directly
const titleState = await db.crdtState(post, 'title');
// MVRegister: { values: ['Updated Title'], conflicts: false }
// If two peers set title concurrently:
// { values: ['Title A', 'Title B'], conflicts: true }

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

---

## 6. Cryptography & Auth

### 6.1 Identity Model

Each peer generates an **Ed25519 keypair** on first run. The public key hash serves as the `PeerId`. All operations can optionally be signed, creating a verifiable chain of authorship.

```
PeerId = BLAKE3(Ed25519PublicKey)  // 32 bytes, truncated to 16 for display
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

Applications can swap in their own provider — e.g., hardware security modules, institutional PKI, or WebAuthn-backed keys.

---

## 7. Storage Layer

### 7.1 Storage Adapter Trait

```rust
#[async_trait]
pub trait StorageAdapter: Send + Sync {
    // Oplog operations
    async fn append_ops(&self, ops: &[Operation]) -> Result<()>;
    async fn read_ops(&self, range: OpRange) -> Result<Vec<Operation>>;
    async fn ops_since(&self, version: &VersionVector) -> Result<Vec<Operation>>;
    
    // Materialized state
    async fn get_node(&self, id: &NodeId) -> Result<Option<Node>>;
    async fn get_edge(&self, id: &EdgeId) -> Result<Option<Edge>>;
    async fn query_nodes(&self, label: &str, filter: &Filter) -> Result<Vec<Node>>;
    async fn query_edges(&self, label: &str, filter: &Filter) -> Result<Vec<Edge>>;
    async fn traverse(&self, start: &NodeId, pattern: &TraversalPattern) -> Result<Vec<Path>>;
    async fn put_materialized(&self, entity: &Entity) -> Result<()>;
    
    // Merkle tree
    async fn merkle_root(&self) -> Result<Hash>;
    async fn merkle_subtree(&self, depth: usize) -> Result<MerkleTree>;
    
    // Housekeeping
    async fn compact(&self) -> Result<CompactionStats>;
    async fn snapshot(&self) -> Result<Snapshot>;
}
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

- **Causal stability pruning:** Once an operation has been acknowledged by all known peers (its HLC is below every peer's version vector entry), the raw operation can be discarded and only its effect on materialized state is retained.
- **Tombstone GC:** Deleted nodes/edges (tombstoned) are fully removed after all peers have seen the delete.
- **Snapshot checkpointing:** Periodically, the materializer writes a full state snapshot.  The oplog can be truncated to only contain operations after the snapshot.

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
| **Malicious peer injects forged operations** | Ed25519 signatures on all operations; unsigned ops rejected by default |
| **Relay censors or drops operations** | Merkle root comparison detects omissions; peers can switch relays |
| **Replay attack** | HLC monotonicity + operation deduplication by OpId |
| **Clock manipulation** | HLC bounded drift detection; logical counter caps prevent runaway timestamps |
| **Data exfiltration from relay** | E2E encrypted properties; relay sees only ciphertext |
| **Sybil attack (fake peers flood network)** | Rate limiting per PeerId; optional proof-of-work for peer registration |

### 9.2 Access Control

ZeroDB supports **capability-based access control** at the schema level:

```typescript
const Post = schema.node('Post', {
  title:  LWW(z.string()),
  body:   Richtext(),
}, {
  acl: {
    write: (peer, node) => node._meta.origin === peer.id 
                        || peer.hasCap('post:edit'),
    read:  (peer, node) => node.properties.published 
                        || node._meta.origin === peer.id,
  }
});
```

ACL evaluation happens at the storage layer — unauthorized operations are rejected before entering the oplog.

---

## 10. Roadmap

### Phase 1: Foundation

- [ ] Rust core: HLC, LWW register, Counter, ORSet
- [ ] Oplog with Merkle DAG (in-memory)
- [ ] SQLite storage adapter
- [ ] CLI: `init`, `schema apply`, `repl`, basic queries
- [ ] TypeScript SDK: WASM build, `open`, `create`, `query`, `mutate`
- [ ] Cold sync + live sync over WebSocket

### Phase 2: Browser & DX

- [ ] IndexedDB + OPFS storage adapters
- [ ] React hooks package
- [ ] MVRegister, RGA, LWWMap CRDT types
- [ ] Schema migrations
- [ ] CLI: `oplog`, `inspect`, `peers`, `keys`
- [ ] WebRTC P2P transport

### Phase 3: Production Hardening

- [ ] Richtext CRDT (Peritext-based)
- [ ] Compaction & garbage collection
- [ ] E2E encryption + key rotation
- [ ] Snapshot sync for new peers
- [ ] Relay server reference implementation
- [ ] Performance benchmarks vs. GunDB, Automerge, Loro, Yjs

### Phase 4: Formal Verification & Trust

- [ ] Lean 4 proofs: LWW, Counter, ORSet convergence
- [ ] Lean 4 proofs: HLC correctness
- [ ] Lean 4 proofs: Merkle sync completeness
- [ ] Published proof artifacts + independent verification guide
- [ ] Security audit (external)

### Phase 5: Ecosystem

- [ ] Swift / Kotlin bindings for mobile
- [ ] Plugin system for custom CRDT types
- [ ] Hosted relay service (optional, SaaS)
- [ ] Visual graph inspector (web UI)
- [ ] Cypher query optimizer

---

## 11. Competitive Landscape

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

These are design decisions that need further research or community input:

1. **CRDT garbage collection granularity:** Per-entity or per-time-bucket?  Per-entity is more precise but requires tracking per-entity causal stability.

2. **Partial replication:** Should peers be able to subscribe to subsets of the graph? (e.g., "I only want User and Post nodes, not analytics"). This complicates the Merkle tree structure significantly.

3. **Schema enforcement strictness:** Should schemaless mode exist for prototyping? Or always require schema (with a `z.any()` escape hatch)?

4. **Query language scope:** Full Cypher, or a minimal subset?  Full Cypher is a large specification with OPTIONAL MATCH, aggregation, path patterns, etc.

5. **WASM binary size budget:** Automerge's WASM is ~250KB gzipped. Loro is ~200KB. What's our target? Each additional CRDT type adds size.

6. **Relay protocol standardization:** Should the relay protocol be its own spec, enabling third-party relay implementations?

7. **Backwards compatibility with GunDB:** Is a migration path from GunDB worth engineering, or is a clean break cleaner?

---

## Appendix A: Glossary

| Term | Definition |
|------|------------|
| **CRDTs** | Conflict-free Replicated Data Types — data structures that merge deterministically without coordination |
| **HLC** | Hybrid Logical Clock — combines physical time with logical counters for causality-preserving timestamps |
| **HAM** | Hypothetical Amnesia Machine — GunDB's conflict resolution algorithm |
| **Oplog** | Operation log — append-only record of all mutations |
| **Merkle DAG** | Directed Acyclic Graph where each node is content-addressed by its hash |
| **SEC** | Strong Eventual Consistency — given the same set of operations, all peers converge to identical state |
| **LWW** | Last-Writer-Wins — CRDT where the value with the latest timestamp wins |
| **ORSet** | Observed-Remove Set — CRDT set where add/remove are causally tracked |
| **MVRegister** | Multi-Value Register — CRDT register that preserves all concurrent values |
| **RGA** | Replicated Growable Array — ordered sequence CRDT |
| **Peritext** | Rich text CRDT algorithm that correctly handles concurrent formatting |
| **ULID** | Universally Unique Lexicographically Sortable Identifier |
| **Version Vector** | Map of peer → latest timestamp, used for efficient delta computation |

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