# ZeroDB Relay Protocol Specification

**Version:** 0.1.0-draft
**Date:** 2026-03-19
**Author:** Matt / Turing Automations
**Status:** Draft
**Companion to:** [ZeroDB Technical Specification](SPEC.md)

---

## 1. Introduction & Scope

This document specifies the ZeroDB relay protocol — the communication protocol between ZeroDB peers and relay servers. It is designed to enable third-party relay implementations in any language that interoperate with any conforming ZeroDB peer.

### 1.1 Relationship to SPEC.md

This specification is a **sister document** to the [ZeroDB Technical Specification](SPEC.md). It defines relay-specific behavior: wire format, message types, authentication, routing, and operational concerns. It references SPEC.md for shared data structures and peer-side behavior rather than duplicating them.

**Shared types defined in SPEC.md:**

| Type | SPEC.md Section |
|------|----------------|
| `Operation` | §2.5 |
| `HLCTimestamp` | §2.4 |
| `OpId` (BLAKE3 content hash) | §2.5 |
| `PeerId` (BLAKE3 of Ed25519 pubkey) | §6.1 |
| `MerkleTree` / `MerkleRoot` | §2.6 |
| `GroupId` | §2.8 |
| `Signature`, `PublicKey` (Ed25519) | §6.1–6.2 |

### 1.2 Terminology

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT", "SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this document are to be interpreted as described in [RFC 2119](https://www.rfc-editor.org/rfc/rfc2119).

- **Peer:** A ZeroDB client instance (browser, Node.js, CLI, mobile).
- **Relay:** A server implementing this protocol. Does not run application logic or CRDT merges.
- **Datastore:** An independent unit of replication with its own oplog and Merkle tree (see SPEC.md §4.4, resolved open question #5).

### 1.3 Non-Goals

This specification does **not** define:

- Peer-to-peer direct sync behavior (see SPEC.md §4)
- CRDT merge semantics (see SPEC.md §3)
- Storage engine internals for relay persistence
- Application-level access control evaluation (see SPEC.md §9.2)

---

## 2. Conformance Levels

Relay implementations declare one of three conformance levels. Each level is a strict superset of the previous.

### Level 0 — Signal Relay

Minimal implementation for peer discovery and WebRTC signaling.

**Capabilities:**
- Accept peer connections and authenticate identity
- Forward WebRTC signaling messages between peers
- Respond to peer list queries
- Keepalive (PING/PONG)

**Does NOT:** store operations, participate in sync, or forward live operations.

A Level 0 relay can be implemented in ~200 lines of code in any language with WebSocket support.

### Level 1 — Stateless Relay

Signal relay plus live operation forwarding.

**Capabilities (in addition to L0):**
- Accept datastore subscriptions from peers
- Forward live operations to subscribed peers (fan-out)
- Validate operation signatures before forwarding
- Deduplicate operations by OpId
- Enforce rate limits and backpressure

**Does NOT:** persist operations or participate in Merkle sync. If the relay restarts, no history is available.

### Level 2 — Persistent Relay

Full relay with oplog persistence and sync participation. This is the "always-on peer" described in SPEC.md §4.4.

**Capabilities (in addition to L1):**
- Persist all operations to durable storage
- Maintain a Merkle sync tree per datastore
- Participate in the full sync protocol (Merkle sync, delta exchange)
- Serve snapshot sync to new peers
- Compact oplog per SPEC.md §7.3 rules

---

## 3. Wire Format

### 3.1 Framing

The protocol operates over byte-stream or message-oriented transports.

- **Message-oriented transports** (WebSocket binary frames, WebRTC DataChannel): each transport message carries exactly one protocol message. No additional framing is needed.
- **Stream transports** (TCP): messages are length-prefixed with a 4-byte big-endian unsigned integer indicating the payload length in bytes, followed by the payload.

### 3.2 Serialization

The canonical serialization format is **CBOR** ([RFC 8949](https://www.rfc-editor.org/rfc/rfc8949)). CBOR is binary, compact, well-specified, and has broad cross-language support.

- All protocol messages MUST be serializable to and from CBOR.
- Relay implementations SHOULD also support **JSON** serialization as a debug and development mode. Peers MAY negotiate JSON mode during the handshake (see §4.1).
- The serialization mode is established during the `HELLO`/`WELCOME` exchange and applies for the duration of the session.

### 3.3 Message Envelope

Every protocol message shares a common envelope structure:

```
{
  type:       uint8       // Message type discriminator (see §4)
  version:    uint8       // Protocol version (currently 1)
  request_id: uint32      // Request/response correlation ID (0 for unsolicited messages)
  payload:    map         // Type-specific payload fields
}
```

The `request_id` field enables request/response correlation. A response message carries the same `request_id` as the request that triggered it. Unsolicited messages (e.g., `LIVE_OP` fan-out) use `request_id: 0`.

---

## 4. Message Types

Messages are grouped by protocol phase. Each message type is annotated with:
- **Direction:** `P→R` (peer to relay), `R→P` (relay to peer), or `↔` (bidirectional)
- **Level:** minimum conformance level required (`L0`, `L1`, `L2`)

### 4.1 Connection & Authentication

#### `HELLO` (0x01) — P→R [L0]

Initiates a connection. Sent by the peer immediately after transport establishment.

```
{
  peer_id:      PeerId          // Claimed peer identity
  public_key:   bytes           // Ed25519 public key (32 bytes)
  protocol_version: uint8       // Requested protocol version
  features:     [string]        // Optional feature flags (e.g., "json-debug")
  datastores:   [string]?       // Optional: datastores to subscribe to immediately
}
```

#### `CHALLENGE` (0x02) — R→P [L0]

Relay sends a random nonce for the peer to sign, proving ownership of the claimed private key.

```
{
  nonce:        bytes           // 32 random bytes
  relay_id:     PeerId?         // Relay's own PeerId (if relay supports mutual auth)
  relay_pubkey: bytes?          // Relay's Ed25519 public key (if mutual auth)
}
```

#### `AUTH` (0x03) — P→R [L0]

Peer signs the challenge nonce.

```
{
  signature:    Signature       // Ed25519 signature over the nonce
}
```

The relay MUST verify:
1. The signature is valid for the nonce and the public key from `HELLO`
2. `BLAKE3(public_key) == peer_id` from `HELLO`

If verification fails, the relay MUST respond with `ERROR` and close the connection.

#### `WELCOME` (0x04) — R→P [L0]

Sent after successful authentication. Establishes the session.

```
{
  session_id:       string          // Opaque session identifier
  relay_level:      uint8           // Conformance level (0, 1, or 2)
  relay_features:   [string]        // Supported features
  serialization:    string          // Negotiated serialization ("cbor" or "json")
  limits: {
    max_payload_bytes:   uint32     // Maximum operation payload size
    max_batch_size:      uint16     // Maximum operations per batch
    max_subscriptions:   uint16     // Maximum concurrent datastore subscriptions
    ops_per_second:      uint32?    // Per-peer rate limit (null = unlimited)
  }
  known_relays:     [RelayInfo]?    // Optional list of other known relays
  merkle_roots:     map?            // Datastore ID → MerkleRoot (L2 only, for datastores requested in HELLO)
}
```

```
RelayInfo = {
  url:    string        // Relay connection URL
  level:  uint8         // Conformance level
  pubkey: bytes?        // Relay's public key (if known)
}
```

#### `ERROR` (0xFF) — ↔ [L0]

Signals an error. May be sent at any point during the session.

```
{
  code:     uint16      // Error code (see §10)
  message:  string      // Human-readable description
  fatal:    bool        // If true, the sender will close the connection
}
```

#### `GOODBYE` (0xFE) — ↔ [L0]

Clean disconnection.

```
{
  reason:   uint16      // Reason code (0 = normal, 1 = going offline, 2 = switching relay, ...)
  message:  string?     // Optional human-readable message
}
```

After sending `GOODBYE`, the sender SHOULD close the transport connection. The receiver SHOULD NOT send further messages after receiving `GOODBYE`.

### 4.2 Datastore Subscription

#### `SUBSCRIBE` (0x10) — P→R [L1]

Subscribe to receive operations for one or more datastores.

```
{
  datastores: [string]  // Datastore IDs to subscribe to
}
```

#### `SUBSCRIBED` (0x11) — R→P [L1]

Confirms subscription. Sent once per `SUBSCRIBE` request.

```
{
  datastores: [DatastoreInfo]
}
```

```
DatastoreInfo = {
  id:           string
  peer_count:   uint32          // Number of currently connected peers
  merkle_root:  MerkleRoot?     // Current Merkle root (L2 only)
}
```

#### `UNSUBSCRIBE` (0x12) — P→R [L1]

Stop receiving operations for the specified datastores.

```
{
  datastores: [string]
}
```

### 4.3 Sync Protocol (L2)

These messages mirror the sync lifecycle defined in SPEC.md §4.1, adapted for relay participation. A Level 2 relay participates in sync as a peer — it has its own Merkle tree and oplog.

Level 0 and Level 1 relays MUST reject these messages with an `ERROR` (code `0x401`).

#### `SYNC_REQUEST` (0x20) — ↔ [L2]

Initiates Merkle sync for a datastore.

```
{
  datastore:    string
  merkle_root:  MerkleRoot      // Sender's current Merkle root
}
```

#### `SYNC_RESPONSE` (0x21) — ↔ [L2]

Response to `SYNC_REQUEST` with the receiver's Merkle root.

```
{
  datastore:    string
  merkle_root:  MerkleRoot
  match:        bool            // True if roots match (already in sync)
}
```

If `match` is true, both sides skip delta exchange and enter live mode.

#### `DELTA_REQUEST` (0x22) — ↔ [L2]

Request operations for divergent Merkle subtrees.

```
{
  datastore:        string
  missing_hashes:   [Hash]      // Subtree hashes the sender does not have
}
```

#### `DELTA_BATCH` (0x23) — ↔ [L2]

A batch of operations in response to `DELTA_REQUEST`.

```
{
  datastore:    string
  operations:   [Operation]     // Operations (see SPEC.md §2.5)
  remaining:    uint32          // Estimated remaining operations (0 = last batch)
}
```

The sender MUST respect the receiver's `max_batch_size` and `max_payload_bytes` limits from `WELCOME`. If the delta exceeds a single batch, multiple `DELTA_BATCH` messages are sent with `remaining > 0` until the final batch (`remaining = 0`).

#### `SYNC_ACK` (0x24) — ↔ [L2]

Confirms convergence after delta exchange.

```
{
  datastore:    string
  merkle_root:  MerkleRoot      // Sender's updated Merkle root (should now match)
}
```

### 4.4 Live Operation Relay

#### `LIVE_OP` (0x30) — P→R [L1]

A peer sends a single operation to the relay for distribution.

```
{
  datastore:    string
  operation:    Operation       // See SPEC.md §2.5
}
```

#### `LIVE_OP_BATCH` (0x31) — ↔ [L1]

Multiple operations sent together. Used for operation groups (shared `GroupId`) and relay fan-out batching.

```
{
  datastore:    string
  operations:   [Operation]
}
```

Operations in a batch sharing a `GroupId` MUST be kept together during forwarding (see §6.4).

#### `OP_ACK` (0x32) — R→P [L1]

Relay acknowledges receipt of operation(s).

```
{
  op_ids:       [OpId]          // Acknowledged operation IDs
  merkle_root:  MerkleRoot?     // Updated Merkle root (L2 only)
}
```

#### `RELAY_OP` (0x33) — R→P [L1]

Relay forwards operation(s) from other peers to a subscribed peer. Structurally identical to `LIVE_OP_BATCH` but distinguished by message type to indicate relay-originated delivery.

```
{
  datastore:    string
  operations:   [Operation]
}
```

### 4.5 Peer Discovery & Signaling

#### `PEER_ANNOUNCE` (0x40) — P→R [L0]

Announce presence on a datastore. Used for peer discovery even without subscription (L0 relays).

```
{
  datastores:   [string]        // Datastores this peer is interested in
  connectable:  bool            // Whether this peer accepts direct P2P connections
  metadata:     map?            // Optional: transport hints (e.g., STUN candidates)
}
```

#### `PEER_LIST_REQUEST` (0x41) — P→R [L0]

Request the list of peers connected to a datastore.

```
{
  datastore:    string
}
```

#### `PEER_LIST_RESPONSE` (0x42) — R→P [L0]

```
{
  datastore:    string
  peers: [{
    peer_id:      PeerId
    connectable:  bool
    metadata:     map?
  }]
}
```

#### `SIGNAL_OFFER` (0x43) — P→R→P [L0]

WebRTC signaling: session description offer. The relay forwards this to the target peer without inspecting content.

```
{
  target:   PeerId      // Intended recipient
  payload:  bytes       // Opaque signaling data (SDP offer)
}
```

#### `SIGNAL_ANSWER` (0x44) — P→R→P [L0]

WebRTC signaling: session description answer.

```
{
  target:   PeerId
  payload:  bytes       // Opaque signaling data (SDP answer)
}
```

#### `SIGNAL_ICE` (0x45) — P→R→P [L0]

WebRTC signaling: ICE candidate exchange.

```
{
  target:   PeerId
  payload:  bytes       // Opaque ICE candidate data
}
```

For all signaling messages, the relay MUST:
1. Verify the sender is authenticated
2. Look up the target peer by `PeerId`
3. Forward the message with the sender's `PeerId` attached
4. Respond with `ERROR` if the target peer is not connected

### 4.6 Administrative

#### `PING` (0x50) / `PONG` (0x51) — ↔ [L0]

Keepalive. The sender SHOULD send `PING` at a regular interval (RECOMMENDED: every 30 seconds). The receiver MUST respond with `PONG`. If no `PONG` is received within a timeout (RECOMMENDED: 60 seconds), the connection SHOULD be considered dead.

```
// PING
{ timestamp: uint64 }      // Sender's wall-clock timestamp (for latency estimation)

// PONG
{ timestamp: uint64 }      // Echo of the PING timestamp
```

#### `STATUS_REQUEST` (0x52) — P→R [L0]

Request relay health and status information.

```
{}
```

#### `STATUS_RESPONSE` (0x53) — R→P [L0]

```
{
  relay_level:      uint8
  uptime_seconds:   uint64
  peer_count:       uint32
  datastore_count:  uint32
  ops_per_second:   float32         // Recent throughput
  version:          string          // Relay software version
  protocol_version: uint8
}
```

#### `RATE_LIMIT` (0x54) — R→P [L1]

Notifies a peer that it has exceeded rate limits.

```
{
  retry_after_ms:   uint32      // Minimum delay before resuming sends
  limit_type:       string      // "ops_per_second" | "bytes_per_second" | "batch_size"
  current:          uint32      // The peer's current rate
  allowed:          uint32      // The allowed rate
}
```

#### `BACKPRESSURE` (0x55) — R→P [L1]

Relay is overwhelmed and requests all peers slow down. Unlike `RATE_LIMIT` (which targets a specific peer exceeding its limits), `BACKPRESSURE` is a system-wide signal.

```
{
  delay_ms:     uint32          // Suggested delay between sends
  reason:       string?         // Optional: "queue_depth" | "memory" | "io"
}
```

---

## 5. Authentication Handshake

### 5.1 Handshake Sequence

```
Peer                            Relay
  │                               │
  ├── [transport connect] ───────►│
  │                               │
  ├── HELLO ─────────────────────►│  (peer_id, public_key, features)
  │                               │
  │◄── CHALLENGE ─────────────────┤  (nonce, optional relay_id)
  │                               │
  ├── AUTH ──────────────────────►│  (signature over nonce)
  │                               │
  │◄── WELCOME ───────────────────┤  (session_id, limits, features)
  │                               │
  │  [session established]        │
```

### 5.2 Identity Verification

The handshake proves the peer controls the Ed25519 private key corresponding to their claimed `PeerId`:

1. Peer sends `HELLO` with their `peer_id` and `public_key`.
2. Relay generates 32 cryptographically random bytes as a `nonce`.
3. Peer signs the nonce with their Ed25519 private key.
4. Relay verifies: `Ed25519.verify(nonce, signature, public_key) == true`
5. Relay verifies: `BLAKE3(public_key) == peer_id`

If either check fails, the relay MUST respond with `ERROR` (code `0x201`) and close the connection.

### 5.3 Mutual Authentication (Optional)

Peers MAY request that the relay also prove its identity. If the relay includes `relay_id` and `relay_pubkey` in the `CHALLENGE` message, the peer can verify the relay's identity. The relay signs a concatenation of the nonce and the peer's `peer_id`:

```
relay_proof = Ed25519.sign(relay_private_key, nonce || peer_id)
```

This is included in the `WELCOME` message as an additional `relay_signature` field. Mutual auth protects against relay impersonation when peers are configured with a specific relay's public key.

### 5.4 Session Tokens

After successful authentication, the `WELCOME` message includes a `session_id`. Relay implementations MAY support reconnection using the session token to skip re-authentication, subject to an expiry window (RECOMMENDED: 5 minutes).

To reconnect with a session token, the peer sends `HELLO` with an additional `session_id` field. If the relay accepts the session, it skips `CHALLENGE`/`AUTH` and proceeds directly to `WELCOME`.

### 5.5 Transport Security

Relay connections SHOULD use TLS (for WebSocket: `wss://`; for TCP: TLS wrapper). TLS provides transport-level encryption and server authentication via certificates.

**Important:** TLS does NOT replace ZeroDB's operation-level E2E encryption (SPEC.md §6.2). TLS protects the transport; E2E encryption protects operation payloads from the relay itself.

---

## 6. Fan-Out & Message Routing

### 6.1 Routing Model

The relay maintains a **subscription table**: a mapping from `datastore_id` to the set of connected `PeerId`s subscribed to that datastore.

When the relay receives a `LIVE_OP` or `LIVE_OP_BATCH` from a peer:

1. Validate the operation(s) per §9
2. Acknowledge receipt with `OP_ACK`
3. Persist the operation(s) if L2
4. Forward via `RELAY_OP` to all other peers subscribed to the same datastore

The relay MUST NOT forward operations back to the peer that sent them.

### 6.2 Deduplication

The relay MUST deduplicate operations by `OpId`. If an operation with a given `OpId` has already been received (from any peer), it MUST NOT be forwarded again.

Implementation: the relay maintains a set of recently seen `OpId`s. For L1 relays (no persistence), this set MAY be bounded (e.g., last 100,000 OpIds). For L2 relays, the oplog itself serves as the deduplication index.

### 6.3 Causal Ordering

The relay SHOULD forward operations in causal order — an operation should not be forwarded before its dependencies (`deps` field) have been forwarded.

- **L2 relays:** SHOULD buffer operations with unmet dependencies and forward them once dependencies arrive. If dependencies do not arrive within a timeout (RECOMMENDED: 30 seconds), forward anyway — the receiving peer can handle out-of-order operations.
- **L1 relays:** MAY forward operations immediately regardless of causal order, as they lack the state to track dependencies.

### 6.4 Group Atomicity

Operations sharing a `GroupId` (see SPEC.md §2.8) SHOULD be forwarded together in a single `RELAY_OP` batch.

- If all operations in a group arrive in a single `LIVE_OP_BATCH`, the relay forwards them as a single `RELAY_OP`.
- If group operations arrive across multiple messages, the relay SHOULD buffer until the group is complete. The relay MUST impose a timeout on group buffering (RECOMMENDED: 5 seconds) to prevent indefinite buffering from incomplete groups.

### 6.5 Fan-Out Efficiency

The relay MAY batch operations from multiple peers into a single `RELAY_OP` for downstream peers, provided:
- Causal ordering is preserved within each datastore
- Group atomicity is preserved (operations with the same `GroupId` stay together)
- The batch does not exceed the receiver's `max_batch_size`

---

## 7. Persistence Requirements (Level 2)

### 7.1 Oplog Storage

A Level 2 relay MUST persist all validated operations to durable storage, keyed by `OpId`. The storage MUST support:

- **Append:** Store new operations
- **Lookup by OpId:** Retrieve a specific operation by its content hash
- **Range query by HLC:** Retrieve operations within an HLC timestamp range (for time-bucket Merkle tree construction and delta serving)
- **Iteration by datastore:** Enumerate all operations belonging to a datastore

The specification does NOT mandate a specific storage engine. Implementers may use SQLite, RocksDB, PostgreSQL, or any system satisfying these requirements.

### 7.2 Merkle Sync Tree

A Level 2 relay MUST maintain a Merkle sync tree per datastore, as defined in SPEC.md §2.6. The Merkle tree is a derived structure computed from the oplog.

The relay MUST update its Merkle tree as new operations are persisted. The relay MUST be able to serve `SYNC_REQUEST`, `SYNC_RESPONSE`, `DELTA_REQUEST`, and `DELTA_BATCH` messages as part of the standard sync protocol.

### 7.3 Snapshot Sync

A Level 2 relay SHOULD support snapshot sync (SPEC.md §4.2) for new peers joining a datastore with no history. The relay serves a compressed state snapshot plus the recent oplog tail, enabling new peers to bootstrap without replaying the entire oplog.

### 7.4 Compaction

A Level 2 relay MAY compact its oplog per the rules in SPEC.md §7.3 (causal stability pruning, tombstone GC, snapshot checkpointing, CRDT metadata pruning).

**Minimum retention:** The relay MUST retain at least the full oplog for a configurable retention window (RECOMMENDED default: 30 days). Operations older than the retention window that have been acknowledged by all currently known peers MAY be compacted.

---

## 8. Rate Limiting & Backpressure

### 8.1 Protocol-Level Limits

The relay announces its limits in the `WELCOME` message. Recommended defaults:

| Limit | Default | Notes |
|-------|---------|-------|
| Max operation payload | 1 MB | Per-operation; configurable by relay operator |
| Max batch size | 64 operations | Per `LIVE_OP_BATCH` message |
| Max batch bytes | 16 MB | Per batch; whichever limit is hit first applies |
| Max subscriptions | 64 | Per-peer concurrent datastore subscriptions |

### 8.2 Per-Peer Rate Limiting

The relay MAY enforce per-`PeerId` rate limits:

- **Operations per second** (RECOMMENDED default: 100 ops/s)
- **Bytes per second** (RECOMMENDED default: 10 MB/s)

When a peer exceeds its rate limit:

1. The relay MUST send a `RATE_LIMIT` message indicating the violated limit and a `retry_after_ms` duration
2. The relay MUST NOT silently drop operations — it MUST either accept, reject with `ERROR`, or signal with `RATE_LIMIT`
3. If the peer continues to exceed limits after receiving `RATE_LIMIT`, the relay MAY disconnect with `GOODBYE` (reason code `3`: rate limit exceeded)

### 8.3 System-Wide Backpressure

When the relay is under global load pressure (high queue depth, memory pressure, I/O saturation):

1. The relay sends `BACKPRESSURE` to connected peers with a suggested delay
2. Peers SHOULD respect backpressure by reducing their send rate
3. If peers ignore backpressure, the relay MAY disconnect them with `GOODBYE`

### 8.4 Abuse Mitigation

Relay operators SHOULD implement:

- **Per-PeerId connection limits:** Maximum concurrent connections from a single `PeerId` (RECOMMENDED: 3)
- **Proof-of-work for connection:** Optional computational puzzle in the `CHALLENGE` message to mitigate Sybil attacks (see SPEC.md §9.1)
- **IP-based rate limiting:** Transport-level defense, outside the scope of this protocol but RECOMMENDED for production deployments

---

## 9. Operation Validation

The relay MUST validate operations before forwarding or persisting them. Validation ensures relay integrity without requiring application-level schema knowledge.

### 9.1 Required Checks

All Level 1 and Level 2 relays MUST perform these checks on every received operation:

1. **Signature verification:** `Ed25519.verify(operation_content, operation.signature, sender_public_key)` — the operation's signature MUST be valid. If the operation is unsigned and the relay is configured to require signatures (RECOMMENDED default), reject with `ERROR` (code `0x301`).

2. **Content hash integrity:** `BLAKE3(operation_content) == operation.id` — the `OpId` MUST match the content hash. This detects corruption and tampering.

3. **Author consistency:** The operation's `peer` field MUST correspond to the public key that produced the signature. This prevents peers from claiming operations authored by others.

4. **Timestamp bounds:** The operation's HLC `physical_time` SHOULD be within `max_clock_drift` (configurable, RECOMMENDED default: 60 seconds) of the relay's own clock. Operations with timestamps far in the future are suspect (see SPEC.md §2.4, §9.1). The relay SHOULD reject such operations with `ERROR` (code `0x302`) but MAY accept them with a warning logged.

### 9.2 Checks the Relay MUST NOT Perform

- **ACL evaluation:** The relay does not have schema context and MUST NOT evaluate application-level access control rules (see SPEC.md §9.2). ACLs are enforced by peers.
- **CRDT type validation:** The relay does not know the schema and MUST NOT validate operation payloads against CRDT type expectations.
- **Referential integrity:** The relay MUST NOT check whether referenced nodes or edges exist.

This boundary is fundamental to the untrusted relay model: the relay ensures operations are **authentic** (signed by who they claim to be) but cannot judge whether they are **authorized** (permitted by application rules).

---

## 10. Error Handling

### 10.1 Error Code Space

| Range | Category | Examples |
|-------|----------|----------|
| `0x100–0x1FF` | Protocol errors | Bad framing, unknown message type, version mismatch, malformed CBOR |
| `0x200–0x2FF` | Authentication errors | Bad signature, unknown peer, expired session, challenge failed |
| `0x300–0x3FF` | Validation / resource errors | Rate exceeded, payload too large, too many subscriptions, invalid OpId |
| `0x400–0x4FF` | Sync errors | Unknown datastore, Merkle mismatch, missing dependencies, unsupported at this level |
| `0x500–0x5FF` | Internal relay errors | Storage failure, out of memory, internal timeout |

### 10.2 Specific Error Codes

| Code | Name | Fatal | Description |
|------|------|-------|-------------|
| `0x100` | `PROTOCOL_ERROR` | Yes | Unrecoverable protocol violation |
| `0x101` | `UNKNOWN_MESSAGE` | No | Unknown message type received |
| `0x102` | `VERSION_MISMATCH` | Yes | Incompatible protocol version |
| `0x103` | `MALFORMED_MESSAGE` | No | Message failed to decode |
| `0x201` | `AUTH_FAILED` | Yes | Authentication challenge failed |
| `0x202` | `SESSION_EXPIRED` | No | Session token no longer valid |
| `0x301` | `UNSIGNED_OP` | No | Operation lacks required signature |
| `0x302` | `CLOCK_DRIFT` | No | Operation timestamp too far in future |
| `0x303` | `PAYLOAD_TOO_LARGE` | No | Operation exceeds max payload size |
| `0x304` | `RATE_EXCEEDED` | No | Rate limit exceeded (see also `RATE_LIMIT` message) |
| `0x305` | `TOO_MANY_SUBS` | No | Max subscriptions reached |
| `0x306` | `INVALID_OPID` | No | OpId does not match content hash |
| `0x401` | `UNSUPPORTED_LEVEL` | No | Message requires a higher conformance level |
| `0x402` | `UNKNOWN_DATASTORE` | No | Datastore ID not recognized |
| `0x500` | `INTERNAL_ERROR` | No | Unspecified internal relay error |
| `0x501` | `STORAGE_ERROR` | No | Relay storage backend failure |

### 10.3 Recovery Behavior

- **Fatal errors** (`fatal: true`): the sender will close the connection. The receiver SHOULD NOT retry on the same connection.
- **Non-fatal errors:** the sender remains connected. The receiver MAY adjust behavior and retry the failed operation.

---

## 11. Relay Discovery

### 11.1 Static Configuration

The most common discovery method. Peers are configured with one or more relay URLs:

```typescript
const db = await ZeroDB.open({
  relay: 'wss://relay.example.com/v1/relay',
  // or multiple:
  relays: ['wss://relay1.example.com/v1/relay', 'wss://relay2.example.com/v1/relay'],
});
```

### 11.2 DNS-Based Discovery

Organizations running their own relays MAY publish DNS SRV records:

```
_zerodb-relay._tcp.example.com. 86400 IN SRV 10 0 443 relay.example.com.
```

Peers supporting DNS discovery SHOULD look up `_zerodb-relay._tcp.<domain>` and connect to advertised relays.

### 11.3 Well-Known URL

A domain MAY serve relay metadata at a well-known path:

```
GET https://example.com/.well-known/zerodb-relay
```

Response:

```json
{
  "relays": [
    {
      "url": "wss://relay.example.com/v1/relay",
      "level": 2,
      "pubkey": "<hex-encoded Ed25519 public key>"
    }
  ]
}
```

### 11.4 Relay Lists

A relay MAY include a list of other known relays in its `WELCOME` message (`known_relays` field). This enables peers to discover backup relays without out-of-band configuration.

---

## 12. Multi-Relay Federation

### 12.1 Peer-Bridged Replication

Peers MAY connect to multiple relays simultaneously for redundancy and availability. When a peer is connected to both Relay A and Relay B, it naturally bridges them: operations received from Relay A are synced to Relay B and vice versa.

This is the default federation model — no relay-to-relay coordination is required.

### 12.2 Relay-to-Relay Peering (Optional)

Relay operators MAY connect relays directly to each other. In this mode, relays behave as peers toward each other, using the standard sync protocol (§4.3) to exchange operations.

This is marked as **MAY** because:
- Peer bridging (§12.1) handles the common case
- Relay-to-relay peering adds operational complexity
- It is an optimization for large deployments where peer bridging introduces unacceptable latency

When relay-to-relay peering is used, both relays MUST be Level 2 (persistent).

---

## 13. Security Considerations

### 13.1 Relay Trust Model

Relays are **untrusted intermediaries**. This is a core design principle inherited from SPEC.md §4.4 and §9.1.

**A malicious relay CAN:**
- Delay or reorder operations
- Refuse connections from specific peers
- Observe unencrypted metadata: OpIds, PeerIds, HLC timestamps, datastore IDs, operation sizes, connection timing

**A malicious relay CANNOT:**
- Forge operations (Ed25519 signatures verify authorship)
- Undetectably drop operations (Merkle root comparison exposes omissions)
- Read E2E encrypted operation payloads (relay sees only ciphertext)
- Impersonate a peer (challenge-response authentication)

### 13.2 Metadata Leakage

Even with E2E encryption on operation payloads, the relay necessarily observes:

- Which peers are connected and when
- Which datastores each peer subscribes to
- Operation frequency and timing patterns
- Operation sizes (though not content)

**Mitigations (peer-side, outside this spec's scope):**
- Peers MAY pad operations to uniform sizes
- Peers MAY inject dummy operations at random intervals
- Peers MAY route through Tor or similar anonymization layers

### 13.3 Relay Censorship Detection

Peers detect relay censorship by comparing Merkle roots from multiple sources:

1. If a peer is connected to multiple relays, it compares Merkle roots across relays
2. If a peer has direct P2P connections, it compares relay Merkle roots with peer Merkle roots
3. A relay whose Merkle root consistently diverges from other sources is suspect

Peers SHOULD connect to at least two relays or one relay plus direct peer connections for censorship resistance.

### 13.4 Denial of Service

The relay is a natural target for denial-of-service attacks. Defenses:

- Protocol-level rate limiting (§8)
- Proof-of-work for connection establishment (§8.4)
- Transport-level defenses (firewalls, CDN, DDoS mitigation) — outside this spec's scope but RECOMMENDED for production

---

## 14. Monitoring & Observability

Guidance for relay operators. These are RECOMMENDED practices, not protocol requirements.

### 14.1 Metrics

Relay implementations SHOULD expose the following metrics:

| Metric | Type | Description |
|--------|------|-------------|
| `zerodb_relay_peers_connected` | Gauge | Currently connected peers |
| `zerodb_relay_datastores_active` | Gauge | Datastores with at least one subscriber |
| `zerodb_relay_ops_received_total` | Counter | Operations received from peers |
| `zerodb_relay_ops_forwarded_total` | Counter | Operations forwarded to peers |
| `zerodb_relay_ops_stored_total` | Counter | Operations persisted (L2) |
| `zerodb_relay_ops_rejected_total` | Counter | Operations rejected (by reason) |
| `zerodb_relay_sync_sessions_active` | Gauge | Active Merkle sync sessions (L2) |
| `zerodb_relay_auth_failures_total` | Counter | Failed authentication attempts |
| `zerodb_relay_message_latency_ms` | Histogram | Time from receive to forward |

### 14.2 Health Endpoint

The relay SHOULD expose an HTTP health endpoint **separate** from the protocol port:

```
GET /health
```

```json
{
  "status": "healthy",
  "level": 2,
  "uptime_seconds": 86400,
  "peers": 142,
  "version": "0.1.0"
}
```

### 14.3 Logging

- The relay SHOULD log connection events, authentication failures, rate limit triggers, and sync session lifecycle.
- The relay MUST NOT log operation payloads (privacy).
- The relay MAY log OpIds and PeerIds for debugging (these are public identifiers).

### 14.4 Telemetry

The relay MAY expose traces and metrics via OpenTelemetry (OTLP). This is RECOMMENDED for production deployments integrated into existing observability infrastructure.

---

## 15. Transport Bindings

### 15.1 WebSocket

The primary transport for browser-to-relay and general-purpose connections.

- **Path:** `/v1/relay`
- **Subprotocol:** `zerodb-relay-v1` (negotiated via `Sec-WebSocket-Protocol`)
- **Mode:** Binary frames. Each WebSocket message is one protocol message.
- **TLS:** `wss://` RECOMMENDED for production

### 15.2 TCP

For server-to-server and CLI connections.

- **Default ports:** `7433` (plaintext), `7434` (TLS)
- **Framing:** 4-byte big-endian length prefix + payload (see §3.1)
- **TLS:** RECOMMENDED for production

### 15.3 WebRTC DataChannel

For relay-to-browser connections where WebSocket is not available or for relay-facilitated P2P upgrade.

- **Channel label:** `zerodb-relay`
- **Ordered:** Yes
- **Reliable:** Yes
- Each DataChannel message is one protocol message

### 15.4 QUIC (Future)

QUIC is a natural fit for the relay protocol due to its multiplexed streams and built-in TLS. A future version of this specification MAY define a QUIC transport binding. This is noted as a forward-looking consideration and is not required for conformance.

---

## Appendix A: Message Type Registry

| Code | Name | Direction | Level |
|------|------|-----------|-------|
| `0x01` | `HELLO` | P→R | L0 |
| `0x02` | `CHALLENGE` | R→P | L0 |
| `0x03` | `AUTH` | P→R | L0 |
| `0x04` | `WELCOME` | R→P | L0 |
| `0x10` | `SUBSCRIBE` | P→R | L1 |
| `0x11` | `SUBSCRIBED` | R→P | L1 |
| `0x12` | `UNSUBSCRIBE` | P→R | L1 |
| `0x20` | `SYNC_REQUEST` | ↔ | L2 |
| `0x21` | `SYNC_RESPONSE` | ↔ | L2 |
| `0x22` | `DELTA_REQUEST` | ↔ | L2 |
| `0x23` | `DELTA_BATCH` | ↔ | L2 |
| `0x24` | `SYNC_ACK` | ↔ | L2 |
| `0x30` | `LIVE_OP` | P→R | L1 |
| `0x31` | `LIVE_OP_BATCH` | ↔ | L1 |
| `0x32` | `OP_ACK` | R→P | L1 |
| `0x33` | `RELAY_OP` | R→P | L1 |
| `0x40` | `PEER_ANNOUNCE` | P→R | L0 |
| `0x41` | `PEER_LIST_REQUEST` | P→R | L0 |
| `0x42` | `PEER_LIST_RESPONSE` | R→P | L0 |
| `0x43` | `SIGNAL_OFFER` | P→R→P | L0 |
| `0x44` | `SIGNAL_ANSWER` | P→R→P | L0 |
| `0x45` | `SIGNAL_ICE` | P→R→P | L0 |
| `0x50` | `PING` | ↔ | L0 |
| `0x51` | `PONG` | ↔ | L0 |
| `0x52` | `STATUS_REQUEST` | P→R | L0 |
| `0x53` | `STATUS_RESPONSE` | R→P | L0 |
| `0x54` | `RATE_LIMIT` | R→P | L1 |
| `0x55` | `BACKPRESSURE` | R→P | L1 |
| `0xFE` | `GOODBYE` | ↔ | L0 |
| `0xFF` | `ERROR` | ↔ | L0 |

Message type codes `0x60–0xEF` are reserved for future use. Codes `0xF0–0xFD` are reserved for implementation-specific extensions.

---

## Appendix B: Example Session Transcript

An annotated example of a complete session. Shown in JSON for readability (actual wire format is CBOR).

```
// 1. Peer connects via WebSocket to wss://relay.example.com/v1/relay

// 2. Peer sends HELLO
→ {
    "type": 1,
    "version": 1,
    "request_id": 1,
    "payload": {
      "peer_id": "a1b2c3d4e5f6...",
      "public_key": "<32 bytes, hex>",
      "protocol_version": 1,
      "features": ["json-debug"],
      "datastores": ["app:main"]
    }
  }

// 3. Relay sends CHALLENGE
← {
    "type": 2,
    "version": 1,
    "request_id": 1,
    "payload": {
      "nonce": "<32 random bytes, hex>"
    }
  }

// 4. Peer sends AUTH
→ {
    "type": 3,
    "version": 1,
    "request_id": 1,
    "payload": {
      "signature": "<Ed25519 signature over nonce, hex>"
    }
  }

// 5. Relay sends WELCOME (authentication successful)
← {
    "type": 4,
    "version": 1,
    "request_id": 1,
    "payload": {
      "session_id": "sess_abc123",
      "relay_level": 2,
      "relay_features": ["json-debug", "snapshot-sync"],
      "serialization": "json",
      "limits": {
        "max_payload_bytes": 1048576,
        "max_batch_size": 64,
        "max_subscriptions": 64,
        "ops_per_second": 100
      },
      "merkle_roots": {
        "app:main": "<merkle root hash, hex>"
      }
    }
  }

// 6. Peer initiates Merkle sync (roots differ)
→ {
    "type": 32,
    "version": 1,
    "request_id": 2,
    "payload": {
      "datastore": "app:main",
      "merkle_root": "<peer's merkle root, hex>"
    }
  }

// 7. Relay responds with its root
← {
    "type": 33,
    "version": 1,
    "request_id": 2,
    "payload": {
      "datastore": "app:main",
      "merkle_root": "<relay's merkle root, hex>",
      "match": false
    }
  }

// 8. Delta exchange happens (DELTA_REQUEST / DELTA_BATCH)...
// ... (omitted for brevity)

// 9. Sync confirmed
← {
    "type": 36,
    "version": 1,
    "request_id": 2,
    "payload": {
      "datastore": "app:main",
      "merkle_root": "<converged merkle root>"
    }
  }

// 10. Live mode: peer sends an operation
→ {
    "type": 48,
    "version": 1,
    "request_id": 0,
    "payload": {
      "datastore": "app:main",
      "operation": {
        "id": "<BLAKE3 hash>",
        "hlc": { "physical_time": 1742428800000, "logical_counter": 0, "peer_id": "a1b2c3d4e5f6..." },
        "peer": "a1b2c3d4e5f6...",
        "deps": ["<dep OpId>"],
        "entity": "<node UUIDv7>",
        "field": "name",
        "payload": { "type": "LWW", "value": "Alice" },
        "signature": "<Ed25519 signature>"
      }
    }
  }

// 11. Relay acknowledges
← {
    "type": 50,
    "version": 1,
    "request_id": 0,
    "payload": {
      "op_ids": ["<BLAKE3 hash>"],
      "merkle_root": "<updated merkle root>"
    }
  }

// 12. Relay forwards to other subscribers
// (sent to all other peers subscribed to "app:main")
← {
    "type": 51,
    "version": 1,
    "request_id": 0,
    "payload": {
      "datastore": "app:main",
      "operations": [{ ... same operation ... }]
    }
  }

// 13. Clean disconnect
→ {
    "type": 254,
    "version": 1,
    "request_id": 0,
    "payload": {
      "reason": 0,
      "message": "going offline"
    }
  }
```

---

## Appendix C: References

### SPEC.md Cross-References

| This Spec Section | SPEC.md Section | Topic |
|-------------------|----------------|-------|
| §3 Wire Format | §2.5 | Operation structure |
| §4.3 Sync Protocol | §4.1–4.2 | Sync lifecycle and modes |
| §4.4 Live Relay | §2.5, §2.8 | Operation structure, GroupId |
| §5 Authentication | §6.1–6.2 | Ed25519 identity, BLAKE3 PeerId |
| §6 Fan-Out | §2.5, §2.6, §2.8 | OpId dedup, Merkle tree, groups |
| §7 Persistence | §2.6, §7.3 | Merkle sync tree, compaction |
| §8 Rate Limiting | §9.1 | Sybil attack mitigation |
| §9 Validation | §6.1, §9.2 | Signatures, ACL boundary |
| §13 Security | §4.4, §9.1 | Trust model, threat model |

### External References

- [RFC 2119](https://www.rfc-editor.org/rfc/rfc2119) — Key words for use in RFCs
- [RFC 8949](https://www.rfc-editor.org/rfc/rfc8949) — CBOR (Concise Binary Object Representation)
- [Ed25519](https://ed25519.cr.yp.to/) — High-speed high-security signatures
- [BLAKE3](https://github.com/BLAKE3-team/BLAKE3) — Cryptographic hash function
