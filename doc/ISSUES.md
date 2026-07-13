# ZeroDB — Issues & Decisions

Single tracking list for specification issues and pending design decisions.
Resolved items are **removed**; durable outcomes land in the Decision Log at the bottom.

- **Critical (C):** blocks a correct/interoperable implementation or permits data loss or a security-boundary failure. All C-issues must have approved normative resolutions before any wire or persistent format freezes — this is the M0 exit gate ([SPEC §10](SPEC.md)).
- **High (H):** leaves a major guarantee, lifecycle, or roadmap outcome unreliable.
- **Open question (O):** design decision needing an owner and a milestone.

Consolidated 2026-07-13 from the external review (`FINDINGS.CODEX.md`, retired) after the M0–M6 roadmap was adopted into SPEC §10.

---

## Critical — M0 exit gate

### C1 — Canonical operation format
`Operation` (SPEC §2.5) lacks: variants for node/edge creation, labels, edge endpoints, migrations, capability grants, and key rotation; the datastore ID, schema epoch, entity kind, and author public key; a canonical byte encoding; and hash/signature preimage rules (including whether `id` and `signature` are excluded from their own preimages). The relay requires `BLAKE3(operation_content) == id` but `operation_content` is undefined — CBOR serializability alone does not select unique bytes.
**Resolution:** versioned operation algebra + normative wire schema: every variant, fixed identifier encodings/lengths, deterministic CBOR rules, duplicate-key rejection, domain-separated preimages, signed context, golden byte-level vectors; generate Rust/TypeScript types from the canonical schema. → **M0**

### C2 — Schema evolution breaks deterministic replay
CRDT type is resolved from the *current* mutable schema, but migrations can change types and arrive at peers in different orders; JavaScript resolver closures cannot replicate deterministically across languages; the initial schema has no replicated identity; historical replay after a type change is undefined. Violates the SEC guarantee (SPEC §2.7).
**Resolution:** one canonical schema representation with immutable IDs/versions; a causally ordered schema epoch bound to every data operation; CRDT variant carried in the operation or bound to an immutable schema version; deterministic migration DSL (no closures); mixed-version buffering/rejection and rollback rules. → **M0** (cross-peer migration ships M4)

### C3 — Merkle sync is not executable from the specified messages
Core says peers walk the remote tree root-down; the relay protocol exchanges only roots and then expects the requester to name `missing_hashes` it has no way to discover — no subtree/child/manifest request exists. The tree itself is non-canonical: bucket boundaries, leaf ordering, empty-node hashes, shape, internal-node encoding, late operations, and compaction representation are unspecified. Equal oplogs may hash unequal roots; unequal roots cannot be traversed.
**Resolution:** canonical authenticated tree + published root vectors; path/range subtree request/response (or paginated bucket manifests); `sync_id`, checkpoint/epoch, pagination/retry rules, concurrent-write semantics; a complete two-way transcript including mismatch recovery. → **M0** (wire protocol ships M3)

### C4 — Datastore boundary is neither authorized nor signed
Datastores are declared replication/RBAC boundaries, but the relay has no datastore admission credential — any authenticated key can subscribe to guessed datastores. The datastore ID sits outside the signed operation, so a valid op can be replayed into another datastore without breaking its signature; global OpId dedup can also suppress legitimate reuse across independent datastores.
**Resolution (direction decided 2026-07-13):** relay-verifiable **datastore-membership capabilities**; canonical `DatastoreId`, protocol version, and schema epoch inside the signed/hashed operation context; dedup scoped per datastore (already normative in RELAY-SPEC 0.2 §6.2). → **M0** (enforcement ships M3)

### C5 — Forwarded author signatures cannot be verified
Operations carry an author `PeerId` (a key *hash*) and signature but no resolvable public key. Relay validation verifies against the transport sender's key — which necessarily rejects legitimately forwarded history (bridging, relay-to-relay sync). Key rotation compounds the missing lookup contract.
**Resolution:** distinguish transport sender from author; carry or resolve an authenticated author key/certificate verified against `operation.peer`; specify rotation/revocation and historical-key lookup; define out-of-order arrival of key records vs. signed ops. → **M0** (resolution protocol ships M3)

### C6 — Entity-level ACLs conflict with SEC and read confidentiality
Write-ACL evaluation at receipt time depends on arrival order (grant/revoke/create races); quarantine-vs-materialize divergence between origin and receivers is permanent absent a deterministic reevaluation protocol; bootstrap authority for the first grant is undefined; read ACLs cannot protect plaintext already replicated to a hostile peer.
**Resolution (direction decided 2026-07-13):** v0.1 ships **datastore-level access control only** (membership + mandatory signatures); entity-level distributed ACLs are deferred until a causal authorization model exists — root authority, grant/revoke ordering, policy/schema versions, deterministic accept/reject/quarantine, reevaluation, and SEC scoped to a precisely defined accepted-operation set. Confidential reads via replication boundaries + cryptography, not local filters. → **design in M5/M6 window**

### C7 — Snapshot, compaction, and causal stability are unsafe as written
An HLC scalar acknowledged by "all known peers" is not a causal frontier; no durable per-peer acknowledgement, peer-membership lifecycle, retirement/lease, or reconnect rule exists ("known peers" undefined → collect too early or never). Level 2 relays are schema-blind yet expected to serve state snapshots; no snapshot request/chunk/format/signature/tail-boundary exists; compacted and uncompacted peers cannot compare Merkle roots without a shared checkpoint representation.
**Resolution:** causal frontiers independent of wall-clock; durable peer acks + retirement; authenticated checkpoint/snapshot identity and tail boundaries; anti-replay commitments; root comparison across checkpoints; decide whether L2 materializes or only stores peer-produced authenticated snapshots. GC stays **disabled** until partition/rejoin, forgotten-peer, late-op, and restore tests pass. → **contracts M0; implementation M5**

### C8 — Group and crash atomicity cannot be implemented from the contracts
A grouped op carries only `GroupId` — no member count, manifest, or commit marker, so no peer or relay can know a group is complete; the relay's five-second timeout has no defined post-timeout behavior. Storage traits expose `append_ops` and `put_materialized` with no transaction/recovery boundary spanning both.
**Resolution:** signed group manifest (or cardinality/index/member hashes) + abort/expiry semantics; atomic storage transaction or WAL/replay protocol with explicit crash points and idempotent recovery tests. → **M0 contracts; M1 local half; M3 sync half**

---

## High

### H1 — Future-clock poisoning detected but not mitigated
Logical-counter caps don't bound attacker-controlled `physical_time`; a far-future signed HLC wins LWW until real time catches up. Core warns-never-blocks while relays may reject — divergent behavior.  Define one peer-side acceptance/quarantine rule, max forward skew, recovery path, and semantics for locally-accepted-relay-rejected ops. → M3

### H2 — Offline unique indexes have no conflict semantics
Two offline peers can create the same "unique" value; mapping `unique: true` to SQLite/IDB uniqueness makes remote materialization fail platform-dependently.  Define advisory/conflict-reporting uniqueness, an ownership CRDT, or required coordination — plus query/resolution behavior. → M5

### H3 — Delete/resurrection semantics incomplete
Unclear which peer generates cascading edge tombstones, how it's authorized to delete others' edges, whether late dangling edges get tombstoned or only hidden, whether node resurrection revives them, and which CRDT governs `__tombstone`. Cascades generated from each peer's current view produce divergent op sets. Define a deterministic delete state machine (or derived visibility instead of generated cascades). → M1

### H4 — Delivery, dedup, and replay semantics incomplete
Dedup state is bounded/resettable (L1) or compactable (L2), so old signed counter/set ops can replay after it disappears. No explicit delivery contract; per-op outcomes within a batch undefined; multi-batch deltas lack sequence/resume cursors. Define at-least-once semantics, durable anti-replay commitment surviving compaction, request-ID lifetime, per-op outcomes, resumable cursors, retry/backoff. → M0/M3

### H5 — Handshake, resumption, and serialization underspecified
Narrowed by RELAY-SPEC 0.2 (CBOR-only, no session resumption, no in-protocol mutual auth, TLS required outside dev, domain-separated nonce signature). Remaining: the `AUTH` signature covers only the nonce, not the negotiated handshake transcript (version, limits, transport binding) — bind it to a full transcript. → M3

### H6 — Direct P2P sync has no protocol
Core promises WebRTC/`connectPeer`; relay spec excludes direct sync; no peer handshake, role negotiation, datastore admission, reconnect, or conformance profile exists. Define a shared peer-sync protocol reused by relay participation (M3), or keep P2P out of the SDK surface until M4.

### H7 — Version and upgrade policy missing
No owner or compatibility policy relating wire, operation, schema, snapshot, and on-disk format versions; protocol version appears in four places without selection/negotiation rules; document versions (`0.x-draft`) don't map to wire `protocol_version: 1`. Define per-format authority, a supported version window, and rolling upgrade/downgrade tests. → M0 policy; M4 tests

### H8 — Security claims stronger than mechanisms
Narrowed by RELAY-SPEC 0.2 (proof-of-work removed; independent-second-source censorship requirement now normative in RELAY-SPEC §12.3). Remaining: whole-operation encryption (vs. property-level) is an open choice for metadata privacy. → M3

### H9 — RELAY-SPEC wire inconsistencies
Message-set inconsistencies fixed in RELAY-SPEC 0.2 (single bidirectional `OPS`; `SIGNAL` forwarded form carries `sender`; `max_batch_bytes`/`bytes_per_second` in `WELCOME.limits`). Remaining: no normative identifier/hash encoding-and-length table; generate registry, schemas, transcript, and limits from one machine-readable protocol definition; validate golden + negative fixtures in two languages. → M0/M3

### H10 — Encrypted-property envelope and key lifecycle missing
X25519/XChaCha20-Poly1305 are named but there is no ciphertext envelope, nonce construction, AAD, key IDs, recipient/group-key distribution, recipient add/remove, post-compromise rotation, offline-peer revocation, or bootstrap ordering of keys vs. data. Required before any private-data claim (EXEMPLAR). → M3

### H11 — No durable relay acknowledgement
`OP_ACK` acknowledges receipt before L2 persistence; callers get no durable-commit signal for the backup/catch-up role relays are sold on. Define separate receipt vs. durable acks (or persistence-before-ack for L2), sender retention, duplicate re-ack, timeout/reconnect/retransmit behavior. → M3

---

## Open questions

- **O1 — Large operation payloads.** Size limits vs. chunking vs. external blob storage; must be decided before Richtext (100 MB documents). → decide by M4
- **O2 — Schema source of truth.** TypeScript SDK vs. `.zerodb` DSL: one canonical, the other generated. → decide M0, implement M2
- **O3 — Query language subset.** Minimal (MATCH/WHERE/RETURN/ORDER BY/LIMIT) vs. aggregation/paths; grammar, null/conflict semantics, parameterization, traversal limits. → decide M0
- **O4 — WASM size budget.** Target vs. Automerge ~250 KB / Loro ~200 KB gz; optional modules for RGA/Richtext. → M4
- ~**O5 — GunDB migration path.** ~~State-snapshot converter (no history import possible) or clean break.~~ → unscheduled / won't do
- **O6 — Operation/batch size limits & protocol-level rate limiting.** Interacts with H4/H9. → M0/M3
- **O7 — Causal `deps` scale.** Last-seen-op-per-peer grows with peer count and can reference compacted ops; needs a compact causal frontier + checkpoint translation. Interacts with C7. → M0/M5

---

## Decision Log

| Date | Decision |
|------|----------|
| 2026-07-13 | **Relay protocol 0.2:** pruned for simplicity — CBOR-only over WebSocket/DataChannel; 22 message types (`OPS`, `SIGNAL`, `THROTTLE` consolidations); removed JSON mode, session resumption, in-protocol mutual auth, proof-of-work, TCP/QUIC bindings, DNS/well-known discovery, and relay-side causal/group buffering. TLS required outside dev; per-datastore dedup; domain-separated auth signature. |
| 2026-07-13 | **Roadmap:** M0–M6 milestone plan adopted (SPEC §10), replacing Phase 1–5. M0 spec-stabilization precedes all implementation; no format freezes before its exit gate. |
| 2026-07-13 | **First product slice:** Rust core + SQLite + CLI (M1). Node/NAPI SDK follows as M2; browser/WASM in M4. |
| 2026-07-13 | **v0.1 trust model:** operation signatures mandatory for all synced ops; datastore-membership capabilities for admission; entity-level distributed ACLs deferred (C6). |
| 2026-07-13 | **Interop crypto suite fixed:** Ed25519 + BLAKE3 + X25519/XChaCha20-Poly1305. Pluggable providers change key custody only. |
| 2026-07-13 | **Flutter:** Dart binding over the shared stable C ABI exported by the Rust core (same ABI as Swift/Kotlin); `dart:ffi` + ffigen; scheduled M6. UniFFI-only codegen avoided. |
| 2026-07 | **Clock:** HLC over Lamport/HAM (SPEC §2.4). Causality from the `deps` graph; the clock serves LWW tiebreak, time-bucketed Merkle sync, and time-range queries. |
| 2026-03 | **GC granularity:** per time-bucket, not per-entity (SPEC §7.3). |
| 2026-03 | **Schema enforcement:** schemaless-with-warnings default, opt-in strict mode (SPEC §3.4). |
| 2026-03 | **ACL extensibility:** application-level composition of built-in rule primitives; no engine-level custom rule execution (SPEC §9.2). |
| 2026-03 | **Partial replication:** separate datastores with independent Merkle trees + membership control; no filtered sync. |
| 2026-03 | **Sync mechanism:** unified Merkle sync; version vectors removed. |
| 2026-03 | **Relay protocol:** specified separately in RELAY-SPEC.md. |
