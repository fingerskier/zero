# ZeroDB Relay Protocol Specification

**Version:** 0.2.2-draft
**Date:** 2026-08-14
**Author:** Matt / Turing Automations
**Status:** Draft — handshake, dual-root, resume-cursor, reject-ack, and the experimental `merkle-walk-v1` traversal are implemented. Canonical CBOR wire is still protocol v3; no format freeze.
**Companion to:** [ZeroDB Technical Specification](SPEC.md), [MERKLE.md](MERKLE.md), [AUTH.md](AUTH.md), [DELIVERY.md](DELIVERY.md), [KERNEL.md](KERNEL.md)

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
- **Datastore:** An independent unit of replication with its own oplog and Merkle tree (see SPEC.md §4.4).

### 1.3 Non-Goals

This specification does **not** define:

- Peer-to-peer direct sync behavior (see SPEC.md §4, ISSUES H6)
- CRDT merge semantics (see SPEC.md §3)
- Storage engine internals for relay persistence
- Application-level access control evaluation (see SPEC.md §9.2)

### 1.4 Design Principle

The protocol is deliberately minimal: one serialization, two transports, one message per job. Features that can live peer-side (causal ordering, group atomicity, censorship detection) live peer-side. Anything a relay cannot implement correctly from peer-visible information is excluded rather than approximated.

---

## 2. Conformance Levels

Relay implementations declare one of three conformance levels. Each level is a strict superset of the previous.

### Level 0 — Signal Relay

Minimal implementation for peer discovery and WebRTC signaling.

**Capabilities:**
- Accept peer connections and authenticate identity
- Track datastore subscriptions (for presence/signaling scope only)
- Forward signaling messages between peers
- Respond to peer list queries
- Keepalive (PING/PONG)

**Does NOT:** store operations, participate in sync, or forward operations.

A Level 0 relay can be implemented in ~200 lines of code in any language with WebSocket support.

### Level 1 — Stateless Relay

Signal relay plus live operation forwarding.

**Capabilities (in addition to L0):**
- Forward operations to subscribed peers (fan-out)
- Validate operation signatures before forwarding
- Deduplicate operations by `(datastore, OpId)`
- Enforce limits and throttling

**Does NOT:** persist operations or participate in Merkle sync. If the relay restarts, no history is available.

### Level 2 — Persistent Relay

Full relay with oplog persistence and sync participation. This is the "always-on peer" described in SPEC.md §4.4.

**Capabilities (in addition to L1):**
- Persist all operations to durable storage
- Maintain a Merkle sync tree per datastore
- Participate in the sync protocol (Merkle sync, delta exchange)
- Compact oplog per SPEC.md §7.3 rules (subject to ISSUES C7 — GC disabled by default)

---

## 3. Wire Format

### 3.1 Serialization

The wire format is **CBOR** ([RFC 8949](https://www.rfc-editor.org/rfc/rfc8949)) — binary, compact, well-specified, broad cross-language support. There is no alternative or negotiated serialization; every message on every transport is CBOR. (Tooling that wants human-readable output decodes CBOR to JSON out-of-band.)

Canonical/deterministic encoding rules for *operation* bytes (hash and signature preimages) are defined by the operation format (ISSUES C1, SPEC §2.5), not by this document. Protocol envelope bytes are never hashed or signed and need not be canonical.

### 3.2 Framing

Both supported transports are message-oriented (see §14): each transport message carries exactly one protocol message. No additional framing is defined.

### 3.3 Message Envelope

Every protocol message shares a common envelope structure:

```
{
  type:       uint8       // Message type discriminator (see §4)
  request_id: uint32      // Request/response correlation ID (0 for unsolicited messages)
  payload:    map         // Type-specific payload fields
}
```

A response message carries the same `request_id` as the request that triggered it. Unsolicited messages (relay-initiated forwards, THROTTLE, ERROR not tied to a request) use `request_id: 0`. Peers MUST use a non-zero `request_id` on any message for which they expect a reply.

The protocol version is carried once, in `HELLO`/`WELCOME` (§4.1) — not per message.

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
  peer_id:          PeerId      // Claimed peer identity
  public_key:       bytes       // Ed25519 public key (32 bytes)
  protocol_version: uint8       // Requested protocol version (currently 1)
  capabilities:     [text]      // Offered session capabilities (§4.1.1)
}
```

#### `CHALLENGE` (0x02) — R→P [L0]

Relay sends a random nonce for the peer to sign, proving ownership of the claimed private key.

```
{
  nonce:  bytes     // 32 cryptographically random bytes, fresh per connection
}
```

#### `AUTH` (0x03) — P→R [L0]

Peer signs the negotiated handshake transcript with domain separation (draft AUTH preimage; not a format freeze):

```
{
  signature:  Signature   // Ed25519.sign(private_key, "zerodb-relay-auth-v2" || canonical_cbor(transcript))
}
```

`transcript` is a deterministic CBOR map:

```
{
  hello: {
    peer_id:            bytes
    public_key:         bytes
    protocol_version:   uint
    capabilities:       [text]   // as offered in HELLO
  }
  nonce:                bytes    // CHALLENGE nonce
  welcome: {
    protocol_version:   uint     // version the relay is about to send
    relay_level:        uint
    capabilities:       [text]   // intersection the relay is about to send
    limits:             { … }    // WELCOME.limits the relay is about to send
  }
}
```

AUTH is sent before WELCOME, so both sides reconstruct the WELCOME the relay will send (advertised limits + negotiated caps). A v1 nonce-only signature (`"zerodb-relay-auth-v1" || nonce`) MUST fail closed (`0x201`). A transcript whose limits or version bits differ from the relay's intended WELCOME MUST fail closed.

The relay MUST verify:
1. `Ed25519.verify("zerodb-relay-auth-v2" || canonical_cbor(transcript), signature, public_key)` for the key from `HELLO`
2. `BLAKE3(public_key) == peer_id` from `HELLO`

If verification fails, the relay MUST respond with `ERROR` (code `0x201`) and close the connection.

#### `WELCOME` (0x04) — R→P [L0]

Sent after successful authentication. Establishes the session.

```
{
  protocol_version: uint8       // Selected protocol version
  relay_level:      uint8       // Conformance level (0, 1, or 2)
  capabilities:     [text]      // Sorted intersection of peer offer and relay offer (§4.1.1)
  limits: {
    max_payload_bytes:  uint32  // Maximum operation payload size
    max_batch_ops:      uint16  // Maximum operations per OPS message
    max_batch_bytes:    uint32  // Maximum total bytes per OPS message
    max_subscriptions:  uint16  // Maximum concurrent datastore subscriptions
    ops_per_second:     uint32? // Per-peer rate limit (null = unlimited)
    bytes_per_second:   uint32? // Per-peer bandwidth limit (null = unlimited)
  }
}
```

#### 4.1.1 Session capabilities

Known capability tokens (registry `relay_capabilities`; unknown tokens are ignored):

| Token | Meaning |
|-------|---------|
| `dual-root` | L2 messages carry `validated_root` and `accepted_root` separately (§7.4) |
| `merkle-walk-v1` | Frozen-snapshot node/leaf traversal followed by an OpId delta (§4.3) |
| `resume-cursor` | `SYNC_REQUEST` / `SUBSCRIBE` may carry `Cursor = { frontier, epoch }` (DELIVERY §4) |
| `reject-ack` | `OP_ACK` lists per-op `ACCEPT` / `DUPLICATE` / `REJECT`; rejected OpIds are not retried |

`WELCOME.capabilities` MUST be the sorted intersection of the peer's offer and the relay's offer, restricted to the known set. A session without `dual-root` MUST NOT claim catch-up completeness.

There is no session resumption: a reconnecting peer repeats the full handshake (one signature — cheap by design).

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
  reason:   uint16      // 0 = normal, 1 = going offline, 2 = switching relay, 3 = limit violations
  message:  string?     // Optional human-readable message
}
```

After sending `GOODBYE`, the sender SHOULD close the transport connection. The receiver SHOULD NOT send further messages after receiving `GOODBYE`.

### 4.2 Datastore Subscription

Subscription is the single membership verb: it scopes presence, peer listing, signaling, and (L1+) operation forwarding.

#### `SUBSCRIBE` (0x10) — P→R [L0]

```
{
  datastores:   [string | {
    id: string,
    token: MembershipCapabilityToken // AUTH §3.3; required for protected stores
  }]
  connectable:  bool        // Whether this peer accepts direct P2P connections
  metadata:     map?        // Optional transport hints for signaling
}
```

For a datastore present in the relay membership control plane, the relay MUST
verify the AUTH §3.3 token, bind its device to the authenticated connection,
require `read` + `sync`, and reject guessed-id, forged, expired, or revoked
membership with `MEMBERSHIP_DENIED`. The relay MUST apply the same admission
state to OPS, SYNC, Merkle walk, and DELTA access; revocation closes access for
already-open sessions. Stores not yet provisioned remain experimental/open for
backward compatibility during M3b rollout.

#### `SUBSCRIBED` (0x11) — R→P [L0]

Confirms subscription. Sent once per `SUBSCRIBE` request.

```
{
  datastores: [{
    id:              string
    peer_count:      uint32
    validated_root:  MerkleRoot?   // L2 + dual-root: relay validated oplog
    accepted_root:   MerkleRoot?   // L2 + dual-root: omitted unless the relay itself accepted
  }]
}
```

A Level 2 relay that negotiated `dual-root` MUST include `validated_root`. It MUST NOT publish a single `merkle_root` that peers are expected to match.

#### `UNSUBSCRIBE` (0x12) — P→R [L0]

```
{
  datastores: [string]
}
```

### 4.3 Sync Protocol (L2)

When `merkle-walk-v1` is negotiated, the responder freezes its datastore op set at `SYNC_RESPONSE`. The requester walks only mismatched subtrees, compares OpIds at mismatched leaves, and asks for the missing full operations. Concurrent writes land after the frozen walk and are discovered by a later root comparison/session.

A Level 2 relay participates in sync as a peer — it has its own Merkle tree and oplog. Level 0 and Level 1 relays MUST reject these messages with `ERROR` (code `0x401`).

#### `SYNC_REQUEST` (0x20) — ↔ [L2]

```
{
  datastore:       string
  accepted_root:   MerkleRoot?     // required when the sender is a peer (P→R / peer→peer)
  validated_root:  MerkleRoot?     // required when the sender is a relay (R→P)
  cursor:          Cursor?         // { frontier: PeerId → {op_id, physical_ms, logical}, epoch } when resume-cursor is on
}
```

Required root is **direction-dependent** on both `SYNC_REQUEST` and `SYNC_RESPONSE`. A peer MUST publish `accepted_root` and MUST NOT invent a relay `validated_root`. A relay MUST publish `validated_root` and MAY omit `accepted_root` unless the relay itself accepted.

#### `SYNC_RESPONSE` (0x21) — ↔ [L2]

```
{
  datastore:       string
  validated_root:  MerkleRoot?     // required when the responder is the relay (R→P)
  accepted_root:   MerkleRoot?     // required when the responder is a peer (P→R)
  merkle_format_version: uint8?    // required with merkle-walk-v1
  bucket_width_ms: uint64?         // required with merkle-walk-v1
  bucket_indices: [uint64]?        // responder active leaves, sorted ascending
}
```

Required root is **direction-dependent**. A peer answering a relay-initiated `SYNC_REQUEST` MUST include `accepted_root` and MUST NOT be required to fabricate `validated_root` (the relay owns the validated oplog). A relay answering a peer-initiated `SYNC_REQUEST` MUST include `validated_root` and MAY omit `accepted_root` unless the relay itself accepted. Peers MUST NOT invent a relay validated root.

Equal `accepted_root` values between honest peers mean catch-up is complete. Equal `validated_root` vs `accepted_root` is **not** required. A late op covered by `cursor.frontier` MUST NOT be retransmitted (DELIVERY §4).

#### `DELTA_REQUEST` (0x22) — ↔ [L2]

```
{
  datastore:        string
  op_ids:           [OpId]      // Missing ids discovered from mismatched leaves
}
```

#### `DELTA_BATCH` (0x23) — ↔ [L2]

```
{
  datastore:    string
  operations:   [Operation]     // See SPEC.md §2.5
  remaining:    uint32          // Estimated remaining operations (0 = last batch)
}
```

The sender MUST respect the receiver's `max_batch_ops` and `max_batch_bytes` limits from `WELCOME`. If the delta exceeds a single batch, multiple `DELTA_BATCH` messages are sent with `remaining > 0` until the final batch (`remaining = 0`).

#### `SYNC_ACK` (0x24) — ↔ [L2]

Confirms convergence after delta exchange.

```
{
  datastore:    string
  merkle_root:  MerkleRoot      // Sender's updated Merkle root (should now match)
}
```

#### `MERKLE_NODE_REQUEST` (0x25) / `MERKLE_NODE_RESPONSE` (0x26) — ↔ [L2]

The requester names a canonical padded-tree `(level, index)`. The responder returns that node's hash and two child hashes from the frozen snapshot. Level 0 is a leaf and is requested with `MERKLE_LEAF_REQUEST` instead.

```text
request:  { datastore, level: uint32, index: uint32 }
response: { datastore, level, index, hash, left, right }
```

#### `MERKLE_LEAF_REQUEST` (0x27) / `MERKLE_LEAF_RESPONSE` (0x28) — ↔ [L2]

```text
request:  { datastore, leaf_index: uint32 }
response: { datastore, leaf_index, bucket_index: uint64?, op_ids: [OpId] }
```

The requester computes `remote op_ids − local op_ids` and sends those ids in `DELTA_REQUEST`. Implementations MUST bound a walk and fail or restart if the responder cannot serve the frozen snapshot.

### 4.4 Operations

One message type carries operations in both directions. Peer→relay it is a submission (correlated with `OP_ACK` via `request_id`); relay→peer it is a forward (`request_id: 0`). Direction is unambiguous from the transport, so no separate forward type exists.

#### `OPS` (0x30) — ↔ [L1]

```
{
  datastore:    string
  operations:   [Operation]     // 1..max_batch_ops, see SPEC.md §2.5
}
```

#### `OP_ACK` (0x31) — R→P [L1]

Relay acknowledges receipt of a peer's `OPS` submission, echoing its `request_id`.

```
{
  outcomes: [{
    op_id:    OpId
    outcome:  "ACCEPT" | "DUPLICATE" | "REJECT"
    reason:   text?            // required on REJECT (e.g. AUTHZ, SIG, DECODE)
  }]
  validated_root:  MerkleRoot? // L2 after persist
}
```

`REJECT` is not retryable. The sender MUST remove those OpIds from its retransmit set so a rejected op is not replayed forever (§7.4).

> **Note:** `OP_ACK` is a *receipt* acknowledgement. A durable-commit acknowledgement for L2 relays (persistence-before-ack or a separate durable ack) is pending ISSUES H11 → M3.

### 4.5 Peer Discovery & Signaling

#### `PEER_LIST_REQUEST` (0x40) — P→R [L0]

```
{
  datastore:    string
}
```

#### `PEER_LIST_RESPONSE` (0x41) — R→P [L0]

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

#### `SIGNAL` (0x42) — ↔ [L0]

Opaque signaling forwarding (WebRTC SDP offers/answers, ICE candidates — the relay neither inspects nor distinguishes them).

Peer→relay:

```
{
  target:   PeerId      // Intended recipient
  payload:  bytes       // Opaque signaling data
}
```

Relay→target (forwarded form — the relay replaces `target` with the authenticated sender):

```
{
  sender:   PeerId      // Authenticated originator, set by the relay
  payload:  bytes
}
```

The relay MUST: verify the sender is authenticated, look up the target among connected subscribers, forward with `sender` attached, and respond with `ERROR` (code `0x307`) if the target is not connected.

### 4.6 Control

#### `PING` (0x50) / `PONG` (0x51) — ↔ [L0]

Keepalive. The sender SHOULD send `PING` at a regular interval (RECOMMENDED: every 30 seconds). The receiver MUST respond with `PONG`. If no `PONG` is received within a timeout (RECOMMENDED: 60 seconds), the connection SHOULD be considered dead.

```
// PING
{ timestamp: uint64 }      // Sender's wall-clock timestamp (for latency estimation)

// PONG
{ timestamp: uint64 }      // Echo of the PING timestamp
```

#### `THROTTLE` (0x52) — R→P [L1]

Unified flow-control signal, covering both per-peer rate limiting and relay-wide backpressure.

```
{
  scope:            string      // "peer" (this peer exceeded its limits) | "relay" (relay under global load)
  retry_after_ms:   uint32      // Minimum delay before resuming sends
  reason:           string?     // Optional: "ops_per_second" | "bytes_per_second" | "queue_depth" | "memory" | "io"
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
  ├── HELLO ─────────────────────►│  (peer_id, public_key, protocol_version)
  │                               │
  │◄── CHALLENGE ─────────────────┤  (nonce)
  │                               │
  ├── AUTH ──────────────────────►│  (domain-separated signature over transcript)
  │                               │
  │◄── WELCOME ───────────────────┤  (protocol_version, relay_level, limits)
  │                               │
  │  [session established]        │
```

### 5.2 Identity Verification

The handshake proves the peer controls the Ed25519 private key corresponding to their claimed `PeerId`:

1. Peer sends `HELLO` with their `peer_id` and `public_key`.
2. Relay generates 32 cryptographically random bytes as a `nonce`, fresh per connection.
3. Peer signs `"zerodb-relay-auth-v2" || canonical_cbor(transcript)` with their Ed25519 private key. The transcript binds HELLO fields, the CHALLENGE nonce, and the WELCOME limits/version/caps the relay is about to send. A v1 nonce-only signature MUST fail closed.
4. Relay verifies the transcript signature against the public key from `HELLO`.
5. Relay verifies `BLAKE3(public_key) == peer_id`.

If either check fails, the relay MUST respond with `ERROR` (code `0x201`) and close the connection.

> Draft AUTH preimage (unfrozen). Direct P2P reuse of this helper is parked with H6 → M4.

### 5.3 Relay Identity

The relay is authenticated at the transport layer: peers verify the relay's TLS certificate (see §5.4). In-protocol mutual authentication (relay key pinning independent of the certificate chain) is deliberately excluded from this version.

### 5.4 Transport Security

Relay connections MUST use TLS (`wss://`) except for loopback and explicitly configured development environments. The `zerodb-relay` binary does not terminate TLS and does not mint certificates. It refuses a non-loopback plaintext listen unless `--allow-insecure` is passed (loopback `127.0.0.1` / `localhost` / `::1` may listen plaintext without the flag).

**Important:** TLS does NOT replace ZeroDB's E2E encryption of operation content (SPEC.md §6.2; whether the encryption unit is individual properties or whole operations is an open choice — ISSUES H8/H10). TLS protects the transport; E2E encryption protects operation content from the relay itself.

---

## 6. Fan-Out & Message Routing

### 6.1 Routing Model

The relay maintains a **subscription table**: a mapping from `datastore_id` to the set of connected `PeerId`s subscribed to that datastore.

When the relay receives an `OPS` submission from a peer:

1. Validate the operation(s) per §9
2. Acknowledge receipt with `OP_ACK`
3. Persist the operation(s) if L2
4. Forward via `OPS` (`request_id: 0`) to all other peers subscribed to the same datastore

The relay MUST NOT forward operations back to the peer that sent them.

### 6.2 Deduplication

The relay MUST deduplicate operations by `(datastore, OpId)` — dedup is scoped per datastore (ISSUES C4), so a legitimately re-signed operation in an independent datastore is not suppressed. If an operation has already been received for a datastore (from any peer), it MUST NOT be forwarded again.

Implementation: L1 relays maintain a bounded set of recently seen `(datastore, OpId)` pairs (e.g., last 100,000). For L2 relays, the oplog serves as the deduplication index. (Replay after dedup-state loss is tracked in ISSUES H4.)

### 6.3 Ordering

The relay forwards operations in arrival order and makes **no causal-ordering guarantee**. Peers already handle out-of-order delivery (SPEC.md §4) — relay-side dependency buffering would duplicate that logic with worse information, so it is deliberately excluded.

### 6.4 Groups

Operations sharing a `GroupId` that arrive in one `OPS` message MUST be forwarded in one `OPS` message. The relay performs **no group-completeness buffering**: it cannot know a group's membership (ISSUES C8), so group atomicity is a peer-side concern. If group operations arrive across multiple messages, they are forwarded across multiple messages.

### 6.5 Fan-Out Batching

The relay MAY coalesce operations from multiple submissions into a single forwarded `OPS` per receiver, provided per-datastore arrival order is preserved and the batch respects the receiver's `max_batch_ops` / `max_batch_bytes`.

---

## 7. Persistence Requirements (Level 2)

### 7.1 Oplog Storage

A Level 2 relay MUST persist all validated operations to durable storage, keyed by `(datastore, OpId)`. The storage MUST support:

- **Append:** Store new operations
- **Lookup by OpId:** Retrieve a specific operation by its content hash
- **Range query by HLC:** Retrieve operations within an HLC timestamp range (for time-bucket Merkle tree construction and delta serving)
- **Iteration by datastore:** Enumerate all operations belonging to a datastore

The specification does NOT mandate a storage engine. SQLite, RocksDB, PostgreSQL, or any system satisfying these requirements is acceptable.

### 7.2 Merkle Sync Tree

A Level 2 relay MUST maintain a Merkle sync tree per datastore as defined in [MERKLE.md](MERKLE.md) (M0c closed 2026-07-18). The tree is a derived structure. **Which op set it hashes is defined in §7.4** — hashing the raw validated oplog is not the same as hashing a peer's accepted set.

### 7.3 Compaction

Compaction follows SPEC.md §7.3 and is gated on ISSUES C7: garbage collection is **disabled by default** until causal-frontier, peer-retirement, and restore semantics are specified and tested. Independent of GC, the relay MUST retain the full oplog: the configurable retention window (RECOMMENDED: 30 days) is a **minimum service commitment**, not a deletion license — deleting operations after the window is still forbidden until C7 GC semantics ship (M5).

Snapshot sync for bootstrapping new peers has no messages in this protocol version; snapshot identity is M0f and shipping is M4 (SPEC §10).

### 7.4 Accepted sets vs Merkle roots (CX-08)

Three sets exist around an L2 relay. They are **not** interchangeable.

| Set | Who computes it | Filter |
|-----|-----------------|--------|
| **Validated** | Relay | Decode, signature, OpId, ds match, size limits. **Not** the AUTH §4 causal authz predicate. |
| **Accepted** | Honest peer | Validated **plus** AUTH §4 (membership at grant-time, revoke, founder). MERKLE.md hashes **this** set. |
| **Rejected** | Honest peer | Validated ops the peer will not materialize (unauthorized, equivocation, wrong ds after forwarding). |

A colluding or merely schema-blind relay can retain authentic but unauthorized ops. Those ops stay in the relay validated oplog and would change a Merkle root built over that oplog. Honest peers reject them. **Equal Merkle roots between relay and peer are therefore not a protocol invariant** if both trees hash different sets.

**v0.1 contract (Decision Log 2026-08-14):** L2 publishes **two** roots when it claims catch-up:

1. `validated_root` — MERKLE over the relay's validated oplog (what it stored).
2. Peers publish `accepted_root` — MERKLE over their accepted set.

Catch-up completeness (EXEMPLAR E3) is: after sync, every honest peer's `accepted_root` matches every other honest peer's `accepted_root`. The relay's `validated_root` MAY differ. The protocol MUST acknowledge rejected OpIds (explicit `REJECT` outcomes per DELIVERY) so a sender does not retry forever.

Wire frames (M3a transcripts, `conformance/vectors/required/relay/`): ordered `{type, request_id, payload}` envelopes for `HELLO`/`CHALLENGE`/`AUTH`/`WELCOME`/`ERROR`, `SYNC_*` (direction-dependent roots + `Cursor`), and `OPS`/`OP_ACK.outcomes`. Both conformance runners walk type codes, directions, required fields, and `request_id` correlation. Dual-root **Merkle walk** messages (subtree traversal carrying both roots) remain M3a implementation, not this contract slice.

Do **not** implement a relay that claims peer-root equality against its validated oplog.

---

## 8. Limits & Throttling

### 8.1 Protocol-Level Limits

The relay announces its limits in the `WELCOME` message. Recommended defaults:

| Limit | Default | Notes |
|-------|---------|-------|
| `max_payload_bytes` | 1 MB | Per-operation; configurable by relay operator |
| `max_batch_ops` | 64 | Per `OPS` message |
| `max_batch_bytes` | 16 MB | Per `OPS` message; whichever limit is hit first applies |
| `max_subscriptions` | 64 | Per-peer concurrent datastore subscriptions |
| `ops_per_second` | 100 | Per-peer |
| `bytes_per_second` | 10 MB/s | Per-peer |

### 8.2 Enforcement

When a peer exceeds its limits, or the relay is under global load (queue depth, memory, I/O):

1. The relay MUST NOT silently drop operations — it MUST accept, reject with `ERROR`, or signal with `THROTTLE`. Rejected OPS MUST persist zero durable writes.
2. `max_payload_bytes` / `max_batch_*` are enforced on every `OPS` (and as a pre-decode frame ceiling on `handle`, including before AUTH).
3. `max_subscriptions` is enforced per session. A `SUBSCRIBE` that would exceed the cap is `ERROR` `0x305` `TOO_MANY_SUBS`. Re-subscribing an existing datastore does not increment the count.
4. `ops_per_second` and `bytes_per_second` are enforced per session on admitted OPS count/bytes (sliding 1s window). Exceeding either is `ERROR` `0x304` `RATE_EXCEEDED`.
5. Rate, subscription, and connection caps MUST NOT wait for a membership grant.
6. `THROTTLE` with `scope: "peer"` targets the offending peer; `scope: "relay"` asks all peers to slow down.
7. Peers SHOULD respect `retry_after_ms`. If a peer persistently ignores throttling, the relay MAY disconnect it with `GOODBYE` (reason `3`).

### 8.3 Abuse Mitigation

Relay operators SHOULD limit concurrent connections per `PeerId` (RECOMMENDED: 3). This implementation enforces 3: a fourth AUTH from the same `PeerId` is `ERROR` `0x304` `TOO_MANY_CONNECTIONS` (fatal) and the session is closed. IP-based rate limiting and DDoS mitigation are transport-level defenses outside this protocol, RECOMMENDED for production deployments.

---

## 9. Operation Validation

The relay MUST validate operations before forwarding or persisting them. Validation ensures relay integrity without requiring application-level schema knowledge.

### 9.1 Required Checks

All Level 1 and Level 2 relays MUST perform these checks on every received operation:

1. **Signature presence and verification:** operation signatures are mandatory for all synced operations (v0.1 trust model). Unsigned operations are rejected with `ERROR` (code `0x301`). Author-key resolution is AUTH §1 (M0d); on-wire enforcement is M3b. Relays MUST NOT reject forwarded operations solely because the author key is not the transport sender's key.

2. **Content hash integrity:** `OpId` MUST equal `BLAKE3(id-preimage)` per KERNEL §4.4 (M0a). This detects corruption and tampering.

3. **Author consistency:** The operation's `peer` field MUST correspond to the public key that produced the signature.

4. **Timestamp bounds:** The load-bearing H1 rule is **peer-side** (KERNEL §5 / AUTH.md §6): `CLOCK_DRIFT` quarantine, `max_drift_ms` default 60 seconds, release when `ts.p ≤ wall + max_drift_ms`. A relay SHOULD persist and forward well-formed member operations even when `physical_time` is far ahead of the relay clock, so two honest peers cannot be left with different LWW winners because one relay dropped the op. The relay MAY still refuse with `ERROR` (code `0x302` `CLOCK_DRIFT`) as a bandwidth filter; that refusal is not integrity. A locally-accepted (author's own far-future wall) op that a refusing relay dropped is recovered by resubmit after the window, at which point peers apply or quarantine under the same rule.

### 9.2 Checks the Relay MUST NOT Perform

- **ACL evaluation:** The relay does not have schema context and MUST NOT evaluate application-level access control rules (see SPEC.md §9.2). ACLs are enforced by peers.
- **CRDT type validation:** The relay does not know the schema and MUST NOT validate operation payloads against CRDT type expectations.
- **Referential integrity:** The relay MUST NOT check whether referenced nodes or edges exist.

This boundary is fundamental to the untrusted relay model: the relay ensures operations are **authentic** (signed by who they claim to be) but cannot judge whether they are **authorized** (permitted by application rules).

---

## 10. Error Handling

### 10.1 Error Code Space

| Range | Category |
|-------|----------|
| `0x100–0x1FF` | Protocol errors |
| `0x200–0x2FF` | Authentication errors |
| `0x300–0x3FF` | Validation / resource errors |
| `0x400–0x4FF` | Sync errors |
| `0x500–0x5FF` | Internal relay errors |

### 10.2 Specific Error Codes

| Code | Name | Fatal | Description |
|------|------|-------|-------------|
| `0x100` | `PROTOCOL_ERROR` | Yes | Unrecoverable protocol violation |
| `0x101` | `UNKNOWN_MESSAGE` | No | Unknown message type received |
| `0x102` | `VERSION_MISMATCH` | Yes | Incompatible protocol version |
| `0x103` | `MALFORMED_MESSAGE` | No | Message failed to decode |
| `0x201` | `AUTH_FAILED` | Yes | Authentication challenge failed |
| `0x202` | `MEMBERSHIP_DENIED` | No | Missing, invalid, expired, or revoked datastore membership |
| `0x301` | `UNSIGNED_OP` | No | Operation lacks required signature |
| `0x302` | `CLOCK_DRIFT` | No | Operation timestamp too far in future |
| `0x303` | `PAYLOAD_TOO_LARGE` | No | Operation or batch exceeds limits |
| `0x304` | `RATE_EXCEEDED` | No | Rate limit exceeded (see also `THROTTLE`) |
| `0x305` | `TOO_MANY_SUBS` | No | Max subscriptions reached |
| `0x306` | `INVALID_OPID` | No | OpId does not match content hash |
| `0x307` | `TARGET_NOT_CONNECTED` | No | Signaling target peer not connected |
| `0x401` | `UNSUPPORTED_LEVEL` | No | Message requires a higher conformance level |
| `0x402` | `UNKNOWN_DATASTORE` | No | Datastore ID not recognized |
| `0x500` | `INTERNAL_ERROR` | No | Unspecified internal relay error |
| `0x501` | `STORAGE_ERROR` | No | Relay storage backend failure |

### 10.3 Recovery Behavior

- **Fatal errors** (`fatal: true`): the sender will close the connection. The receiver SHOULD NOT retry on the same connection.
- **Non-fatal errors:** the sender remains connected. The receiver MAY adjust behavior and retry the failed operation.

---

## 11. Discovery & Federation

### 11.1 Discovery

Peers are configured with one or more relay URLs:

```typescript
const db = await ZeroDB.open({
  relay: 'wss://relay.example.com/v1/relay',
  // or multiple:
  relays: ['wss://relay1.example.com/v1/relay', 'wss://relay2.example.com/v1/relay'],
});
```

Static configuration is the only discovery mechanism in this version. Automatic discovery (DNS SRV, well-known URLs, relay-advertised relay lists) is deliberately excluded — it adds attack surface and specification weight without being needed for any current milestone.

### 11.2 Federation

Peers MAY connect to multiple relays simultaneously for redundancy. A peer connected to Relay A and Relay B naturally bridges them: operations received from one are synced to the other. This is the only federation model; no relay-to-relay coordination protocol exists or is required. (Two Level 2 relays MAY nevertheless connect to each other as ordinary peers using this same protocol — nothing about it is peer-exclusive.)

---

## 12. Security Considerations

### 12.1 Relay Trust Model

Relays are **untrusted intermediaries**. This is a core design principle inherited from SPEC.md §4.4 and §9.1.

**A malicious relay CAN:**
- Delay or reorder operations
- Refuse connections from specific peers
- Observe unencrypted metadata: OpIds, PeerIds, HLC timestamps, datastore IDs, operation sizes, connection timing

**A malicious relay CANNOT:**
- Forge operations (Ed25519 signatures verify authorship)
- Undetectably drop operations **from a peer that compares Merkle roots with a second independent source** (§12.3) — a peer relying on this relay alone can be censored
- Read E2E-encrypted operation content (relay sees only ciphertext; encryption scope per ISSUES H8/H10)
- Impersonate a peer **to the relay's own auth layer** (challenge-response). Caveat: the forwarded `SIGNAL.sender` field is relay-asserted — signaling identity is not end-to-end authenticated until the signed peer handshake ships (ISSUES H6, M3)

### 12.2 Metadata Leakage

Even with E2E encryption on operation payloads, the relay necessarily observes which peers are connected and when, which datastores each peer subscribes to, and operation frequency, timing, and sizes. Peer-side mitigations (padding, cover traffic, anonymizing transports) are outside this spec's scope.

### 12.3 Censorship Detection

A single relay is a trivially effective censor for any peer that depends on it alone. Therefore, normatively: **peers SHOULD maintain at least two independent sync sources** — two relays, or one relay plus direct peer connections. Peers detect censorship by comparing Merkle roots across sources; a relay whose root consistently diverges is suspect.

### 12.4 Denial of Service

Defenses: protocol-level limits and throttling (§8), per-PeerId connection caps (§8.3), and transport-level protections (firewalls, DDoS mitigation) outside this spec's scope. Authentication requires only one signature verification per connection attempt, which bounds the relay's per-connection crypto cost; deployments needing stronger Sybil resistance should apply transport-level controls.

---

## 13. Operational Guidance

RECOMMENDED practices, not protocol requirements:

- Expose an HTTP health endpoint separate from the protocol port (`GET /health` returning status, level, uptime, peer count, version).
- Export operational metrics (connected peers, active datastores, ops received/forwarded/rejected, auth failures, forward latency) via the deployment's usual telemetry stack.
- Log connection events, authentication failures, throttle triggers, and sync session lifecycle. OpIds and PeerIds are public identifiers and MAY be logged.

One requirement: the relay MUST NOT log operation payloads (privacy).

---

## 14. Transport Bindings

Two transports, both message-oriented. Additional bindings (e.g., QUIC) may be defined in future versions.

### 14.1 WebSocket

The primary transport for all connections — browser, server, and CLI.

- **Path:** `/v1/relay`
- **Subprotocol:** `zerodb-relay-v1` (negotiated via `Sec-WebSocket-Protocol`)
- **Mode:** Binary frames. Each WebSocket message is one protocol message.
- **TLS:** `wss://` REQUIRED except loopback/development (§5.4)

### 14.2 WebRTC DataChannel

For relay-facilitated P2P upgrade or environments without WebSocket.

- **Channel label:** `zerodb-relay`
- **Ordered:** Yes
- **Reliable:** Yes
- Each DataChannel message is one protocol message

---

## Appendix A: Message Type Registry

| Code | Name | Direction | Level |
|------|------|-----------|-------|
| `0x01` | `HELLO` | P→R | L0 |
| `0x02` | `CHALLENGE` | R→P | L0 |
| `0x03` | `AUTH` | P→R | L0 |
| `0x04` | `WELCOME` | R→P | L0 |
| `0x10` | `SUBSCRIBE` | P→R | L0 |
| `0x11` | `SUBSCRIBED` | R→P | L0 |
| `0x12` | `UNSUBSCRIBE` | P→R | L0 |
| `0x20` | `SYNC_REQUEST` | ↔ | L2 |
| `0x21` | `SYNC_RESPONSE` | ↔ | L2 |
| `0x22` | `DELTA_REQUEST` | ↔ | L2 |
| `0x23` | `DELTA_BATCH` | ↔ | L2 |
| `0x24` | `SYNC_ACK` | ↔ | L2 |
| `0x25` | `MERKLE_NODE_REQUEST` | ↔ | L2 |
| `0x26` | `MERKLE_NODE_RESPONSE` | ↔ | L2 |
| `0x27` | `MERKLE_LEAF_REQUEST` | ↔ | L2 |
| `0x28` | `MERKLE_LEAF_RESPONSE` | ↔ | L2 |
| `0x30` | `OPS` | ↔ | L1 |
| `0x31` | `OP_ACK` | R→P | L1 |
| `0x40` | `PEER_LIST_REQUEST` | P→R | L0 |
| `0x41` | `PEER_LIST_RESPONSE` | R→P | L0 |
| `0x42` | `SIGNAL` | ↔ | L0 |
| `0x50` | `PING` | ↔ | L0 |
| `0x51` | `PONG` | ↔ | L0 |
| `0x52` | `THROTTLE` | R→P | L1 |
| `0xFE` | `GOODBYE` | ↔ | L0 |
| `0xFF` | `ERROR` | ↔ | L0 |

Message type codes `0x60–0xEF` are reserved for future use. Codes `0xF0–0xFD` are reserved for implementation-specific extensions.

---

## Appendix B: Example Session Transcript

An annotated example of a complete session. Shown as JSON for readability; the wire format is CBOR.

```
// 1. Peer connects via WebSocket to wss://relay.example.com/v1/relay

// 2. Peer sends HELLO
→ { "type": 1, "request_id": 1, "payload": {
      "peer_id": "a1b2c3d4e5f6...",
      "public_key": "<32 bytes, hex>",
      "protocol_version": 1
  } }

// 3. Relay sends CHALLENGE
← { "type": 2, "request_id": 1, "payload": {
      "nonce": "<32 random bytes, hex>"
  } }

// 4. Peer sends AUTH
→ { "type": 3, "request_id": 1, "payload": {
      "signature": "<Ed25519 sig over 'zerodb-relay-auth-v2' || canonical_cbor(transcript), hex>"
  } }

// 5. Relay sends WELCOME
← { "type": 4, "request_id": 1, "payload": {
      "protocol_version": 1,
      "relay_level": 2,
      "limits": {
        "max_payload_bytes": 1048576,
        "max_batch_ops": 64,
        "max_batch_bytes": 16777216,
        "max_subscriptions": 64,
        "ops_per_second": 100,
        "bytes_per_second": 10485760
      }
  } }

// 6. Peer subscribes
→ { "type": 16, "request_id": 2, "payload": {
      "datastores": ["app:main"],
      "connectable": true
  } }

← { "type": 17, "request_id": 2, "payload": {
      "datastores": [{ "id": "app:main", "peer_count": 3,
                       "merkle_root": "<hash, hex>" }]
  } }

// 7. Merkle sync (roots differ → delta exchange, omitted; see §4.3 provisional note)
→ { "type": 32, "request_id": 3, "payload": {
      "datastore": "app:main", "merkle_root": "<peer's root, hex>"
  } }
← { "type": 33, "request_id": 3, "payload": {
      "datastore": "app:main", "merkle_root": "<relay's root, hex>"
  } }
// ... DELTA_REQUEST / DELTA_BATCH / SYNC_ACK ...

// 8. Live mode: peer submits an operation
→ { "type": 48, "request_id": 4, "payload": {
      "datastore": "app:main",
      "operations": [{
        "id": "<BLAKE3 hash>",
        "hlc": { "physical_time": 1783987200000, "logical_counter": 0, "peer_id": "a1b2c3d4e5f6..." },
        "peer": "a1b2c3d4e5f6...",
        "deps": ["<dep OpId>"],
        "entity": "<node UUIDv7>",
        "field": "name",
        "payload": { "type": "LWW", "value": "Alice" },
        "signature": "<Ed25519 signature>"
      }]
  } }

// 9. Relay acknowledges receipt
← { "type": 49, "request_id": 4, "payload": {
      "op_ids": ["<BLAKE3 hash>"],
      "merkle_root": "<updated root>"
  } }

// 10. Relay forwards to other subscribers (unsolicited → request_id 0)
← { "type": 48, "request_id": 0, "payload": {
      "datastore": "app:main",
      "operations": [{ ... same operation ... }]
  } }

// 11. Clean disconnect
→ { "type": 254, "request_id": 0, "payload": {
      "reason": 0, "message": "going offline"
  } }
```

---

## Appendix C: References

### SPEC.md Cross-References

| This Spec Section | SPEC.md Section | Topic |
|-------------------|----------------|-------|
| §3 Wire Format | §2.5 | Operation structure |
| §4.3 Sync Protocol | §4.1–4.2 | Sync lifecycle and modes |
| §4.4 Operations | §2.5, §2.8 | Operation structure, GroupId |
| §5 Authentication | §6.1–6.2 | Ed25519 identity, BLAKE3 PeerId |
| §6 Fan-Out | §2.5, §2.6, §2.8 | OpId dedup, Merkle tree, groups |
| §7 Persistence | §2.6, §7.3 | Merkle sync tree, compaction |
| §9 Validation | §6.1, §9.2 | Signatures, ACL boundary |
| §12 Security | §4.4, §9.1 | Trust model, threat model |

### External References

- [RFC 2119](https://www.rfc-editor.org/rfc/rfc2119) — Key words for use in RFCs
- [RFC 8949](https://www.rfc-editor.org/rfc/rfc8949) — CBOR (Concise Binary Object Representation)
- [Ed25519](https://ed25519.cr.yp.to/) — High-speed high-security signatures
- [BLAKE3](https://github.com/BLAKE3-team/BLAKE3) — Cryptographic hash function

---

## Appendix D: Changes from 0.1.0-draft

Pruned (each removable without loss for any current milestone):

- **JSON serialization mode + negotiation** — resolved the bootstrap paradox (encoding negotiated in messages that must already be decoded, ISSUES H5). CBOR only.
- **Session-token resumption** — bearer token skipping key proof (ISSUES H5). Reconnect = one signature.
- **In-protocol mutual auth** (`relay_id`/`relay_pubkey`/`relay_signature`) — underspecified; TLS authenticates the relay. Revisit with transcript binding (M3).
- **Proof-of-work challenge** — fields were never specified (ISSUES H8).
- **TCP and QUIC transports** — removed length-prefix framing, plaintext ports, and the plaintext-auth hazard. WebSocket + DataChannel only.
- **`LIVE_OP` / `LIVE_OP_BATCH` / `RELAY_OP`** → single bidirectional `OPS` (ISSUES H9 direction confusion).
- **`SIGNAL_OFFER` / `SIGNAL_ANSWER` / `SIGNAL_ICE`** → single `SIGNAL`; payloads were already opaque. Forwarded form now carries `sender` (H9 gap).
- **`RATE_LIMIT` + `BACKPRESSURE`** → single `THROTTLE` with `scope`.
- **`STATUS_REQUEST` / `STATUS_RESPONSE`** — duplicated the HTTP health endpoint.
- **`PEER_ANNOUNCE`** — folded into `SUBSCRIBE` (one membership verb); `SUBSCRIBE` moved to L0.
- **`HELLO.datastores` / `HELLO.features` / `WELCOME.merkle_roots` / `WELCOME.known_relays` / `WELCOME.session_id`** — handshake carries identity and limits, nothing else.
- **DNS SRV / well-known URL / relay-list discovery** — static configuration only.
- **Relay-side causal-ordering buffering** — peers must handle out-of-order delivery anyway.
- **Relay-side group-completeness buffering** — unimplementable without group manifests (ISSUES C8).
- **Snapshot-sync obligation** — no snapshot messages existed; contracts M0f, shipping M4 (ISSUES C7).
- **Per-message envelope `version` field** — version lives in `HELLO`/`WELCOME` only (ISSUES H7).
- **Metrics table / OTLP section** — condensed to operational guidance.

### 0.2.2-draft (2026-08-14)

- `HELLO`/`WELCOME.capabilities` — sorted intersection of `dual-root`, `resume-cursor`, `reject-ack`.
- Claimed `HELLO.peer_id` MUST equal `BLAKE3(public_key)` or AUTH fails with `0x201` (RELAY-HELLO-003).
- L2 publishes `validated_root` / `accepted_root` (not a single `merkle_root`).
- `SYNC_REQUEST` / `SYNC_RESPONSE` required root is direction-dependent: peer messages carry `accepted_root`; relay messages carry `validated_root`. Peers MUST NOT invent a relay validated root.
- `SYNC_REQUEST.cursor` is DELIVERY `{frontier, epoch}`.
- `OP_ACK.outcomes` with non-retryable `REJECT`.
- `relay-transcript` vectors carry ordered `{type, request_id, payload, cbor_hex}` frames. Binary fields (`peer_id`, `public_key`, `nonce`, `signature`, `validated_root`, `accepted_root`, `op_id`, `author`) encode as CBOR bytes. RELAY-HELLO-001/002/003, RELAY-ROOT-001, RELAY-RESUME-001, RELAY-REJECT-001.

### 0.2.1-draft

Added / fixed:

- `max_batch_bytes` and `bytes_per_second` in `WELCOME.limits` (ISSUES H9).
- Domain-separated handshake AUTH (`"zerodb-relay-auth-v2"` ‖ canonical CBOR transcript; v1 nonce-only fails closed). Draft preimage; not a format freeze.
- Dedup scoped per `(datastore, OpId)` (ISSUES C4 direction).
- TLS now REQUIRED outside loopback/development.
- Normative two-independent-sources censorship-resistance statement (ISSUES H8).
- `request_id` correlation for `OPS`/`OP_ACK`; `0x307 TARGET_NOT_CONNECTED`.
- Provisional/pending notes tying §4.3 to C3, §4.4 to H11, §9.1 to C1/C5/H1.
