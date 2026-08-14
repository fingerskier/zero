# ZeroDB — Issues & Decisions

Single tracking list for specification issues and pending design decisions.
Resolved items are **removed**; durable outcomes land in the Decision Log at the bottom.

- **Critical (C):** blocks a correct/interoperable implementation or permits data loss or a security-boundary failure. Critical contracts **C1–C5, C7–C8** must have approved normative resolutions before any wire or persistent format freezes — this is the composite **M0** exit gate, delivered as packages **M0a–M0f** ([SPEC §10](SPEC.md)). **C6** is deferred past v0.1 and is not part of composite M0.
- **High (H):** leaves a major guarantee, lifecycle, or roadmap outcome unreliable.
- **Open question (O):** design decision needing an owner and a milestone.

**Approved resolution** (for M0 package exit) requires all of: (1) normative SPEC/RELAY prose, (2) machine-readable artifact where applicable, (3) golden positive + negative vectors with an automated harness, (4) Decision Log entry, (5) issue removed per this file’s policy.  “Direction decided” is not a resolution.  Full checklist: [SPEC §10](SPEC.md).

Consolidated 2026-07-13 from the external plan/spec reviews (archives removed 2026-07-19 after disposition) after the M0–M6 roadmap was adopted into SPEC §10. M0 package-split 2026-07-15 (C-P1).

---

## M0 package map

| Package | Contracts | Outcome (summary) | Implement / ship |
|---------|-----------|-------------------|------------------|
| **M0a** ✓ | C1 ✓, C4 *context* ✓, O6 *provisional limits* ✓ | Op algebra, deterministic CBOR, preimages, ID encodings — resolved 2026-07-16 ([KERNEL.md](KERNEL.md)) | draft-1 profile; unfrozen until Decision Log freeze |
| **M0b** ✓ | C2 ✓, O2 ✓, O3 ✓ | Schema IR, epochs, migration DSL, minimal query — resolved 2026-07-18 ([SCHEMA.md](SCHEMA.md)) | draft-1 profile; TS→IR compiler ≤ M1; cross-peer migration M4 |
| **M0c** ✓ | C3 ✓ | Canonical Merkle tree + sync transcripts — resolved 2026-07-18 ([MERKLE.md](MERKLE.md)) | draft-1 profile; wire framing M3 |
| **M0d** ✓ | C4 *admission* ✓, C5 ✓ | Identity, genesis, membership, authz — resolved 2026-07-18 ([AUTH.md](AUTH.md)) | draft-1 profile; on-wire enforcement M3b |
| **M0e** ✓ | C8 ✓, H4 ✓, H7 ✓, H9 *registry* ✓, H11 *contract* ✓ | Groups/WAL, delivery, versions — resolved 2026-07-18 ([WAL.md](WAL.md), [DELIVERY.md](DELIVERY.md), [VERSIONS.md](VERSIONS.md)) | draft-1; SQLite layer 2 M1; wire M3 |
| **M0f** ✓ | C7 ✓, O7 ✓ | Frontiers, snapshot identity — resolved 2026-07-18 ([FRONTIER.md](FRONTIER.md)); **GC still disabled** | snapshots M4; GC M5 |

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

*C1 resolved 2026-07-16 (Decision Log below; contract in [KERNEL.md](KERNEL.md); Decision Log entry below).*
*C2 resolved 2026-07-18 (Decision Log below; contract in [SCHEMA.md](SCHEMA.md); Decision Log entry below). Cross-peer migration shipping remains M4.*
*C4 fully resolved 2026-07-18 (context half 2026-07-16 in KERNEL; admission half in [AUTH.md](AUTH.md); Decision Log entry below). On-wire admission enforcement ships M3b.*
*C5 resolved 2026-07-18 (Decision Log below; contract in [AUTH.md](AUTH.md); Decision Log entry below). On-wire author-key resolution enforcement ships M3b.*
*C3 resolved 2026-07-18 (Decision Log below; contract in [MERKLE.md](MERKLE.md); Decision Log entry below). Wire framing of walk messages ships M3.*
*C7 resolved 2026-07-18 (Decision Log below; contract in [FRONTIER.md](FRONTIER.md); **GC remains disabled** until M5).*
*C8 resolved 2026-07-18 (Decision Log below; contract in [WAL.md](WAL.md); SQLite layer 2 at M1).*
***Composite M0 exit 2026-07-18:** C1–C5, C7–C8 approved; C6 deferred. Draft-1 only — no format freeze without explicit Decision Log freeze.*

### C6 — Entity-level ACLs conflict with SEC and read confidentiality
Write-ACL evaluation at receipt time depends on arrival order (grant/revoke/create races); quarantine-vs-materialize divergence between origin and receivers is permanent absent a deterministic reevaluation protocol; bootstrap authority for the first grant is undefined; read ACLs cannot protect plaintext already replicated to a hostile peer.
**Resolution (direction decided 2026-07-13):** v0.1 ships **datastore-level access control only** (membership + mandatory signatures); entity-level distributed ACLs are deferred until a causal authorization model exists — root authority, grant/revoke ordering, policy/schema versions, deterministic accept/reject/quarantine, reevaluation, and SEC scoped to a precisely defined accepted-operation set. Confidential reads via replication boundaries + cryptography, not local filters. → **not part of composite M0**; design in M5/M6 window.  *Direction only until checklist complete.*


---

## High

### H1 — Future-clock poisoning detected but not mitigated
Logical-counter caps don't bound attacker-controlled `physical_time`; a far-future signed HLC wins LWW until real time catches up. Core warns-never-blocks while relays may reject — divergent behavior.  Define one peer-side acceptance/quarantine rule, max forward skew, recovery path, and semantics for locally-accepted-relay-rejected ops. → M3

### H2 — Offline unique indexes have no conflict semantics
Two offline peers can create the same "unique" value; mapping `unique: true` to SQLite/IDB uniqueness makes remote materialization fail platform-dependently.  Define advisory/conflict-reporting uniqueness, an ownership CRDT, or required coordination — plus query/resolution behavior. → M5

### H4 — Delivery, dedup, and replay semantics incomplete
*Resolved 2026-07-18 (contract):* [DELIVERY.md](DELIVERY.md) + DELIV-001..004. Enforcement M3.

### H5 — Handshake, resumption, and serialization underspecified
Narrowed by RELAY-SPEC 0.2 (CBOR-only, no session resumption, no in-protocol mutual auth, TLS required outside dev, domain-separated nonce signature). Remaining: the `AUTH` signature covers only the nonce, not the negotiated handshake transcript (version, limits, transport binding) — bind it to a full transcript. → M3

### H6 — Direct P2P sync has no protocol
Core promises WebRTC/`connectPeer`; relay spec excludes direct sync; no peer handshake, role negotiation, datastore admission, reconnect, or conformance profile exists. Define a shared peer-sync protocol reused by relay participation (M3), or keep P2P out of the SDK surface until M4.

### H7 — Version and upgrade policy missing
*Resolved 2026-07-18 (policy contract):* [VERSIONS.md](VERSIONS.md) + registry. Rolling upgrade tests M4.

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
- **O4 — WASM size budget.** Target vs. Automerge ~250 KB / Loro ~200 KB gz; optional modules for RGA/Richtext. → M4
- **O6 — Operation/batch size limits & protocol-level rate limiting.** Interacts with H4/H9. → **provisional limits M0a**; rate limiting M3
- **O7 — Causal `deps` scale.** Last-seen-op-per-peer grows with peer count and can reference compacted ops; needs a compact causal frontier + checkpoint translation. Interacts with C7. → **contract M0f**; scale tests M5

---

## Decision Log

| Date | Decision |
|------|----------|
| 2026-08-14 | **M3a wire transcripts (no relay binary).** RELAY-SPEC **0.2.2-draft**: `HELLO`/`WELCOME.capabilities` (sorted intersection of `dual-root`, `resume-cursor`, `reject-ack`); L2 dual roots; `SYNC_REQUEST.cursor`; `OP_ACK.outcomes` with non-retryable `REJECT`. Five `relay-transcript` vectors green in both runners (`RELAY-HELLO-001/002`, `RELAY-ROOT-001`, `RELAY-RESUME-001`, `RELAY-REJECT-001`). Required corpus **114**. Dual-root Merkle *walk* and a relay process remain M3a proper. |
| 2026-08-14 | **M2 exit — `v0.1.0-sdk` (experimental format):** Narrowed checklist closed. Shipped: sync NAPI `Database` (CRUD, subscribe, O3 query + params, edges, `listNodes.props`), `applySchema` (pin or SCHEMA IR → persist `SchemaId`/`ep=1`), import `ep`/`deps` enforcement, thin `zerodb.mjs` facade, WS v2 + push, byte-level CRDT fixture replay (`applyCrdtVector` over `required/crdt/*`). Evidence: `m2-*.test.mjs` (28), `m2_schema` + `r0_stabilize`, CI [run 31836860347](https://github.com/fingerskier/zero/actions/runs/31836860347) @ `b352ca4` (rust, NAPI ubuntu+windows, conformance, ts-to-ir). Tag peels to this Decision Log commit. **Not claimed:** SPEC-complete M2, MVRegister/RGA/LWWMap, E11 budgets, query-scoped subscribe, interactive `repl`, format freeze. Wire/bundle/SQLite/event shapes stay draft-1. **Do not start M3** until wire transcripts exist against RELAY 0.2.1-draft. |
| 2026-08-14 | **M2-parity closed.** NAPI `applyCrdtVector` replays every required `crdt-apply` fixture (`conformance/vectors/required/crdt/*`, 9 vectors) through `zerodb_core::apply_crdt_vector` — the same kernel runner as `conformance_crdt`. Covers equal-ts total order, equivocation exclusion, BlobRef reject, observed-remove dots, and replay dedup. High-level semantic smoke in `m2-parity.test.mjs` is unchanged. Narrowed `v0.1.0-sdk` implementation checklist is now complete; the tag itself remains a separate Decision Log act. **Do not start M3.** |
| 2026-08-14 | **M2-schema + binding slice.** `apply_schema_json` compiles pin or IR JSON to canonical CBOR IR, persists `SchemaId`/`schema_ep=1`, stamps `ep` on local ops, and rejects import deps >64 or unknown. NAPI `applySchema`/`createEdge`/`query(q,params)`; `listNodes` includes `props`. Thin `zerodb.mjs` facade. `repl` still deferred. |
| 2026-08-14 | **CX-08 accepted-set (direction + RELAY §7.4).** L2 `validated_root` ≠ peer `accepted_root` under unauthorized-but-authentic ops. v0.1 catch-up invariant is honest-peer accepted-root equality, not relay/peer root equality. Rejected OpIds MUST be acked. Dual-root wire messages are M3a; do not implement a relay against RELAY 0.2.0-draft claiming root equality. |
| 2026-08-14 | **M2a-m0 contract amendments (CX-03..06).** (1) KERNEL §7 AAD is `domain(value_aad) ‖ SlotId` where `SlotId = BLAKE3(domain(value_slot) ‖ ds ‖ ep ‖ path ‖ author ‖ ts)` — no OpId, no ciphertext; ENV-001 constructs a complete op after seal. (2) FRONTIER tip is `{op_id, physical_ms, logical}`; `is_late_op` uses the encoded map (FRONT-004). Snapshot hash includes those bytes; signed snapshot envelope still M4. (3) DELIVERY cursor is `{frontier, epoch}`; resume is independent sender-held vs receiver frontier (DELIV-005: covered late op is not retransmitted). (4) WAL model enforces hlc_persist ⊆ synced wal, unique members + optional `n`, group-id binding, and application-visible hide of unsealed members (WAL-013..016). Required corpus **109**. Freeze still open. |
| 2026-08-14 | **M2a adopted; M2 exit narrowed; do not start M3.** Current work is M2a (honesty + store leftovers + M0 implementability + schema + binding parity) per [PLAN.md](../plan/PLAN.md) §5 and [LEDGER.md](../plan/LEDGER.md). `v0.1.0-sdk` requires: canonical CBOR schema IR + `SchemaId` in the store, NAPI `applySchema`, import-time pin/`ep`/`deps` enforcement, NAPI/wasm edge CRUD + `listNodes` parity, thin promise/typed JS facade. **Not** required for that tag: MVRegister/RGA/LWWMap (still app-triggered), E11 budgets, query-scoped subscribe, interactive `repl`. M3a stays blocked on M2a-m0 + M2a-relay (CX-03..06, CX-08, RELAY rewrite). Freeze stays a separate Decision Log act. July FINDINGS archived under `plan/archive/` — CX-01/CX-02 are historical. |
| 2026-08-14 | **H3 closed as derived visibility.** Node/edge tombstones are set-derived; edge visible iff not tombstoned and both endpoints live; no cascade ops (`e9_delete_machine`, `r0_stabilize`). Remaining same-id CreateNode-after-tombstone and conflicting-label creates are **M2a-store** leftovers, not an open High contract hole. |
| 2026-08-14 | **CX-03 AAD direction (M2a-m0):** bind envelope AAD to a **pre-encryption slot-context hash** (`ds ‖ ep ‖ path ‖ author ‖ ts` or equivalent), not the final `OpId`. Keeps I-10 (replay into another slot fails) without the ciphertext↔OpId construction cycle. End-to-end vector (plaintext → envelope → complete signed op → OpId → decrypt) is required before any freeze. Implementation + vectors land in M2a-m0; this entry is direction, not resolution. |
| 2026-07-25 | **M1 exit — `v0.1.0-local` (experimental format):** SPEC §10 M1 checklist closed with per-box evidence. E1 (`e1_e2_acceptance` + `e1_kill_clock`: repeated hard-kill mid-write integrity, 1 h clock-rollback HLC monotonicity), E2 model-level (`e1_e2_acceptance` e2_*), E4 (`e4_crash_matrix`: 5 named failpoints × commit_local/atomic_group/import_bundle, rollback ≡ process death, recovery invariants), E9 (`e9_delete_machine`: H3 resolved as **derived visibility** — node/edge tombstones set-derived, no cascade ops, replay identity, query exclusion). R0.1 store safety (fail-closed init, set-derived tombstone) and DQ-7 HLC backend included. **No format freeze**: wire, bundle, SQLite layout, and sync protocol stay draft-1 unfrozen; freeze remains a separate Decision Log act. |
| 2026-07-25 | **M1 scope narrowed** (decision-queue item 3): canonical CBOR schema IR + `SchemaId`/`ep` wiring, secondary indexes, and the interactive `repl` move **out of M1** — CBOR IR + SchemaId to **M2-schema** (with the TS→IR pipeline), secondary indexes to **M3-era query work**, `repl` to **M2**. M1 ships the JSON schema pin + type-pin reject, strict/schemaless modes, and the O3 `query` CLI. Rationale: none of the three affect local durability, replay determinism, or the E1/E4/E9 acceptance surface; blocking exit on them rewards scope creep, not correctness. LEDGER rows track the moved work. |
| 2026-07-25 | **R0.2 wire stance:** the JSON `WireOp` / `ExportBundle` dual representation remains the **v2 experimental wire** (sync protocol v2, `zerodb_storage::sync`). The canonical-CBOR wire lands **with protocol v3** (server-push session) so there is exactly one breaking wire migration, not two. Until then all wire, bundle, and event shapes stay draft-1 unfrozen per the freeze policy. Deferred with triggers: M2-crdts (MVRegister/RGA/LWWMap) until a consuming app needs them; E4 WAL named-crash-point matrix to the formal M1 exit pass; CBOR wire implementation to v3. |
| 2026-07-19 | **Plan-doc consolidation (post-M0 / M1 prototype):** Removed completed P0/M0 tables and historical review archives from `plan/` (`FINDINGS.CODEX.md`, `DQ-PROPOSALS.md`; Grok findings reduced to open backlog). Promoted experimental M1 local-store + peer-exchange behavior to [M1-LOCAL.md](M1-LOCAL.md). Live delivery state: [PLAN.md](../plan/PLAN.md) + [LEDGER.md](../plan/LEDGER.md). Experimental multi-process TCP is **non-gating** for M1 exit. |
| 2026-07-18 | **Composite M0 exit** (C1–C5, C7–C8 approved resolutions; C6 deferred). Packages M0a–M0f closed at contract-model layer. Normative docs: KERNEL, SCHEMA, MERKLE, AUTH, WAL, DELIVERY, VERSIONS, FRONTIER. Conformance corpus **103** required vectors green in Rust + independent JS runners (CI-blocking). Cross-package smoke COMP-001. **Draft-1 profiles only — no wire/persistent format freeze** until an explicit freeze Decision Log names a versioned profile. SQLite layer-2 crash injection remains M1. On-wire enforcement M3.  |
| 2026-07-18 | **C8 + H4 + H7 + H11 contract resolved — M0e package exit:** [WAL.md](WAL.md) (C8, WAL-001..012), [DELIVERY.md](DELIVERY.md) (H4/H11, DELIV-001..004), [VERSIONS.md](VERSIONS.md) (H7 + H9 registry half; decode limits via registry + OP-NEG). DQ-4 contract layer closed. Layer-2 backend M1; wire M3. |
| 2026-07-18 | **C7 + O7 contract resolved — M0f package exit:** [FRONTIER.md](FRONTIER.md) — causal frontiers, peer acks/retirement, SnapshotId, late-op rule; FRONT-001..003. **GC remains disabled** until M5. Snapshot shipping M4. |
| 2026-07-18 | **C3 resolved — M0c package exit** (checklist complete): normative contract in [MERKLE.md](MERKLE.md) (1-minute buckets §3, leaf/node/empty domain-separated hashes, power-of-two pad, abstract mismatch-recovery walk §4 with Node/Leaf/Delta messages); **8 golden vectors CI-blocking** in two independent runners — `merkle-root` MERKLE-001..004 (empty/single/same-bucket+perm/two-buckets) + `merkle-transcript` MERKLE-T-001..004 (equal, missing-bucket pull, same-bucket delta, empty-equal). Wire CBOR framing of walk messages remains M3. **Draft-1 profile: no byte freeze before composite M0.**  |
| 2026-07-18 | **C4 admission + C5 resolved — M0d package exit** (checklist complete): normative contract in [AUTH.md](AUTH.md) (two-level identity + device certs §1, genesis/root authority §2, membership capabilities + admission token §3, per-op causal authz predicate §4, relay vs peer roles §5, shared quarantine §6); C4 **context half** already closed 2026-07-16 (KERNEL §4.1); **18 golden/negative auth vectors CI-blocking** in two independent runners across four families — `device-cert` (AUTH-CERT-001..005), `genesis-id` (AUTH-GEN-001..003), `authz-predicate` (AUTH-AUTHZ-001..006), `admission-token` (AUTH-ADM-001..004); full required corpus **75** vectors. DQ-1/DQ-2/DQ-3 contract layer resolved. **Draft-1 profile: no byte freeze before composite M0**; on-wire enforcement of admission + author resolution remains M3b.  Closes CX-02. |
| 2026-07-18 | **C2 resolved — M0b package exit** (checklist complete): normative contract in [SCHEMA.md](SCHEMA.md) (two-layer O2 model §1, schema IR §2 + structural validation outcomes, schema epochs §3, migration DSL + transform registry + segmented-replay model §4/§4.1, v0.1 query grammar + null/conflict/ORDER BY semantics §5); **57 golden/negative vectors CI-blocking in two independent runners** (Rust harnesses + JS models) across five families — `schema-ir` (+NEG), `epoch-replay` (EPOCH-001..014), `migration-transform` (MIG-001..005), `query-parse`, `query-eval`. O2/O3 already decided 2026-07-16; residual O3 prose+vectors completed. TS→IR compiler remains a standalone npm tool ≤ M1 (not an M0b gate). Cross-peer mixed-version migration shipping remains M4. **Draft-1 profile: no byte freeze before composite M0** — byte-affecting changes re-run this checklist.  |
| 2026-07-16 | **O2 decided — TypeScript authoring-canonical, IR identity-canonical:** TypeScript SDK definitions are the canonical *authoring* source; the deterministically compiled **schema IR** (canonical CBOR per KERNEL §3, stable IDs, epochs) is the sole replicated/hashed artifact and the only schema representation the core evaluates; the `.zerodb` DSL is **dropped** (generated read-only docs output at most); the M1 CLI consumes IR files; the TS→IR compiler ships as a standalone npm tool no later than M1. |
| 2026-07-16 | **O3 decided — v0.1 query subset is minimal:** MATCH / WHERE / RETURN / ORDER BY / LIMIT with deterministic null/conflict semantics (normative text in SCHEMA.md, M0b); aggregation, paths, and mutation-in-query deferred post-v0.1. |
| 2026-07-16 | **C1 resolved — M0a package exit** (checklist complete): normative contract in [KERNEL.md](KERNEL.md) (operation algebra §4, identifiers §2, deterministic CBOR §3, domain-separated preimages §4.4, HLC state machine §5, CRDT semantic kernel §6, encrypted-value envelope §7, BlobRef + limits §8); machine-readable registry `conformance/registry.json`; **24 golden/negative vectors CI-blocking in two independent runners** (Rust harnesses + JS models). C4 **context half** resolved with it (`ds`/`v`/`ep` in the signed preimage); C4 admission half remains open → M0d. **Draft-1 profile: no byte freeze before composite M0** — byte-affecting changes re-run this checklist.  |
| 2026-07-16 | **DQ-1..DQ-8 directions ratified** (proposal text retired 2026-07-19; contracts in AUTH/KERNEL/WAL): two-level identity (principal root + device certs, PeerId = BLAKE3(device pk)); self-certifying genesis `DatastoreId`; mandatory peer-side causal grant-time authorization (revocation defeats causally-later ops only); C8 closed in M0 via executable WAL reference model; property-level encryption with M0a-frozen envelope/AAD; payload caps + reserved `BlobRef` variant; HLC durability rides the atomic op commit (resume from oplog max); equal timestamps = cross-peer total order `(physical, logical, peer, op_id)` / same-device equivocation quarantine. *Later closed via package exit checklists 2026-07-16/18.* |
| 2026-07-16 | **O5 GunDB migration: won't do.** Clean break; no state-snapshot converter or migration tooling. "Successor to GunDB" refers to the developer experience, not data portability. |
| 2026-07-16 | **Delivery plan adopted:** `plan/PLAN.md` is the path-to-MVP delivery/tracking plan (P0 readiness package, revised M0 packages, M3a/b/c split, decision queue DQ-1..DQ-12). SPEC §10 remains the normative roadmap. |
| 2026-07-15 | **M0 package-split:** composite M0 delivered as **M0a–M0f** (op encoding; schema/query; Merkle SM; keys/membership; groups/delivery/versions; frontiers/snapshots). C6 excluded from composite M0 exit. Second M0 implementation = TS pure encoder under `conformance/`. Lean proofs do not gate M0. Pre-M0 implementation policy + approved-resolution checklist in SPEC §10. Release labels: `v0.1.0-local`=M1, `v0.1.0-sdk`=M2, `v0.1.0`=M3. |
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
