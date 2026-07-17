# ZeroDB — Issues & Decisions

Single tracking list for specification issues and pending design decisions.
Resolved items are **removed**; durable outcomes land in the Decision Log at the bottom.

- **Critical (C):** blocks a correct/interoperable implementation or permits data loss or a security-boundary failure. Critical contracts **C1–C5, C7–C8** must have approved normative resolutions before any wire or persistent format freezes — this is the composite **M0** exit gate, delivered as packages **M0a–M0f** ([SPEC §10](SPEC.md)). **C6** is deferred past v0.1 and is not part of composite M0.
- **High (H):** leaves a major guarantee, lifecycle, or roadmap outcome unreliable.
- **Open question (O):** design decision needing an owner and a milestone.

**Approved resolution** (for M0 package exit) requires all of: (1) normative SPEC/RELAY prose, (2) machine-readable artifact where applicable, (3) golden positive + negative vectors with an automated harness, (4) Decision Log entry, (5) issue removed per this file’s policy.  “Direction decided” is not a resolution.  Full checklist: [SPEC §10](SPEC.md).

Consolidated 2026-07-13 from the external review (`FINDINGS.CODEX.md`, retired) after the M0–M6 roadmap was adopted into SPEC §10.  M0 package-split 2026-07-15 (`FINDINGS.GROK.md` C-P1).

---

## M0 package map

| Package | Contracts | Outcome (summary) | Implement / ship |
|---------|-----------|-------------------|------------------|
| **M0a** | C1, C4 *context*, O6 *provisional limits* | Op algebra, deterministic CBOR, preimages, ID encodings | codecs experimental until composite M0 |
| **M0b** | C2, O2, O3 | Schema IR, epochs, migration DSL, minimal query grammar | M1 uses IR + query; O2 generation M2; cross-peer migration M4 |
| **M0c** | C3 | Canonical Merkle tree + sync state-machine transcripts | wire protocol M3 |
| **M0d** | C4 *admission*, C5 | Author-key model + membership capability format | enforcement M3 |
| **M0e** | C8, H4, H7, H9 *registry*, H11 *contract* | Groups, delivery/anti-replay, version policy | local crash M1; sync/ack M3 |
| **M0f** | C7 *contracts*, O7 *contract* | Frontiers, snapshot identity, deps compaction rules | snapshots M4; GC M5 (**GC disabled until then**) |

```
M0a ──► M0b ──► M0c
 │        │
 └────────┴──► M0d
M0a ──► M0e
M0c + M0e ──► M0f
```

Second language for M0 golden vectors: **TypeScript pure encoder/decoder** under `conformance/` (not the full SDK).  Lean 4 **proofs** do not gate M0 (assurance track / M5+).

---

## Critical — composite M0 exit (C1–C5, C7–C8)

### C1 — Canonical operation format
`Operation` (SPEC §2.5) lacks: variants for node/edge creation, labels, edge endpoints, migrations, capability grants, and key rotation; the datastore ID, schema epoch, entity kind, and author public key; a canonical byte encoding; and hash/signature preimage rules (including whether `id` and `signature` are excluded from their own preimages). The relay requires `BLAKE3(operation_content) == id` but `operation_content` is undefined — CBOR serializability alone does not select unique bytes.
**Resolution:** versioned operation algebra + normative wire schema: every variant, fixed identifier encodings/lengths, deterministic CBOR rules, duplicate-key rejection, domain-separated preimages, signed context, golden byte-level vectors; generate Rust/TypeScript types from the canonical schema. → **M0a** (schema-epoch field binding completed with **M0b** / C2)

### C2 — Schema evolution breaks deterministic replay
CRDT type is resolved from the *current* mutable schema, but migrations can change types and arrive at peers in different orders; JavaScript resolver closures cannot replicate deterministically across languages; the initial schema has no replicated identity; historical replay after a type change is undefined. Violates the SEC guarantee (SPEC §2.7).
**Resolution:** one canonical schema representation with immutable IDs/versions; a causally ordered schema epoch bound to every data operation; CRDT variant carried in the operation or bound to an immutable schema version; deterministic migration DSL (no closures); mixed-version buffering/rejection and rollback rules. → **M0b** (cross-peer migration ships M4)

### C3 — Merkle sync is not executable from the specified messages
Core says peers walk the remote tree root-down; the relay protocol exchanges only roots and then expects the requester to name `missing_hashes` it has no way to discover — no subtree/child/manifest request exists. The tree itself is non-canonical: bucket boundaries, leaf ordering, empty-node hashes, shape, internal-node encoding, late operations, and compaction representation are unspecified. Equal oplogs may hash unequal roots; unequal roots cannot be traversed.
**Resolution:** canonical authenticated tree + published root vectors; path/range subtree request/response (or paginated bucket manifests); `sync_id`, checkpoint/epoch, pagination/retry rules, concurrent-write semantics; a complete two-way transcript including mismatch recovery. → **M0c** (wire protocol ships M3)

### C4 — Datastore boundary is neither authorized nor signed
Datastores are declared replication/RBAC boundaries, but the relay has no datastore admission credential — any authenticated key can subscribe to guessed datastores. The datastore ID sits outside the signed operation, so a valid op can be replayed into another datastore without breaking its signature; global OpId dedup can also suppress legitimate reuse across independent datastores.
**Resolution (direction decided 2026-07-13):** relay-verifiable **datastore-membership capabilities**; canonical `DatastoreId`, protocol version, and schema epoch inside the signed/hashed operation context; dedup scoped per datastore (already normative in RELAY-SPEC 0.2 §6.2). → **M0a** (signed context fields) + **M0d** (membership capability format); enforcement ships M3.  *Direction only until checklist complete.*

### C5 — Forwarded author signatures cannot be verified
Operations carry an author `PeerId` (a key *hash*) and signature but no resolvable public key. Relay validation verifies against the transport sender's key when author == sender; for forwarded history (bridging, relay-to-relay sync) RELAY-SPEC 0.2 currently instructs relays **not to reject** unresolved authors — i.e. forward unverified — which contradicts the validate-every-operation MUST. One deterministic unresolved-author outcome (reject, quarantine, or authenticated key resolution) is required. Key rotation compounds the missing lookup contract.
**Resolution:** distinguish transport sender from author; carry or resolve an authenticated author key/certificate verified against `operation.peer`; specify rotation/revocation and historical-key lookup; define out-of-order arrival of key records vs. signed ops. → **M0d** (resolution protocol / enforcement ships M3)

### C6 — Entity-level ACLs conflict with SEC and read confidentiality
Write-ACL evaluation at receipt time depends on arrival order (grant/revoke/create races); quarantine-vs-materialize divergence between origin and receivers is permanent absent a deterministic reevaluation protocol; bootstrap authority for the first grant is undefined; read ACLs cannot protect plaintext already replicated to a hostile peer.
**Resolution (direction decided 2026-07-13):** v0.1 ships **datastore-level access control only** (membership + mandatory signatures); entity-level distributed ACLs are deferred until a causal authorization model exists — root authority, grant/revoke ordering, policy/schema versions, deterministic accept/reject/quarantine, reevaluation, and SEC scoped to a precisely defined accepted-operation set. Confidential reads via replication boundaries + cryptography, not local filters. → **not part of composite M0**; design in M5/M6 window.  *Direction only until checklist complete.*

### C7 — Snapshot, compaction, and causal stability are unsafe as written
An HLC scalar acknowledged by "all known peers" is not a causal frontier; no durable per-peer acknowledgement, peer-membership lifecycle, retirement/lease, or reconnect rule exists ("known peers" undefined → collect too early or never). Level 2 relays are schema-blind yet expected to serve state snapshots; no snapshot request/chunk/format/signature/tail-boundary exists; compacted and uncompacted peers cannot compare Merkle roots without a shared checkpoint representation.
**Resolution:** causal frontiers independent of wall-clock; durable peer acks + retirement; authenticated checkpoint/snapshot identity and tail boundaries; anti-replay commitments; root comparison across checkpoints; decide whether L2 materializes or only stores peer-produced authenticated snapshots. GC stays **disabled** until partition/rejoin, forgotten-peer, late-op, and restore tests pass. → **contracts M0f**; snapshot shipping M4; GC implementation M5

### C8 — Group and crash atomicity cannot be implemented from the contracts
A grouped op carries only `GroupId` — no member count, manifest, or commit marker, so no peer or relay can know a group is complete (relay 0.2 removed group buffering for exactly this reason; group atomicity is peer-side). Storage traits expose `append_ops` and `put_materialized` with no transaction/recovery boundary spanning both.
**Resolution:** signed group manifest (or cardinality/index/member hashes) + abort/expiry semantics; atomic storage transaction or WAL/replay protocol with explicit crash points and idempotent recovery tests. → **contracts M0e**; local half M1; sync half M3

---

## High

### H1 — Future-clock poisoning detected but not mitigated
Logical-counter caps don't bound attacker-controlled `physical_time`; a far-future signed HLC wins LWW until real time catches up. Core warns-never-blocks while relays may reject — divergent behavior.  Define one peer-side acceptance/quarantine rule, max forward skew, recovery path, and semantics for locally-accepted-relay-rejected ops. → M3

### H2 — Offline unique indexes have no conflict semantics
Two offline peers can create the same "unique" value; mapping `unique: true` to SQLite/IDB uniqueness makes remote materialization fail platform-dependently.  Define advisory/conflict-reporting uniqueness, an ownership CRDT, or required coordination — plus query/resolution behavior. → M5

### H3 — Delete/resurrection semantics incomplete
Unclear which peer generates cascading edge tombstones, how it's authorized to delete others' edges, whether late dangling edges get tombstoned or only hidden, whether node resurrection revives them, and which CRDT governs `__tombstone`. Cascades generated from each peer's current view produce divergent op sets (SEC risk). Define a deterministic delete state machine — **prefer derived visibility** over generated cascades unless a single deterministic emitter is specified. → **M1 exit gate** (treat as release-blocking for M1 despite High label)

### H4 — Delivery, dedup, and replay semantics incomplete
Dedup state is bounded/resettable (L1) or compactable (L2), so old signed counter/set ops can replay after it disappears. No explicit delivery contract; per-op outcomes within a batch undefined; multi-batch deltas lack sequence/resume cursors. Define at-least-once semantics, durable anti-replay commitment surviving compaction, request-ID lifetime, per-op outcomes, resumable cursors, retry/backoff. → **contracts M0e**; enforcement M3

### H5 — Handshake, resumption, and serialization underspecified
Narrowed by RELAY-SPEC 0.2 (CBOR-only, no session resumption, no in-protocol mutual auth, TLS required outside dev, domain-separated nonce signature). Remaining: the `AUTH` signature covers only the nonce, not the negotiated handshake transcript (version, limits, transport binding) — bind it to a full transcript. → M3

### H6 — Direct P2P sync has no protocol
Core promises WebRTC/`connectPeer`; relay spec excludes direct sync; no peer handshake, role negotiation, datastore admission, reconnect, or conformance profile exists. Define a shared peer-sync protocol reused by relay participation (M3), or keep P2P out of the SDK surface until M4.

### H7 — Version and upgrade policy missing
No owner or compatibility policy relating wire, operation, schema, snapshot, and on-disk format versions; protocol version appears in four places without selection/negotiation rules; document versions (`0.x-draft`) don't map to wire `protocol_version: 1`. Define per-format authority, a supported version window, and rolling upgrade/downgrade tests. → **policy M0e**; rolling tests M4

### H8 — Security claims stronger than mechanisms
Narrowed by RELAY-SPEC 0.2 (proof-of-work removed; independent-second-source censorship requirement now normative in RELAY-SPEC §12.3). Remaining: whole-operation encryption (vs. property-level) is an open choice for metadata privacy. → M3

### H9 — RELAY-SPEC wire inconsistencies
Message-set inconsistencies fixed in RELAY-SPEC 0.2 (single bidirectional `OPS`; `SIGNAL` forwarded form carries `sender`; `max_batch_bytes`/`bytes_per_second` in `WELCOME.limits`). Remaining: no normative identifier/hash encoding-and-length table; generate registry, schemas, transcript, and limits from one machine-readable protocol definition; validate golden + negative fixtures in two languages. → **encoding registry with M0a/M0e**; full two-language wire harness M3

### H10 — Encrypted-property envelope and key lifecycle missing
X25519/XChaCha20-Poly1305 are named but there is no ciphertext envelope, nonce construction, AAD, key IDs, recipient/group-key distribution, recipient add/remove, post-compromise rotation, offline-peer revocation, or bootstrap ordering of keys vs. data. Required before any private-data claim (EXEMPLAR). → M3

### H11 — No durable relay acknowledgement
`OP_ACK` acknowledges receipt before L2 persistence; callers get no durable-commit signal for the backup/catch-up role relays are sold on. Define separate receipt vs. durable acks (or persistence-before-ack for L2), sender retention, duplicate re-ack, timeout/reconnect/retransmit behavior. → **contract M0e**; implementation with L2 relay (M3+; reference relay at M3 may be L1-only)

---

## Open questions

- **O1 — Large operation payloads.** Size limits vs. chunking vs. external blob storage; must be decided before Richtext (100 MB documents). → decide by M4
- **O2 — Schema source of truth.** TypeScript SDK vs. `.zerodb` DSL: one canonical, the other generated. → **decide M0b**, implement generation M2
- **O3 — Query language subset.** Minimal (MATCH/WHERE/RETURN/ORDER BY/LIMIT) vs. aggregation/paths; grammar, null/conflict semantics, parameterization, traversal limits. → **decide M0b** (minimal grammar required before M1 CLI)
- **O4 — WASM size budget.** Target vs. Automerge ~250 KB / Loro ~200 KB gz; optional modules for RGA/Richtext. → M4
- **O6 — Operation/batch size limits & protocol-level rate limiting.** Interacts with H4/H9. → **provisional limits M0a**; rate limiting M3
- **O7 — Causal `deps` scale.** Last-seen-op-per-peer grows with peer count and can reference compacted ops; needs a compact causal frontier + checkpoint translation. Interacts with C7. → **contract M0f**; scale tests M5

---

## Decision Log

| Date | Decision |
|------|----------|
| 2026-07-16 | **DQ-1..DQ-8 directions ratified** per `plan/DQ-PROPOSALS.md`: two-level identity (principal root + device certs, PeerId = BLAKE3(device pk)); self-certifying genesis `DatastoreId`; mandatory peer-side causal grant-time authorization (revocation defeats causally-later ops only); C8 closed in M0 via executable WAL reference model; property-level encryption with M0a-frozen envelope/AAD; payload caps + reserved `BlobRef` variant; HLC durability rides the atomic op commit (resume from oplog max); equal timestamps = cross-peer total order `(physical, logical, peer, op_id)` / same-device equivocation quarantine. *Directions only — each resolves via the SPEC §10 checklist in its owning package.* |
| 2026-07-16 | **O5 GunDB migration: won't do.** Clean break; no state-snapshot converter or migration tooling. "Successor to GunDB" refers to the developer experience, not data portability. |
| 2026-07-16 | **Delivery plan adopted:** `plan/PLAN.md` is the path-to-MVP delivery/tracking plan (P0 readiness package, revised M0 packages, M3a/b/c split, decision queue DQ-1..DQ-12). SPEC §10 remains the normative roadmap. |
| 2026-07-15 | **M0 package-split:** composite M0 delivered as **M0a–M0f** (op encoding; schema/query; Merkle SM; keys/membership; groups/delivery/versions; frontiers/snapshots). C6 excluded from composite M0 exit. Second M0 implementation = TS pure encoder under `conformance/`. Lean proofs do not gate M0. Pre-M0 implementation policy + approved-resolution checklist in SPEC §10. Release labels: `v0.1.0-local`=M1, `v0.1.0-sdk`=M2, `v0.1.0`=M3. (Responds to FINDINGS.GROK C-P1.) |
| 2026-07-13 | **Relay protocol 0.2:** pruned for simplicity — CBOR-only over WebSocket/DataChannel; 22 message types (`OPS`, `SIGNAL`, `THROTTLE` consolidations); removed JSON mode, session resumption, in-protocol mutual auth, proof-of-work, TCP/QUIC bindings, DNS/well-known discovery, and relay-side causal/group buffering. TLS required outside dev; per-datastore dedup; domain-separated auth signature. |
| 2026-07-13 | **Roadmap:** M0–M6 milestone plan adopted (SPEC §10), replacing Phase 1–5. M0 spec-stabilization precedes format freezes; no wire/persistent format freezes before composite M0 exit. |
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
