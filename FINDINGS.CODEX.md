# ZeroDB SPEC, Plan, and Roadmap Review

- **Review date:** 2026-07-13
- **Scope:** `README.md`, `doc/SPEC.md`, `doc/RELAY-SPEC.md`, `doc/EXEMPLAR.md`, and `doc/ISSUES.md`
- **Review type:** Internal consistency, implementability, security/durability, interoperability, and roadmap readiness

## Executive verdict

ZeroDB has a strong architectural direction: a Rust source of truth, explicit causal history, per-field CRDT semantics, a property-graph model, an untrusted-relay goal, and an exemplar intended to exercise the hard cases. Separating the relay protocol into its own document and naming unresolved questions are also good foundations.

The current documents are **not implementation-ready**, however. A networked implementation built directly from them would require independent implementers to invent incompatible answers in the operation format, schema epochs, Merkle traversal, datastore authorization, author-key discovery, group completion, snapshot/compaction, and retry/durability semantics. Several of those gaps can cause silent data loss, unauthorized replication, or divergent materialized state while contradicting guarantees made by the specification.

There is also no standalone implementation plan. The only roadmap is the component checklist in `doc/SPEC.md:870-913`; it has no dependencies, owners, acceptance gates, issue links, or runnable vertical milestones. The recommended next step is therefore a **Milestone 0 specification-stabilization phase**, not Phase 1 implementation.

### Disposition

- **Do not claim interoperable relay conformance** from the current `0.1.0-draft` protocol.
- **Do not implement compaction, distributed ACLs, or cross-peer schema migration** until their causal contracts are defined and tested.
- A local, explicitly experimental Rust + SQLite prototype could proceed only if it is isolated from the unsettled wire/security contracts and does not freeze those formats accidentally.

### Severity

- **Critical:** blocks a correct/interoperable implementation or permits data loss/security-boundary failure.
- **High:** leaves a major guarantee, lifecycle, or roadmap outcome unreliable.
- **Medium:** causes ambiguity, inconsistent implementations, or planning/documentation drift but can be resolved without redesigning the core.

---

## 1. Critical specification blockers

### C1. `Operation` is not a complete, canonical protocol object

**Evidence:** `doc/SPEC.md:170-194`; `doc/SPEC.md:245-250`; `doc/SPEC.md:338-360`; `doc/SPEC.md:628-641`; `doc/RELAY-SPEC.md:103-124`; `doc/RELAY-SPEC.md:704-712`; `doc/RELAY-SPEC.md:1120-1129`.

The `Operation` interface contains an entity ID, field, CRDT payload, causal dependencies, optional group, and optional signature. The documents do not define how it represents or unambiguously resolves:

- node versus edge creation, node labels, edge labels, or edge endpoints;
- schema/migration, capability grant/revoke, or key-rotation operations mentioned elsewhere;
- the datastore, schema epoch, entity kind, author public key, or operation kind;
- the canonical byte representation used for hashing and signing;
- whether `id` and `signature` are excluded from their own hash/signature preimages.

The relay requires `BLAKE3(operation_content) == operation.id` and signature verification, but `operation_content` is undefined. Requiring CBOR serializability does not select a unique byte encoding. The relay transcript also stores `payload.type: "LWW"` while `doc/SPEC.md:194` explicitly says the CRDT type is not stored in an operation.

**Impact:** Independent cores and relays cannot agree on operation IDs, signatures, creation semantics, or replay. Even a single implementation has no stable format to persist before later phases.

**Required correction:** Define a versioned operation algebra and normative wire schema before implementation. Specify every variant, fixed identifier encodings and lengths, deterministic CBOR rules, duplicate-key rejection, domain-separated hash/signature preimages, signed context, and golden byte-level vectors. Generated Rust/TypeScript types should come from that canonical schema.

### C2. Schema evolution makes deterministic replay impossible as written

**Evidence:** `doc/SPEC.md:182-194`; `doc/SPEC.md:211-221`; `doc/SPEC.md:338-375`; `doc/SPEC.md:959-961`.

The CRDT type is resolved from the current `(entity.label, field)` schema rather than recorded in the operation. The same document permits that type to change through a migration and permits peers to receive migrations in different orders. It does not define the initial schema's replicated identity or which of TypeScript and `.zerodb` is authoritative.

The migration example includes a JavaScript resolver closure. Such a closure is not a declarative, cross-language payload that can be serialized into the oplog and evaluated identically by Rust, JavaScript, Swift, Kotlin, and third-party implementations. Historical replay after a type change is also undefined: pre- and post-migration payloads cannot safely be interpreted through one mutable current schema.

**Impact:** Two peers with the same operation set can materialize different state, contradicting the SEC guarantee at `doc/SPEC.md:221`. Restart/rebuild can also reinterpret old data differently from the original incremental execution.

**Required correction:** Choose one canonical schema representation. Give schemas immutable IDs/versions and define a causally ordered schema epoch for every data operation. Either include the CRDT operation variant in the operation or bind it cryptographically to an immutable schema version. Replace arbitrary resolver closures with a deterministic migration DSL or explicit, signed resolution operations. Define mixed-version buffering/rejection and rollback behavior.

### C3. Merkle synchronization cannot be executed from the specified messages

**Evidence:** `doc/SPEC.md:196-209`; `doc/SPEC.md:381-404`; `doc/SPEC.md:687-693`; `doc/RELAY-SPEC.md:278-337`; `doc/RELAY-SPEC.md:638-642`; `doc/RELAY-SPEC.md:974-1007`.

The core specification says peers walk the remote tree from the root downward. The relay protocol exchanges only roots, then asks the requester to send `missing_hashes`. There is no message that retrieves remote child hashes, nodes, paths, or bucket manifests. A requester therefore cannot discover the hashes it is missing. The example transcript omits the entire delta negotiation at `doc/RELAY-SPEC.md:1099-1100`.

The tree itself is also non-canonical. Bucket granularity is configurable, while bucket boundaries, operation ordering inside a leaf, empty-node hashes, tree shape/height, internal-node encoding, late operations, and compaction/checkpoint representation are unspecified. `merkle_subtree(depth)` does not identify which branch to retrieve.

**Impact:** Equal oplogs may produce unequal roots, and unequal roots cannot be traversed to compute a delta. Level 2 interoperability and the claimed sync-completeness proof have no executable subject.

**Required correction:** Specify a canonical authenticated data structure and publish root vectors. Add path/range subtree request/response messages (or a deterministic paginated bucket-manifest exchange), a `sync_id` and checkpoint/epoch, stable pagination/retry rules, concurrent-write semantics, and a complete two-way transcript including mismatch recovery.

### C4. The datastore security boundary is neither authorized nor signed

**Evidence:** `doc/SPEC.md:175-185`; `doc/SPEC.md:949-955`; `doc/RELAY-SPEC.md:136-147`; `doc/RELAY-SPEC.md:234-260`; `doc/RELAY-SPEC.md:341-349`; `doc/RELAY-SPEC.md:596-600`; `doc/RELAY-SPEC.md:716-722`; `doc/EXEMPLAR.md:8-17`.

`doc/SPEC.md:951` declares separate datastores to be replication and RBAC boundaries. The relay accepts datastore strings in `HELLO`, `SUBSCRIBE`, discovery, sync, and live-operation messages, but defines no datastore admission credential or membership check. Entity ACL evaluation is explicitly forbidden at the relay.

The datastore ID is outside the signed `Operation`. A valid signed operation can therefore be replayed into another datastore without invalidating its signature. Global deduplication by `OpId` can also suppress a legitimate use of the same operation content in a second independent datastore.

**Impact:** Any authenticated key can attempt to enumerate or subscribe to guessed datastores, and a malicious relay or forwarding peer can cross-inject authentic operations. This directly conflicts with the exemplar's private-data and access-control goals.

**Required correction:** Decide between (a) relay-verifiable datastore membership capabilities, or (b) public replication with mandatory whole-datastore encryption. In either model, include the canonical `DatastoreId`, protocol version, and relevant schema epoch in the signed and hashed operation context. Define whether deduplication is datastore-scoped.

### C5. Forwarded author signatures cannot be verified

**Evidence:** `doc/SPEC.md:175-185`; `doc/SPEC.md:626-641`; `doc/RELAY-SPEC.md:136-176`; `doc/RELAY-SPEC.md:704-714`; `doc/RELAY-SPEC.md:818-835`.

An operation carries an author `PeerId` and signature but not the author's public key or a resolvable certificate. The relay handshake supplies only the transport sender's public key. Relay validation nevertheless verifies with `sender_public_key` and requires it to match the operation author. Peer bridging and relay-to-relay sync necessarily submit operations authored by other peers.

**Impact:** Correctly validating against the transport sender rejects legitimate forwarded history. Trusting the claimed author cannot work because a hash-derived `PeerId` cannot be reversed into a public key. Key rotation makes the missing lookup contract more consequential.

**Required correction:** Distinguish transport sender from operation author. Carry or resolve an authenticated author key/certificate, verify its hash against `operation.peer`, specify rotation/revocation and historical-key lookup, and define bootstrap behavior when the key record and signed operation arrive out of order.

### C6. ACL enforcement conflicts with both SEC and read confidentiality

**Evidence:** `doc/SPEC.md:211-221`; `doc/SPEC.md:818-866`; `doc/SPEC.md:951`; `doc/RELAY-SPEC.md:716-722`.

Write ACLs are evaluated when an operation is received, against replicated state that may differ by arrival order. Grant, revoke, entity creation, and the protected mutation can race, but authorization is not defined at the operation's causal frontier. Bootstrap authority for the first `grant` capability and grant/revoke conflict semantics are missing.

Rejected operations are quarantined and not materialized, while the originating peer keeps and materializes them. The text says this avoids permanent divergence, but absent a deterministic reevaluation/rejection protocol it creates exactly that divergence for peers holding the same underlying operation history.

Read ACLs cannot protect plaintext already replicated to a malicious peer: that peer controls its storage and can inspect the oplog outside the SDK. Local read filtering is an application convenience, not a confidentiality boundary.

**Impact:** Authorization can depend on delivery order, honest peers can disagree permanently, and developers may rely on read protection that a hostile replica can bypass.

**Required correction:** Define a causal authorization model, including root authority, grant/revoke ordering, policy/schema versions, deterministic accept/reject/quarantine state, and reevaluation. Scope SEC to a precisely defined accepted operation set. Enforce confidential reads through replication boundaries and/or cryptographic key distribution and revocation, not local filters alone. A simpler explicit trust model for v0.1 may be safer than shipping underspecified distributed ACLs.

### C7. Snapshot, compaction, and causal stability do not form a safe protocol

**Evidence:** `doc/SPEC.md:407-413`; `doc/SPEC.md:720-729`; `doc/RELAY-SPEC.md:35-46`; `doc/RELAY-SPEC.md:81-90`; `doc/RELAY-SPEC.md:328-336`; `doc/RELAY-SPEC.md:625-652`; `doc/RELAY-SPEC.md:644-652`; `doc/RELAY-SPEC.md:716-720`; `doc/RELAY-SPEC.md:974-1007`.

The compaction rule treats an HLC bound acknowledged by all known peers as causal stability. An HLC scalar is not the specified per-author dependency frontier, and no durable per-peer acknowledgement frontier, peer-membership lifecycle, retirement/lease rule, or reconnect behavior exists. “Currently known peers” is undefined: an implementation could exclude offline peers and collect too early, or retain departed peers forever and never collect.

The relay is declared schema-blind and not a CRDT materializer, yet Level 2 is expected to serve state snapshots and perform the core's state/CRDT-metadata compaction. No snapshot request, chunk, format, signature, integrity root, compression negotiation, or exact oplog-tail boundary exists. Snapshot support is both an L2 capability and later only `SHOULD`. Peers with compacted and uncompacted histories also cannot compare ordinary Merkle roots over different operation sets without a shared checkpoint representation.

**Impact:** Tombstones or causal metadata can be removed while an offline peer can still reintroduce old state; compacted peers may never synchronize; a schema-blind relay cannot safely create the promised snapshot; Level 2 conformance is contradictory.

**Required correction:** Define causal frontiers independently of wall-clock order, durable peer acknowledgements, peer retirement, checkpoint/snapshot identity, snapshot authentication, tail boundaries, anti-replay commitments, and root comparison across checkpoints. Decide whether L2 materializes schemas or merely stores peer-produced authenticated snapshots. Keep GC disabled until partition/rejoin, forgotten-peer, late-operation, and restore tests pass.

### C8. Operation-group and local crash atomicity cannot be implemented from the contracts

**Evidence:** `doc/SPEC.md:175-185`; `doc/SPEC.md:223-243`; `doc/SPEC.md:616-620`; `doc/SPEC.md:663-707`; `doc/RELAY-SPEC.md:609-621`.

A grouped operation carries only `GroupId`: there is no member count, index, manifest, member-root, or commit marker. Neither relay nor peer can know when the full group has arrived. The relay requires a five-second timeout but does not define post-timeout behavior; forwarding an incomplete group remains possible even though the core promises sync atomicity.

The storage traits independently expose `append_ops` and `put_materialized`; no transaction/recovery boundary covers oplog append plus materialized state. That is insufficient to guarantee that a crash persists all grouped operations and materialization or none.

**Impact:** Incomplete groups can remain buffered forever or be applied partially, and crash recovery can violate the local atomicity guarantee.

**Required correction:** Add a signed group manifest or group cardinality/index/member hash and define abort/expiry semantics. Add an atomic storage transaction or a write-ahead/replay protocol with explicit crash points and idempotent recovery tests.

---

## 2. High-priority specification findings

### H1. Future-clock poisoning is detected but not mitigated

**Evidence:** `doc/SPEC.md:145-169`; `doc/SPEC.md:268`; `doc/RELAY-SPEC.md:704-715`.

The logical-counter cap does not bound an attacker-controlled `physical_time`. Because LWW selects the latest HLC, a far-future signed value can dominate legitimate writes until time catches up. The core says skewed peers are warned but never blocked, while relays should reject them but may merely warn.

Define one peer-side acceptance/quarantine rule, maximum forward movement, recovery path, and semantics for locally accepted operations rejected by relays. Qualify the claim that HLC alone prevents future-dated attacks.

### H2. Offline global uniqueness has no conflict semantics

**Evidence:** `doc/SPEC.md:731-764`.

Two disconnected peers can concurrently create the same supposedly unique value. Mapping `unique: true` directly to SQLite or IndexedDB uniqueness can cause a remote merge/materialization failure and platform-dependent behavior.

Define unique indexes as advisory/conflict-reporting, introduce a deterministic ownership CRDT, or require coordination for strict uniqueness. State query and resolution behavior for conflicts.

### H3. Referential-integrity and delete/resurrection semantics are incomplete

**Evidence:** `doc/SPEC.md:99-132`; `doc/SPEC.md:245-253`.

It is unclear which peer generates cascading tombstone operations, how that peer is authorized to delete edges authored by others, whether a late dangling edge is ever tombstoned or only hidden, whether node resurrection makes that edge live, and which CRDT governs `__tombstone`. Generating cascades from each peer's current materialized view can produce different operation sets.

Define a deterministic delete state machine, late-edge behavior, resurrection policy, cascade authority, and whether derived visibility rather than generated edge tombstones can provide the invariant.

### H4. Delivery, deduplication, and replay semantics are incomplete

**Evidence:** `doc/SPEC.md:188-192`; `doc/SPEC.md:807-816`; `doc/RELAY-SPEC.md:113-124`; `doc/RELAY-SPEC.md:314-326`; `doc/RELAY-SPEC.md:596-600`; `doc/RELAY-SPEC.md:648-652`; `doc/RELAY-SPEC.md:676-680`; `doc/RELAY-SPEC.md:1113-1142`.

L1 deduplication may be bounded and resets on restart; L2 uses an oplog index that compaction can remove. Old signed counter/set operations can therefore be accepted and forwarded after dedup state disappears even though replay is claimed to be prevented. The protocol also omits an explicit delivery contract.

`request_id: 0` is reserved for unsolicited messages, yet the transcript uses it for a peer-originated operation and its acknowledgement. Rate limiting does not say whether the triggering operation was accepted, queued, or rejected. Multi-batch deltas have only an estimated `remaining` count, without sequence or resume cursor.

State durable-at-least-once or at-least-once semantics, preserve an anti-replay commitment after payload compaction, and define request-ID lifetime, per-operation outcomes, resumable cursors, retry/backoff, duplicate acknowledgements, and reconnect behavior.

### H5. Authentication, resumption, and serialization negotiation are underspecified

**Evidence:** `doc/RELAY-SPEC.md:103-109`; `doc/RELAY-SPEC.md:136-196`; `doc/RELAY-SPEC.md:525-577`; `doc/RELAY-SPEC.md:942-957`.

The handshake negotiates CBOR versus JSON in `WELCOME`, but the relay must decode `HELLO`, `CHALLENGE`, and `AUTH` before the encoding is selected. Session resumption uses `session_id` as a bearer token that skips proof of key possession, while TLS is only recommended and plaintext TCP is an allowed binding. Token entropy, binding, rotation, replay, and revocation are unspecified.

Mutual authentication is described in prose but its request flag and `relay_signature` are absent from the normative message schemas. The signature covers only `nonce || peer_id`, not the negotiated transcript.

Require a fixed deterministic encoding through handshake, require authenticated encryption for authentication/resumption, bind resumption to a fresh key proof, and sign a domain-separated transcript including versions, identities, features, limits, and serialization.

### H6. Direct peer-to-peer sync has no protocol specification

**Evidence:** `doc/SPEC.md:379-434`; `doc/SPEC.md:573-576`; `doc/RELAY-SPEC.md:39-46`.

The core promises direct P2P operation over WebRTC and `connectPeer`, while the relay specification explicitly excludes direct-sync behavior. There is no peer-to-peer handshake, role negotiation, datastore admission, message binding, reconnect, or conformance profile.

Define a shared peer sync protocol reused by relay participation, or explicitly defer direct P2P and remove it from the early SDK/roadmap surface.

### H7. Versioning and upgrade behavior are missing or contradictory

**Evidence:** `doc/SPEC.md:663-674`; `doc/SPEC.md:949-973`; `doc/RELAY-SPEC.md:111-124`; `doc/RELAY-SPEC.md:136-196`; `doc/RELAY-SPEC.md:738-756`; `doc/RELAY-SPEC.md:942-949`.

The storage trait still exposes `ops_since(&VersionVector)` while the resolved decision says version vectors were removed. Protocol version appears in the envelope, `HELLO`, WebSocket path, and subprotocol, but selection/range negotiation and compatibility are absent. Snapshot, operation, schema, on-disk, and migration format versions are not owned or related.

Create an authority and compatibility policy for each format, support and test an explicit version window, define rolling upgrade/downgrade behavior, and remove stale interfaces/decisions.

### H8. Security and confidentiality claims are stronger than the mechanisms

**Evidence:** `doc/SPEC.md:424-434`; `doc/SPEC.md:626-641`; `doc/SPEC.md:805-816`; `doc/RELAY-SPEC.md:704-722`; `doc/RELAY-SPEC.md:839-878`.

- Signatures are optional in the operation and identity sections, default-required at the relay, and assumed universal by the threat model.
- Encryption is property-level and allows public properties, but the threat table says the relay sees only ciphertext.
- Censorship is detectable only by comparison with an independent source; a sole/eclipsing relay can present a self-consistent censored view or equivocate.
- Per-`PeerId` rate limiting does not by itself mitigate Sybil identities, and the optional proof-of-work message fields are not specified.

Make signatures mandatory for interoperable v1 or label an unsigned mode explicitly insecure. State that only encrypted fields are confidential unless whole-operation encryption is added. Qualify censorship detection and define its independent-source requirement.

### H9. Wire schemas and advertised limits contain concrete inconsistencies

**Evidence:** `doc/RELAY-SPEC.md:111-124`; `doc/RELAY-SPEC.md:182-196`; `doc/RELAY-SPEC.md:352-384`; `doc/RELAY-SPEC.md:424-460`; `doc/RELAY-SPEC.md:658-667`; `doc/RELAY-SPEC.md:974-1007`; `doc/RELAY-SPEC.md:1120-1129`.

- `LIVE_OP_BATCH` is bidirectional although `RELAY_OP` is the separate relay-to-peer form.
- Signaling forwarding must attach sender identity, but no forwarded payload shape includes it.
- `max_batch_bytes` is recommended but absent from `WELCOME.limits`; bytes-per-second is likewise not announced there.
- Identifiers and hashes lack a normative CBOR/JSON length and text/binary encoding table.

Generate the registry, schemas, transcript, and limits table from one machine-readable protocol definition, then validate golden and negative fixtures in at least two languages.

### H10. The encrypted-property and key-lifecycle protocol is missing

**Evidence:** `doc/SPEC.md:634-641`; `doc/SPEC.md:805-816`; `doc/EXEMPLAR.md:4-17`.

The specification names X25519 and XChaCha20-Poly1305 and mentions property-level encryption and key rotation, but does not define an encrypted-property envelope, nonce construction, associated data, key IDs, recipient/group-key distribution, adding/removing recipients, rotation after compromise, revocation of an offline peer, or bootstrap ordering between encrypted data and key material.

Without those contracts, the exemplar's private and shared data cannot be implemented interoperably over an untrusted relay. Define the complete envelope and key lifecycle, bind ciphertext to datastore/entity/field/schema context, and add compromise, removal, late-join, rotation, and offline-recipient tests before private-data claims.

### H11. The protocol has no durable relay acknowledgement

**Evidence:** `doc/SPEC.md:170-192`; `doc/RELAY-SPEC.md:365-373`; `doc/RELAY-SPEC.md:583-594`; `doc/RELAY-SPEC.md:627-636`.

`OP_ACK` explicitly acknowledges receipt, and the routing sequence acknowledges before Level 2 persistence. That is not itself loss of the originating peer's source-of-truth oplog, but it gives callers no signal that a persistent relay has durably committed the backup/catch-up copy.

Define separate receipt and durable-commit acknowledgements, or require persistence before an L2-specific durable acknowledgement. Specify sender retention, duplicate re-acknowledgement, timeout, reconnect, and retransmission behavior so applications know when relay durability may be relied upon.

---

## 3. Plan and roadmap findings

### R1. No executable standalone implementation plan exists

There is no `PLAN.md` or equivalent executable task/dependency document in the repository. The roadmap at `doc/SPEC.md:870-913` is a rudimentary plan in the form of unchecked components, but it does not identify owners, dependencies, target releases/dates, risks, issue IDs, acceptance owners, test commands, rollback, or definitions of done.

Create an executable plan whose rows include:

`Milestone | user-visible outcome | requirement IDs | deliverables | dependencies | owner | status/target | acceptance gates | rollback/risks`

### R2. Phase 1 is not a runnable vertical slice

**Evidence:** `doc/SPEC.md:30-35`; `doc/SPEC.md:68-91`; `doc/SPEC.md:94-253`; `doc/SPEC.md:483-504`; `doc/SPEC.md:872-889`.

Phase 1 combines SQLite, a WASM TypeScript SDK, CLI/query work, and WebSocket sync without choosing a coherent first runtime:

- browser WASM needs IndexedDB/OPFS, deferred to Phase 2;
- Node + SQLite needs the NAPI/native binding, not listed in Phase 1;
- create/query/mutate require graph entities, materialization, groups, referential behavior, indexing, and a query subset, most of which are absent as deliverables.

Choose one first vertical slice. Rust core + SQLite + CLI is the smallest coherent foundation; Node/NAPI + SQLite or browser/WASM + IndexedDB can follow as a separately gated SDK slice.

### R3. Phase 1 network sync lacks a defined profile and security prerequisites

**Evidence:** `doc/SPEC.md:415-434`; `doc/SPEC.md:624-657`; `doc/SPEC.md:805-866`; `doc/SPEC.md:872-905`.

Cold/live WebSocket sync is Phase 1. Node-to-Node WebSocket is listed as possible, but the direct-peer protocol is not specified; if Phase 1 instead intends relay-mediated sync, the reference relay is deferred to Phase 3. Identity, mandatory operation signing, ACL/capability/quarantine, datastore admission, and a conformance harness have no roadmap item. E2E encryption is Phase 3 and the external security audit is Phase 4.

Define Phase 1 sync as an in-process/two-peer test harness, or bring a minimal secure peer/relay profile and its negative tests before any deployable network milestone. Do not label Phase 3 “production hardening” when the audit follows in Phase 4.

### R4. Major specification surfaces are absent from the roadmap

| Requirement area | Present roadmap coverage | Missing or implicit work |
|---|---|---|
| Graph/materialization (`doc/SPEC.md:94-253`) | “Rust core” and oplog | Entity creation, materializer, groups, delete/referential state machine |
| Schema/CRDT (`doc/SPEC.md:257-375`) | Types split across Phases 1-3; migrations in Phase 2 | `Flag`, canonical schema compiler, strict/schemaless semantics, schema epochs |
| Query/SDK (`doc/SPEC.md:438-620`) | Basic CLI/SDK and hooks | Query grammar, subscriptions, batch/resolve semantics, compatibility |
| Identity/security (`doc/SPEC.md:624-657`, `:805-866`) | E2E/key rotation in Phase 3 | Identity bootstrap, signing, keys, datastore admission, ACL/capabilities/quarantine |
| Storage (`doc/SPEC.md:661-764`) | Adapters and GC | Atomic contract, secondary indexes, backup/restore, corruption recovery |
| Relay (`doc/RELAY-SPEC.md:50-970`) | One reference-relay bullet | Target level, conformance suite, deployment, federation, discovery, observability |
| Verification (`doc/SPEC.md:768-801`) | Lean in Phase 4 | Unit/property/model/integration/fuzz/crash/cross-language tests |
| Exemplar (`doc/EXEMPLAR.md:3-28`) | None | No milestone consumes its acceptance scenarios |

Assign stable requirement IDs and maintain a requirement -> issue -> milestone -> test/proof traceability matrix.

### R5. Scheduled work depends on unresolved decisions

**Evidence:** `doc/SPEC.md:935-973`; `doc/ISSUES.md:1-3`.

Schema apply/migrations, querying, WASM, and sync are scheduled before decisions on schema versioning, canonical schema source, query subset, WASM size, operation/blob limits, and migration compatibility. Some questions are stale or split-brain: the relay partially specifies limits while the core still lists them as open; `ISSUES.md` asks Lamport-versus-HLC after HLC has already been adopted; Flutter is not connected to the Swift/Kotlin ecosystem phase.

Move blocking choices into Milestone 0 with decision IDs, owner, due point, status, alternatives, rationale, and affected milestones. Keep resolved decisions out of the active issue list.

### R6. There is no definition of done or conformance program

**Evidence:** `doc/SPEC.md:768-801`; `doc/SPEC.md:870-913`; `doc/RELAY-SPEC.md:50-90`; `doc/RELAY-SPEC.md:1013-1166`; `doc/EXEMPLAR.md:13-28`.

The roadmap contains no red/green tests, failure scenarios, quality budgets, release gates, or test commands. Relay conformance promises third-party interoperability but offers no canonical fixtures, negative vectors, reference state machine, compatibility matrix, or certification harness. The only transcript skips the critical delta exchange.

At minimum, require:

- CRDT algebra/property/model tests and deterministic replay under every operation ordering;
- crash tests at every oplog/materialization/group commit boundary;
- duplicate, replay, reorder, loss, partition, reconnect, backpressure, and resume tests;
- storage-adapter contract tests and Rust/WASM/NAPI parity vectors;
- migration/rollback and adjacent-version interoperability tests;
- canonical CBOR/hash/signature/Merkle positive and negative vectors in two languages;
- fuzzing of decoders, operations, queries, migrations, and sync state machines;
- the exemplar as end-to-end acceptance: offline CRUD, restart, conflict, reconnect, sharing, private/public data, and multi-store behavior.

### R7. Formal verification is late and under-scoped relative to the promise

**Evidence:** `doc/SPEC.md:20-28`; `doc/SPEC.md:768-801`; `doc/SPEC.md:890-905`.

Formal proof is presented as a defining architectural bet. The specification names LWW, Counter, ORSet, MVRegister, RGA, HLC, sync completeness, and schema migration obligations; the roadmap omits MVRegister, RGA, and migration proofs and places all proof work after snapshot/compaction implementation.

Establish executable models and proof statements while formats remain changeable. Use model/property tests from the start. Make the agreed proof and Rust-reference conformance artifacts a GA gate rather than post-hardening cleanup.

### R8. Production operations and recovery are not planned

**Evidence:** `doc/SPEC.md:424-432`; `doc/SPEC.md:461-464`; `doc/SPEC.md:720-729`; `doc/RELAY-SPEC.md:625-652`; `doc/RELAY-SPEC.md:890-936`.

A relay is described as backup/catch-up infrastructure, yet the CLI shows export without restore and the roadmap has no backup verification, restore drill, corruption recovery, deployment/configuration, key/secret handling, rolling upgrade, SLO, load/soak, failure injection, or incident runbook. Relay metrics/health/logging are specified but not roadmapped.

Treat these as GA gates. A snapshot or retained relay is not a backup until restoration and causal continuity are tested.

### R9. Product scope, documentation, and issue tracking are too weak for contributors

**Evidence:** `README.md:1-2`; `doc/SPEC.md:3-6`; `doc/SPEC.md:30-35`; `doc/SPEC.md:870-913`; `doc/EXEMPLAR.md:3-28`; `doc/ISSUES.md:1-3`.

Both specs are drafts, all roadmap boxes are unchecked, and no distinction exists between committed, exploratory, and aspirational scope. The README does not link the specs or explain status, MVP, roadmap, contribution workflow, or validation. The two-line issue list has no IDs or lifecycle. The exemplar is the closest MVP anchor but lacks personas, prioritized workflows, metrics, or milestone links.

Select one v0.1 user, runtime, and complete workflow; state explicit non-goals; turn the exemplar into acceptance scenarios; and give issues/decisions owners and statuses.

---

## 4. Medium-priority corrections

1. **Normative authority is unclear.** Both documents are `0.1.0-draft`, while the wire protocol calls itself version 1 in several layers. State which document/type registry is authoritative and how document versions map to wire versions (`doc/SPEC.md:3-6`; `doc/RELAY-SPEC.md:3-7`, `:111-124`, `:942-949`).

2. **The “fully specified” relay claim is premature.** The core uses that phrase at `doc/SPEC.md:434`, but the companion lacks Merkle traversal, snapshot messages, complete operation encoding, delivery semantics, and conformance vectors. Mark it conceptual until Milestone 0 exits.

3. **The dual-schema source remains a maintenance hazard.** TypeScript examples include validations absent from the DSL, and the Rust core is meant to be authoritative. Decide whether one schema is generated from the other (`doc/SPEC.md:278-336`, `:961`).

4. **Query scope is too open for Phase 1 estimates.** “Cypher-inspired” examples are not a grammar or error/typing contract. Decide the v0.1 subset, null/conflict semantics, parameterization, index guarantees, and traversal limits (`doc/SPEC.md:483-504`, `:965`).

5. **Causal context scale and compaction are unaddressed.** `deps` is described as the last-seen operation per peer, which grows with peer count and can reference compacted operations. Define a compact causal frontier and snapshot/checkpoint translation (`doc/SPEC.md:175-190`, `:720-729`).

6. **Several smaller cross-references are stale.** `Operation.group` points to section 2.7 rather than 2.8 (`doc/SPEC.md:183`); the roadmap does not actually name the admin dashboard promised at `doc/SPEC.md:371`; `ops_since(VersionVector)` survives a decision that removed version vectors (`doc/SPEC.md:673`, `:953`).

7. **The competitive and security wording should distinguish goals from demonstrated properties.** Formal proofs, independent relay interoperability, performance, censorship detection, and built-in confidentiality are planned or conditional, not current guarantees (`doc/SPEC.md:917-931`).

8. **Relay schema awareness contradicts the schema-blind model.** The core requires relays to warn when no schema is registered and surface it in an admin dashboard, while the relay has no registration message and is forbidden from schema/CRDT interpretation (`doc/SPEC.md:368-373`; `doc/RELAY-SPEC.md:35-46`, `:716-720`). Remove the promise or define a separate authenticated, non-authoritative metadata channel.

9. **Unconstrained pluggable cryptography conflicts with the fixed Ed25519 wire suite.** The core permits institutional PKI or WebAuthn-backed providers, while relay identity and validation require 32-byte Ed25519 keys/signatures (`doc/SPEC.md:626-657`; `doc/RELAY-SPEC.md:136-176`, `:704-714`). Constrain interoperable providers to the suite or negotiate algorithms, encodings, and `PeerId` derivation.

---

## 5. Recommended replacement roadmap

Each milestone should end in a runnable outcome and begin with failing contract/acceptance tests for its stated behavior.

### M0 - Decisions and executable contracts

**Outcome:** Two independent toy implementations can encode, sign, hash, decode, and validate the same operations and Merkle fixtures.

**Deliverables:**

- v0.1 scope, first runtime, trust model, query subset, and explicit non-goals;
- canonical operation algebra, identifier table, deterministic CBOR, signing/hash context;
- canonical schema representation, epochs, migration model, and CRDT operation semantics;
- datastore admission/encryption decision and author-key/rotation model;
- canonical Merkle structure and complete sync state machine;
- delivery/ack/retry/group contracts and protocol/version compatibility policy;
- stable requirement/decision IDs and golden/negative fixtures.

**Exit gate:** All eight critical findings above have approved normative resolutions and red conformance tests. No network or persistent format is frozen before this gate.

### M1 - Local durable core

**Outcome:** Rust + SQLite + CLI completes offline exemplar CRUD and deterministic restart/replay.

**Deliverables:** Graph entities, initial CRDT set, HLC, oplog, materializer, groups, delete/referential state machine, canonical schema/strict mode, basic query, indexes, and atomic recovery.

**Exit gate:** Property/model tests, randomized replay equivalence, crash atomicity, storage contract tests, duplicate/replay tests, and offline exemplar acceptance pass.

### M2 - One SDK vertical

**Outcome:** One coherent SDK/storage pair runs the same exemplar and produces byte-identical core fixtures.

Choose Node/NAPI + SQLite or browser/WASM + IndexedDB; do not schedule both as one undifferentiated task.

**Exit gate:** Binding parity, subscriptions/mutations/conflict resolution, lifecycle tests, artifact-size budget, and baseline performance budget pass.

### M3 - Secure multi-peer sync

**Outcome:** Three peers converge through partition/reorder/retry using one WebSocket profile and a minimal relay level.

**Deliverables:** Mandatory identity/signing policy, author-key resolution, datastore admission, E2E encrypted-property/store envelope and recipient/group-key distribution/rotation/revocation, peer handshake, complete Merkle/delta protocol, delivery semantics, bounded resources, and reference relay/conformance harness.

**Exit gate:** Two-language interoperability; partition/rejoin; duplicate/loss/reorder; malicious operation, datastore, clock, auth, and resource-limit negatives; exemplar sharing/private-data behavior.

### M4 - Browser/P2P and evolution

**Outcome:** Remaining browser storage, WebRTC direct sync, schema migration, snapshot bootstrap, and adjacent-version sync work without data loss.

**Exit gate:** Upgrade/downgrade/rollback matrix, mixed-schema peers, snapshot + tail recovery, direct/relay parity, browser restart/offline tests.

### M5 - Production readiness and GA

**Outcome:** A deployable, observable, recoverable system with evidence for its published guarantees.

**Deliverables:** Compaction/GC, cryptographic hardening and recovery, backup/restore, packaging/configuration, metrics/logs/traces, SLOs, fuzz/load/soak/failure injection, proof artifacts, external security audit, and closed audit findings.

**Exit gate:** Restore drill, forgotten/offline-peer GC cases, rolling upgrade, published conformance suite, proof/Rust conformance, performance budgets, and security sign-off.

### M6 - Ecosystem

Mobile/Flutter decision, additional bindings, custom CRDT/plugin policy, hosted relay, visual/admin tooling, and query optimization follow only after format and compatibility commitments are stable.

---

## 6. Immediate decision queue

Resolve these in order because later choices depend on earlier ones:

1. **First product slice:** Rust + SQLite + CLI is recommended; identify the v0.1 user/workflow and non-goals.
2. **Canonical operation:** variants, bytes, hash/signature inputs, datastore/schema/group context, author keys.
3. **Schema authority and epochs:** one source, serializable migrations, mixed-version behavior.
4. **Trust/privacy model:** mandatory signatures; datastore membership versus public encrypted replication; ACL causal semantics.
5. **Sync/checkpoint model:** canonical Merkle tree, traversal, delivery, snapshot, compaction, and causal frontier.
6. **Version policy:** wire, operation, schema, snapshot, and storage compatibility/upgrade rules.
7. **Definition of done:** requirement IDs, TDD/conformance matrix, exemplar scenarios, quality budgets, owner/status fields.

Until these are settled, the current Phase 1 checklist should be treated as a concept inventory rather than an approved implementation plan.

---

## 7. Review boundaries

This review verified the repository documents against one another and evaluated whether their stated contracts are implementable. It did not validate competitive-landscape claims with external benchmarks, choose final CRDT algorithms, or review implementation code because no implementation/test suite is present in the repository. Findings intentionally distinguish confirmed contradictions/gaps from design choices that still need an explicit owner decision.
