> **Historical document (dispositioned).**  This review drove the P0 readiness package and the M0-package corrections, all now executed — its CX-/HX-/A-/D- findings map to completed or tracked work.  Live status is in [LEDGER.md](LEDGER.md); blocker rollup in [PLAN.md §2](PLAN.md).  Do not treat findings below as open work items.

# ZeroDB Specification & Plan Review — FINDINGS.CODEX

**Date:** 2026-07-16

**Reviewer:** Codex (OpenAI)

**Baseline:** `3aba801` (`docs: split M0 into M0a–M0f packages and record plan review`) plus the working-tree relocation of `FINDINGS.GROK.md` to `plan/FINDINGS.GROK.md`

**Scope:** Current specification, relay protocol, issues/decisions, roadmap, acceptance material, prior Grok review, and the experimental Rust scaffold.

**Review posture:** Findings only. This document does not amend the normative specification or close any tracked issue.

Line citations refer to that reviewed state. `CX-*`, `HX-*`, `A-*`, and `D-*` are review-local identifiers, not replacements for the C/H/O identifiers in `doc/ISSUES.md`.

---

## 1. Executive verdict

The project has made material progress since the 2026-07-13 Codex review and the 2026-07-15 Grok plan review. The revised plan now has a sensible sequencing thesis:

> executable contracts → local durability → one SDK → secure multi-peer sync → browser/evolution → production readiness → ecosystem

The M0a–M0f split, explicit release labels, pre-M0 implementation policy, resolution checklist, C6 deferral, and GC prohibition are all strong decisions worth preserving.

The project is nevertheless **not execution-ready**. More precisely:

- It is ready to continue **M0 contract design and experimental algorithm work**.
- It is not ready to freeze a wire or persistent format.
- It is not ready to begin M1 as a milestone with auditable exit criteria.
- It cannot yet claim that M3/v0.1 supplies offline catch-up through the relay profile the plan permits.
- It has no executable acceptance source, semantic conformance kernel, or green repository baseline.

The central gap is no longer merely “too many open questions.” The current plan can produce byte-compatible encoders while leaving state semantics incompatible, leaves it ambiguous whether M0 relies on work assigned to M1 while M1 is gated on M0, and can label an L1-only live-forwarding system as an offline-first product even though an offline peer has no guaranteed history source.

### Scorecard

| Dimension | Assessment | Why |
|-----------|------------|-----|
| Strategic sequencing | **Strong** | The overall M0→M6 order is sound. |
| Blocker disclosure | **Improved / mixed** | Most core blockers are disclosed, but stale security, scope, and roadmap claims remain. |
| Semantic completeness | **Poor** | CRDT/HLC transition semantics and datastore authority are not normative. |
| Security model | **Mixed** | Good C6/GC restraint; untrusted-relay implications are not carried through admission and signaling. |
| Acceptance falsifiability | **Poor** | Exemplar scenarios and invariant/conformance artifacts do not exist. |
| Delivery planning | **Poor** | No owners, estimates, status/evidence ledger, capacity model, or post-M0 critical path. |
| Repository baseline | **Red** | The workspace does not compile under `cargo test --workspace`. |

---

## 2. What improved and should be preserved

1. **Normative authority and draft status are explicit.** Core and relay authority are separated, open issues are named, and format freeze is prohibited before composite M0 (`doc/SPEC.md:3-7`).
2. **M0 is decomposed.** M0a–M0f now identify operation, schema, Merkle, identity, delivery, and frontier contract packages with a dependency sketch (`doc/SPEC.md:930-1019`; `doc/ISSUES.md:16-35`).
3. **Release labels are no longer ambiguous.** M1, M2, and M3 map to `v0.1.0-local`, `v0.1.0-sdk`, and `v0.1.0` (`doc/SPEC.md:891-904`).
4. **Experimental code is labeled honestly.** The plan distinguishes safe pre-M0 algorithm work from formats that must not freeze (`doc/SPEC.md:905-916`; `README.md:5-7`).
5. **Resolution requires evidence.** Normative prose, machine-readable artifacts, positive/negative vectors, and a Decision Log entry are now required (`doc/SPEC.md:918-928`).
6. **Entity ACLs are deferred.** The spec now admits that read filtering is not confidentiality and limits v0.1 to datastore-level control (`doc/SPEC.md:833-835`; `doc/ISSUES.md:61-63`).
7. **Unsafe GC is disabled.** Causal-frontier and peer-lifecycle work must precede compaction, and destructive tests are named (`doc/SPEC.md:733-744`; `doc/ISSUES.md:65-67`).
8. **Relay 0.2 is simpler.** CBOR-only framing and message consolidation removed substantial negotiation and conformance ambiguity (`doc/RELAY-SPEC.md:854-884`).

These improvements mean the plan does **not** need another wholesale rewrite. It needs a semantic foundation, internally satisfiable gates, and a delivery-control layer.

---

## 3. Critical findings

Critical here means the current text can permit divergent implementations, make a milestone logically impossible to close, or make the proposed v0.1 fail its defining offline/security outcome.

### CX-01 — M0 has no owner for executable CRDT and HLC semantics

`Operation.payload` is an undefined `CRDTPayload`; the CRDT table supplies one-line behavior descriptions; and the HLC section states properties without a normative local-event/receive/overflow state machine (`doc/SPEC.md:146-160`, `doc/SPEC.md:175-197`, `doc/SPEC.md:273-285`). The specification nonetheless promises that any peer with the same operation set computes identical state (`doc/SPEC.md:216-226`).

M0a–M0f cover bytes, schema epochs, Merkle traversal, keys, delivery, and frontiers, but no package owns the state-transition algebra for the five CRDTs M1 immediately implements (`doc/SPEC.md:949-1019`, `doc/SPEC.md:1027-1034`). `doc/INVARIANTS.md:1-7` remains a placeholder.

The formal-verification section also conflates state-based merge properties with arbitrary operation application: commutative/associative/idempotent state merge does not by itself make non-idempotent increment operations safe under replay (`doc/SPEC.md:783-816`). Delivery, deduplication, causal context, and each payload's effectors must be part of the semantic contract.

**Risk:** Rust and an independent implementation can produce identical bytes and Merkle roots, then materialize different values. Composite M0 could pass while strong eventual consistency (SEC) remains undefined.

**Required correction:** Add a semantic-kernel package or make it an explicit M0a sub-gate. It must define, for every M1 CRDT and HLC:

- state and operation algebra;
- preconditions and rejection rules;
- causal context/dot/tag representation;
- duplicate and out-of-order application behavior;
- equal-timestamp/equivocation behavior;
- local, receive, overflow, restart, and clock-rollback HLC transitions;
- positive, negative, permutation, replay, and boundary vectors run by independent model implementations.

### CX-02 — Datastore membership is not an end-to-end authorization model

The core calls relays untrusted (`doc/SPEC.md:435-445`), but v0.1 membership is described primarily as admission verified by a relay, and M0d's outcome is a capability format “relays can check” (`doc/SPEC.md:833-835`, `doc/SPEC.md:985-994`). Relay 0.2 proposes adding a credential to `SUBSCRIBE` (`doc/RELAY-SPEC.md:219-233`). `OPS`, sync, and peer-list messages name datastores independently, while `SIGNAL` has no datastore or shared-subscription binding at all (`doc/RELAY-SPEC.md:263-386`, `doc/RELAY-SPEC.md:464-475`). Operation validation does not normatively verify current author membership (`doc/RELAY-SPEC.md:551-573`).

The `Datastore` itself is not defined in the core specification. Relay terminology points to SPEC §4.4, but that section describes relay servers rather than datastore identity, genesis, ownership, schema binding, membership, or lifecycle (`doc/RELAY-SPEC.md:35-38`; `doc/SPEC.md:435-445`).

An honest relay is not clearly required to reject an unsubscribed `OPS` submission. A malicious relay can ignore admission, fabricate subscriptions, or forward an operation authored by a non-member. Replication boundaries enforced only by an untrusted relay are not a confidentiality or integrity boundary.

**Risk:** The v0.1 trust model can admit or disclose data contrary to its stated datastore boundary; migration, membership, key rotation, and checkpoint control operations have no root authority or role model.

**Required correction:** M0d must specify an end-to-end datastore control plane:

- canonical datastore genesis and `DatastoreId` derivation;
- stable principal/device/key model and bootstrap/root authority;
- capability issuer, subject, datastore, scopes, epoch, expiry, delegation, and revocation;
- authority for schema migration, membership, key rotation, and checkpoints;
- historical authorization rules for delayed operations;
- peer-side verification of author authorization, independent of relay behavior;
- confidentiality language that treats encryption—not relay routing—as the boundary against a malicious relay.

### CX-03 — Composite M0's red/green gate is not auditable as written

Approved resolution requires automated positive and negative vectors, and composite M0 requires red→green conformance tests for each package (`doc/SPEC.md:918-934`). M0e, however, exits with group/crash/dedup tests merely defined, with local crash behavior becoming green in M1 (`doc/SPEC.md:996-1007`). M1 then says it depends on composite M0 while implementing and gating the missing atomic storage behavior (`doc/SPEC.md:1021-1034`).

The plan does not explicitly distinguish a green executable contract model from a still-red production-backend test. Its “Depends on composite M0 (at minimum...)” wording is compatible with M0c/M0d/M0f remaining fixture-only, but it does not make that model-versus-backend distinction clear enough for an auditor (`doc/SPEC.md:1021-1024`).

**Risk:** One team can close C8/M0 on green model vectors while another reads the same text as requiring the SQLite crash tests assigned to M1. The gate permits opposite completion decisions and invites informal waivers.

**Required correction:** Separate two layers explicitly:

1. **M0 contract-model conformance:** an executable storage/WAL state-machine model, group manifest model, crash-point transcripts, and negative vectors are green.
2. **M1 backend conformance:** SQLite implementation tests inject crashes at every named boundary and become green.

Alternatively, allow M1 to start after named package entry gates and stop describing composite M0 as its prerequisite. Do not retain both policies.

### CX-04 — M3/v0.1 has no guaranteed offline catch-up path

M3 is the first multi-peer `v0.1.0` release and promises convergence through partition, reorder, retry, and rejoin over a WebSocket profile and a “minimal relay level” (`doc/SPEC.md:897-904`, `doc/SPEC.md:1046-1058`). H11 explicitly permits the M3 reference relay to be L1-only (`doc/ISSUES.md:107-108`).

An L1 relay stores no operations or Merkle state (`doc/RELAY-SPEC.md:73-84`) and must reject sync messages (`doc/RELAY-SPEC.md:257-261`). Only L2 supplies history and Merkle participation (`doc/RELAY-SPEC.md:85-93`). Direct WebRTC sync is deferred to M4 (`doc/SPEC.md:1060-1067`).

**Risk:** A disconnected peer misses live forwards and, on reconnect, has no protocol participant from which it can request history. That is live fan-out, not an offline-first database.

**Required correction:** Before `v0.1.0`, require one of:

- an L2 reference relay with durable persistence, receipt/durable acknowledgements, and full-oplog catch-up while GC remains disabled; or
- a shipped direct peer catch-up path over an authenticated transport available in M3.

If neither ships, rename M3 as an online-sync preview and do not use it as the first offline-first product release.

### CX-05 — M0 can authorize a format freeze before format-shaping decisions

M0a defines the versioned operation algebra, payload encoding, identifiers, and signed/hash preimages, and composite M0 is the earliest gate after which a format freeze is permitted (`doc/SPEC.md:930-961`). Yet the encrypted-property envelope, key IDs, nonce/AAD context, recipient/group key representation, and even whole-operation-versus-property encryption choice remain H8/H10 work scheduled for M3 (`doc/ISSUES.md:98-105`; `doc/SPEC.md:1053`). Large payload chunking versus blob references remains open until M4 (`doc/ISSUES.md:112-120`; `doc/SPEC.md:1068`).

These choices can add operation variants, schema annotations, signed context, wire fields, or content-addressed blob references after a freeze that the composite gate permits. Unique-index semantics similarly remain M5 work even though the schema IR already advertises `unique` (`doc/SPEC.md:755-779`, `doc/SPEC.md:1076-1078`).

**Risk:** The first post-M0 product milestones immediately require incompatible changes to the supposedly frozen v0.1 operation/schema format.

**Required correction:** Before using composite M0 to freeze a v0.1 profile:

- decide and encode the encryption envelope shape, AAD/context binding, and schema marker;
- define a versioned opaque-extension strategy and compatibility vectors;
- decide whether large values are always opaque bytes, chunk references, or a later format version;
- remove or reject schema features whose distributed semantics are deferred, including `unique`;
- freeze only a named profile, not an unlimited promise that later features cannot introduce a new format version.

### CX-06 — Milestone acceptance depends on artifacts that do not exist

The roadmap admits that Exemplar scenario IDs “still [need] to be written,” but M1 and M3 exits rely on the exemplar (`doc/SPEC.md:887-890`, `doc/SPEC.md:1034`, `doc/SPEC.md:1058`). `doc/EXEMPLAR.md:1-28` is a feature wishlist with no schema, actors, datastore topology, preconditions, fault schedule, operations, expected converged state, security negatives, or performance oracle.

Its individual-item/list sharing, administrative controls, and private data goals (`doc/EXEMPLAR.md:3-17`) are not mapped to a v0.1 limited to datastore-level membership or to the entity ACL work deferred to M6 (`doc/SPEC.md:891-895`, `doc/SPEC.md:1087-1096`). Datastore-per-list/item could satisfy part of the wishlist, but the acceptance model never says so.

The invariant source is a seven-line TODO (`doc/INVARIANTS.md:1-7`). No `conformance/` directory, fixture corpus, external integration tests, or CI configuration exists, although M0 exits depend on them.

**Risk:** Milestones can be declared complete without a shared definition of behavior, while the exemplar silently expands scope into deferred authorization features.

**Required correction:** Add a planning-readiness gate before package completion claims:

- invariant IDs with falsification conditions;
- versioned Given/When/Then Exemplar scenarios;
- explicit peer/datastore topology and fault schedules;
- exact expected materialized state and security outcomes;
- scenario-to-milestone and scenario-to-invariant traceability;
- conformance harnesses checked into CI, with newly activated failing fixtures isolated in an expected-failure lane until promoted to required-green at their gate.

Define sharing as datastore-per-list/item for v0.1 or explicitly defer individual-object sharing/admin behavior.

### CX-07 — HLC monotonicity has no durable lifetime contract

The spec promises strictly increasing timestamps for a peer even when the wall clock moves backward (`doc/SPEC.md:154-160`). Storage traits have no durable HLC state, startup recovery, lease, or “recover latest timestamp from oplog” contract (`doc/SPEC.md:674-721`). M1 names HLC and crash recovery but never makes clock-state persistence part of the atomic boundary (`doc/SPEC.md:1027-1034`).

The experimental implementation initializes each new clock at zero (`zerodb-core/src/hlc.rs:79-98`). After restart plus clock rollback, it can generate a timestamp below a previously persisted operation from the same peer. Key cloning across devices creates the same risk concurrently.

**Risk:** A post-restart write can lose LWW ordering, duplicate an earlier timestamp, or invalidate per-peer causal assumptions. Equal timestamps with unequal values are themselves non-convergent in the current LWW scaffold because merge retains whichever value was local (`zerodb-core/src/crdt/lww.rs:32-49`).

**Required correction:** The M0 semantic contract and M1 atomic boundary must define:

- durable last-HLC state or deterministic reconstruction;
- crash ordering between clock reservation and operation commit;
- clock rollback and restore behavior;
- cloned-key/device detection or a stable-device identity rule;
- equal-timestamp equivocation rejection or a canonical secondary tie-break over signed operation identity.

---

## 4. High-priority findings

### HX-01 — An untrusted relay can impersonate signaling peers

The relay writes `SIGNAL.sender` itself (`doc/RELAY-SPEC.md:364-386`); the challenge authenticates a peer only to that relay (`doc/RELAY-SPEC.md:416-452`). The security section then says an untrusted relay cannot impersonate a peer (`doc/RELAY-SPEC.md:641-654`). A malicious relay can fabricate signaling with any sender before an end-to-end peer handshake exists. `SIGNAL` also carries no datastore despite subscription being described as the signaling scope (`doc/RELAY-SPEC.md:219-221`).

Require a signed end-to-end peer transcript bound to datastore, transport, and negotiated protocol before accepting peer data, or narrow the security claim until H6 ships.

### HX-02 — Relay signature-validation MUSTs conflict while C5 is open

L1 claims signature validation (`doc/RELAY-SPEC.md:73-81`), and validation is a MUST for every operation (`doc/RELAY-SPEC.md:551-565`). The same rule says unresolved forwarded authors must not be rejected (`doc/RELAY-SPEC.md:557-559`). A relay must therefore forward an unverified operation or violate conformance.

The core already prohibits relay conformance before composite M0 (`doc/SPEC.md:445`). C5/M0d must replace the contradictory relay MUSTs with one deterministic unresolved-author outcome—reject, pending/quarantine, or authenticated-key resolution—before that prohibition is lifted.

### HX-03 — The protocol lacks resource-safe decoding limits

The CBOR envelope has no global frame size, nesting depth, collection count, string length, tag policy, or duplicate-key rule (`doc/RELAY-SPEC.md:97-123`). Announced limits cover operations/batches, while pre-auth messages, metadata maps, errors, and opaque signaling remain unbounded (`doc/RELAY-SPEC.md:177-188`, `doc/RELAY-SPEC.md:223-230`, `doc/RELAY-SPEC.md:364-384`, `doc/RELAY-SPEC.md:524-547`).

Define a restricted CBOR decoding profile and pre-allocation limits for every message, especially before authentication. This belongs in H9/M0e, not only O6's operation limits.

### HX-04 — HLC ranges are unsafe if reused as delivery cursors

The local API exposes `ops_since(HLCTimestamp)` as a local read primitive, and L2 storage requires HLC range queries for Merkle construction and delta serving (`doc/SPEC.md:680-687`; `doc/RELAY-SPEC.md:497-506`). Neither is currently defined as a delivery cursor. If M0e reuses either API for resume, an operation appended later with an older remote HLC lies behind the scalar cursor.

Use append sequence, authenticated causal frontier, or checkpoint/resume cursor semantics for delivery. Name/limit HLC-range APIs as diagnostic or bucket-index primitives unless their contract explicitly handles late inserts.

### HX-05 — Key rotation changes `PeerId` without defining identity continuity

`PeerId` is the full hash of an Ed25519 public key, while built-in key rotation replaces that key (`doc/SPEC.md:637-654`). The new key therefore has a new `PeerId`, changing membership subjects, causal per-peer dependencies, rate limits, and historical authorization.

M0d must decide whether identity is a key, device, or stable principal; how rotation rebinds memberships and history; and how recovery works when the old key is compromised rather than cooperative.

### HX-06 — Unique indexes are public before their distributed semantics

The schema advertises `unique: true` and maps it directly to backend indexes (`doc/SPEC.md:755-779`). M1 includes secondary indexes, while offline uniqueness conflicts remain unresolved until M5 (`doc/ISSUES.md:80-81`; `doc/SPEC.md:1027-1032`, `doc/SPEC.md:1076-1078`).

Reject or omit unique indexes through v0.1, or resolve H2 before networked release. A SQLite `UNIQUE` constraint is not a deterministic multi-writer conflict policy.

### HX-07 — “Two-language interoperability” has no independent peer

M0's second implementation is only a TypeScript encoder/decoder (`doc/SPEC.md:930-959`). M2's Node SDK is a binding to the same Rust core (`doc/SPEC.md:1036-1044`). M3 nonetheless requires two-language interoperability and a two-language wire harness (`doc/SPEC.md:1046-1058`).

Name an independent TypeScript protocol/model implementation and the state machines it implements. NAPI parity proves binding consistency, not independent interoperability.

### HX-08 — M3 and M5 remain big-bang milestones

M3 combines durable sync, Merkle traversal, delivery/resume, auth, membership, encryption lifecycle, clock abuse, peer handshake, relay implementation, and independent interoperability (`doc/SPEC.md:1046-1058`). M5 combines GC, uniqueness, Richtext, backup/restore, observability, fuzz/load work, broad Lean proofs, an external audit, and comparative benchmarks, with all of them gating GA (`doc/SPEC.md:1072-1085`).

Split M3 into durable convergence, security, and interop/release gates. Make M5 a focused, staged reliability/security program; track Richtext, broad proofs, and benchmarks as separate feature/assurance streams. Define audit closure by severity and accepted risk, not “all findings closed.”

### HX-09 — The roadmap lacks a delivery-control model

Milestones contain outcomes and checklists but no DRI, status, effort band, capacity assumption, target, evidence link, risk/mitigation, or fallback. Open questions are said to need an owner, but none has one (`doc/ISSUES.md:6-10`, `doc/ISSUES.md:112-120`). External audit is an unplanned third-party dependency (`doc/SPEC.md:1081-1085`).

Add a canonical delivery ledger containing: ID, DRI, status, dependencies, effort band, target, entry gate, exit evidence, risks, mitigation, and release. Derive the critical path from it rather than treating document order as a schedule.

### HX-10 — Resolution policy removes stable in-document traceability

Resolved issues are removed and replaced by a free-form date row (`doc/ISSUES.md:3-12`, `doc/ISSUES.md:124-141`; `doc/SPEC.md:918-928`). Git history preserves recoverability, but the roadmap and specification repeatedly cite C/H IDs that disappear from the current document when resolved.

Retain issue records with `status: resolved`, normative artifact links, fixture/test evidence, decision rationale, supersession data, and approver. If a compact open-only view is desired, generate it from the durable ledger.

### HX-11 — Version namespaces are ambiguous and risk coupling persistent identity to relay negotiation

M0a signs/hashes “protocol version” into persistent operations before M0e defines per-format authority (`doc/SPEC.md:949-956`, `doc/SPEC.md:996-1005`). Relay `protocol_version` is a connection negotiation value (`doc/RELAY-SPEC.md:135-144`, `doc/RELAY-SPEC.md:173-180`). Binding a relay protocol version into an `OpId` would change persistent identity merely because transport negotiation changes.

Define distinct `operation_format_version`, `schema_epoch`, `snapshot_format_version`, `storage_format_version`, and `relay_protocol_version` before M0a fixtures. Make the minimal version registry an M0a prerequisite, with the compatibility policy completed in M0e.

### HX-12 — Receiver limits refer to values peers never advertise

Only the relay sends `WELCOME.limits` (`doc/RELAY-SPEC.md:173-190`), but any delta sender must respect receiver limits “from WELCOME,” and relay fan-out must respect each receiving peer's limits (`doc/RELAY-SPEC.md:292-302`, `doc/RELAY-SPEC.md:491-493`). Peers never advertise receive capabilities.

Either define these as symmetric relay policy applied in both directions or add a peer receive-capability message/profile.

---

## 5. Repository alignment findings

The experimental-code warning is appropriate, so these are not evidence that a frozen implementation is wrong. They are evidence that the current scaffold is not yet a green reference and should not be used to close M0 artifacts.

### A-01 — The workspace does not compile under its test command

- `uuid` enables `v7` and `serde`, while `PeerId::random()` calls `Uuid::new_v4()` (`Cargo.toml:14-18`; `zerodb-core/src/types.rs:9-12`).
- Tests use `serde_json`, but it is absent from dependencies/dev-dependencies (`zerodb-core/Cargo.toml:7-11`; `zerodb-core/src/types.rs:89-94`).
- `cargo test --workspace` therefore compiles no test suite to completion.

### A-02 — `HLC::recv` mishandles remote logical-counter overflow

Remote counters use `saturating_add(1)`, but physical time advances only when the maximum physical time also equals the prior local physical time (`zerodb-core/src/hlc.rs:162-183`). If a newer remote timestamp has `logical == u16::MAX`, the returned timestamp can retain the same physical/logical pair and compare below the remote timestamp when the local `PeerId` sorts lower.

Existing tests cover ordinary receive and local `now()` overflow, not receive overflow (`zerodb-core/src/hlc.rs:245-305`). This contradicts the causal-ordering property in `doc/SPEC.md:154-160`.

### A-03 — Current LWW merge needs an equivocation rule

`set` and `merge` update only for a strictly greater timestamp (`zerodb-core/src/crdt/lww.rs:32-49`). Two different values with the same timestamp preserve whichever value was local, so merge order changes the result. The commutativity test avoids the case by using distinct peer IDs/timestamps; idempotency tests equal timestamp plus equal value (`zerodb-core/src/crdt/lww.rs:96-122`).

The semantic contract must either make equal-timestamp/different-value input impossible and reject it, or add a canonical signed-operation tie-break.

### A-04 — Experimental serialized shapes conflict with the intended profile

- `PeerId` is a random 16-byte UUID, versus the specified full 32-byte public-key hash (`zerodb-core/src/types.rs:4-12`; `doc/SPEC.md:637-645`).
- HLC serializes `physical_ms`/`logical`, while the relay transcript uses `physical_time`/`logical_counter` (`zerodb-core/src/hlc.rs:11-18`; `doc/RELAY-SPEC.md:795-806`).
- Storage is keyed globally by `OpId`, while relay persistence is per `(datastore, OpId)` (`zerodb-storage/src/lib.rs:17-25`; `doc/RELAY-SPEC.md:497-506`).

Keep these APIs explicitly non-conformant and non-persistent until generated or implemented from M0 artifacts.

### A-05 — Test infrastructure is incomplete and potentially flaky

The HLC tests share a process-global mutable mock clock, while Rust tests run concurrently by default (`zerodb-core/src/hlc.rs:194-207`, `zerodb-core/src/hlc.rs:228-305`). No test covers restart with wall-clock rollback. The only serialization test is JSON round-trip, not deterministic CBOR or negative decoding (`zerodb-core/src/types.rs:89-94`). `zerodb-storage` has no tests.

---

## 6. Cross-document corrections

These are smaller than the critical findings but materially increase contributor confusion.

| ID | Inconsistency | Evidence | Correction |
|----|---------------|----------|------------|
| D-01 | Core sync still names `LiveOp`; relay 0.2 replaced it with `OPS`. | `doc/SPEC.md:392-416`; `doc/RELAY-SPEC.md:315-339`, `doc/RELAY-SPEC.md:862-864` | Use one informative transcript derived from the protocol registry. |
| D-02 | Snapshot shipping is M4 in SPEC/ISSUES but M5 in RELAY-SPEC. | `doc/SPEC.md:1060-1070`; `doc/ISSUES.md:65-67`; `doc/RELAY-SPEC.md:514-518`, `doc/RELAY-SPEC.md:870-872` | Make M0f contract / M4 shipping authoritative everywhere. |
| D-03 | M3 asks for “resumption key proof,” while resumption was removed. | `doc/SPEC.md:1052`; `doc/ISSUES.md:89-90`, `doc/ISSUES.md:129`; `doc/RELAY-SPEC.md:192` | Replace with negotiated-transcript and transport binding. |
| D-04 | Core advertises TCP; relay 0.2 removed TCP/QUIC and no milestone owns a peer TCP profile. | `doc/SPEC.md:31-36`, `doc/SPEC.md:64-88`, `doc/SPEC.md:426-433`; `doc/RELAY-SPEC.md:854-863` | Remove TCP from current scope or add a post-v0.1 transport profile. |
| D-05 | Richtext retains obsolete “Phase 3” wording. | `doc/SPEC.md:273-285`, `doc/SPEC.md:305-308`, `doc/SPEC.md:1072-1083` | Mark it M5 feature-track/post-v0.1. |
| D-06 | O5 is simultaneously “won't do,” open, and an M6 decision. | `doc/ISSUES.md:112-120`; `doc/SPEC.md:1091-1096`, `doc/SPEC.md:1124-1132` | Record one Decision Log outcome and remove conflicting schedule text. |
| D-07 | C5 says validation uses the transport key; current relay text instead forwards unresolved authors. | `doc/ISSUES.md:57-59`; `doc/RELAY-SPEC.md:557-559` | Update the issue's current-state description without weakening C5. |
| D-08 | C8 cites a removed five-second relay group timeout. | `doc/ISSUES.md:69-71`; `doc/RELAY-SPEC.md:487-489`, `doc/RELAY-SPEC.md:870-872` | Remove the obsolete fact; keep the group-manifest/storage problem. |
| D-09 | Relay says censorship cannot be undetected, then admits one relay can censor. | `doc/SPEC.md:437-443`, `doc/SPEC.md:822-831`; `doc/RELAY-SPEC.md:641-663` | Condition detection on comparison with an independent source and a stable checkpoint. |
| D-10 | Relay says operation-level encryption while core specifies encrypted properties and H8 leaves scope open. | `doc/RELAY-SPEC.md:454-458`, `doc/RELAY-SPEC.md:650-653`; `doc/SPEC.md:647-654`; `doc/ISSUES.md:98-105` | Decide the profile before any post-M0 freeze and use one term. |
| D-11 | Release hygiene: workspace crates already report `0.1.0`, which can be mistaken for the M3 product label while the repo is pre-M0. | `Cargo.toml:8-12`; `README.md:5-7`; `README.md:19-23` | Consider `0.1.0-alpha`/`publish = false` until release policy exists; crate semver need not equal the product label. |
| D-12 | README's Grok link targets the removed root path. | `README.md:17`; current `plan/FINDINGS.GROK.md` | Point the link to `plan/FINDINGS.GROK.md`. |
| D-13 | `plan/FINDINGS.GROK.md` still presents several now-applied recommendations as open. | `plan/FINDINGS.GROK.md:24-39`, `plan/FINDINGS.GROK.md:382-400`, `plan/FINDINGS.GROK.md:462-472` | Mark it historical and add a disposition table. |
| D-14 | `_meta.signature?` on a materialized entity is unexplained. | `doc/SPEC.md:100-133`, `doc/SPEC.md:175-186` | Remove it or define what a single signature means for multi-author materialized state. |
| D-15 | L2 retention wording can be read as deletion after 30 days despite GC being disabled. | `doc/RELAY-SPEC.md:499-518` | State that the window is a minimum and deletion is forbidden until C7. |
| D-16 | The future ACL section says quarantine avoids permanent divergence, while C6 correctly says origin and receivers can materialize different accepted sets. | `doc/SPEC.md:879-883`; `doc/ISSUES.md:61-63` | Remove the convergence claim and keep the section explicitly non-normative until C6 is resolved. |

---

## 7. Prior Grok review disposition

The moved Grok review is useful history, but its headline verdict now overstates several items already repaired by `3aba801`.

| Grok finding | Current disposition | Evidence / remainder |
|--------------|---------------------|----------------------|
| C-P1: split M0 | **Substantially resolved** | M0a–M0f now exist (`doc/SPEC.md:930-1019`). Semantic-kernel ownership and green-gate ambiguity remain (CX-01, CX-03). |
| C-P2: pre-M0 code policy | **Substantially resolved** | Policy and experimental warning added (`doc/SPEC.md:905-916`). Scaffold is still red/non-conformant (A-01–A-04). |
| C-P3: map v0.1 | **Resolved in naming** | M1/M2/M3 labels are explicit (`doc/SPEC.md:897-904`). M3 cannot yet meet offline-first semantics (CX-04). |
| C-P4: executable Exemplar | **Open** | Scenario IDs are explicitly unwritten (CX-06). |
| C-P5: split M3 / thin M5 | **Open** | Both remain big-bang gates (HX-08). |
| C-P6: resolution process | **Partially resolved** | Artifact checklist and Decision Log exist; stable issue-ID/evidence/approver records do not (HX-09, HX-10). |
| C-P7: narrative hygiene | **Partially resolved** | C6 and competitive claims improved; TCP, Phase 3, resumption, web-of-trust, and protocol drift remain. |
| H-P1: schema/query timing | **Resolved in plan** | M0b freezes schema IR and minimal query grammar (`doc/SPEC.md:963-972`). |
| H-P2: delete semantics gate | **Substantially resolved** | H3 is explicitly M1-release-blocking (`doc/ISSUES.md:83-84`; `doc/SPEC.md:1029`). |
| H-P3: dependency graph | **Partially resolved** | M0 has a graph; post-M0 and format-shaping dependencies remain implicit. |
| H-P4: resources/critical path | **Open** | No delivery-control model (HX-09). |
| H-P5: second M0 language | **Resolved** | The requested TypeScript pure encoder/decoder is named (`doc/SPEC.md:930-932`). HX-07 is the broader, separate M3 independent-interoperability gap. |
| H-P6: invariants | **Open** | `doc/INVARIANTS.md` is still a TODO. |
| H-P7: relay level | **Open and now critical** | L1 cannot satisfy partition/rejoin; Grok's suggested L1 remedy is therefore superseded (CX-04). |
| H-P8: early code constants | **Mitigated, not corrected** | Experimental policy is clear; serialized shapes and build remain divergent/red. |

One prior point should be narrowed: optional `Node`/`Edge` `_meta.signature` does not directly make operation signatures optional because materialized entities and operations are distinct objects. The field is still undefined and likely misleading, but the stronger contradiction claim is not warranted.

---

## 8. Recommended plan revision

This preserves the adopted M0–M6 strategy while making it executable.

### P0 — Planning and conformance readiness

**Outcome:** contributors can identify the contract owner, failing test, and evidence needed for every planned gate.

- Populate invariant IDs and falsification rules.
- Rewrite Exemplar as versioned scenarios mapped to invariants and milestones.
- Define the v0.1 runtime/support profile and explicitly prohibit deferred schema/API features.
- Add the delivery ledger: DRI, status, effort, dependencies, evidence, risk, target, release.
- Create `conformance/` with Rust and independent TypeScript model runners. Keep default CI green; demonstrate each newly activated contract fixture failing in a non-blocking expected-failure lane, then promote it to required-green at the package gate.
- Resolve the M0-model versus M1-backend gate language.

### M0 — Executable contracts, revised

| Package | Primary issue sources | Required outcome before exit |
|---------|-----------------------|------------------------------|
| **M0a — Semantic and operation kernel** | C1; H1 semantic contract; H7 registry prerequisite | HLC + M1 CRDT state machines; identifiers; operation algebra; distinct version namespaces; deterministic encoding/preimages; encrypted-value envelope/AAD/key-reference bytes; large-value extension framing; equal-timestamp and replay rules. |
| **M0b — Schema/query profile** | C2, O2, O3; H2 exclusion; H10 schema contract | Canonical IR/epochs/migrations; minimal query semantics; encryption annotations; unsupported `unique` rejected; extension/version rules. |
| **M0c — Merkle/sync model** | C3 | Canonical tree, adversarial timestamps, traversal state machine, concurrent-write/retry transcripts, roots/checkpoint hooks. |
| **M0d — Datastore/identity/authorization** | C4, C5; H6 handshake contract; H10 key/control contract | Genesis, stable principal/device model, scoped membership, peer-side verification, rotation/revocation/history, key-reference meaning, signed peer handshake. |
| **M0e — Delivery/durability/version/limits** | C8, H4, H5, H7, H9, H11, O6 | Independently green sub-gates: **M0e.1** group/WAL reference model; **M0e.2** delivery/ack/resume state machine; **M0e.3** resource-safe CBOR and compatibility registry. |
| **M0f — Frontiers/checkpoints/snapshots** | C7, O7 | Executable retirement/reconnect/late-op/checkpoint/root-comparison models; no physical GC. |

**Composite gate:** all package model suites green in Rust and an independent TypeScript semantic/model runner; cross-package fixtures cover signed encrypted operations through datastore authorization, grouping, Merkle sync, checkpoint identity, and replay. Production backends are not required yet.

**Contract dependency sketch:** the version-registry skeleton precedes M0a fixtures; M0a feeds M0b/M0c/M0d/M0e; M0b plus M0d close encrypted-value semantics; M0c plus M0e.2 feed M0f. All six package gates feed the composite gate.

```text
P0 → version registry → M0a → { M0b, M0c, M0d, M0e }
M0b + M0d → encrypted-operation integration fixtures
M0c + M0e.2 → M0f
M0a–M0f composite → M1 → M2 → M3a → M3b → M3c (v0.1.0)
                                      │       │
                                      └→ M4a  └→ M4b → M5 GA program
```

### M1 — Local durable alpha

- Rust + SQLite + CLI subset.
- Atomic oplog/state/HLC transaction and crash injection.
- Deterministic replay/delete semantics.
- Only M1-profile schema and query features; reject `unique`, Richtext, ACLs, and sync-only controls.
- Exit on green storage contracts and Exemplar local scenarios.

### M2 — SDK alpha

- Node/NAPI product API and lifecycle tests.
- Subscription/backpressure/error contract.
- Keep the independent TypeScript semantic/model runner separate from the Rust-backed SDK; evolve that runner into the independent M3c wire peer rather than counting NAPI twice.

### M3a — Durable convergence (internal gate; not a release)

- L2 reference relay without GC.
- Merkle/delta, append/resume cursor, receipt/durable ack, loss/reorder/partition/rejoin.
- Three-peer offline catch-up and crash/restart acceptance.
- Use pre-provisioned signed test identities; do not distribute a product or weaken the mandatory-signature rule before M3b authorization enforcement.

### M3b — Security (internal gate; not a release)

- Author keys, peer-verifiable datastore authorization, signed peer handshake.
- Implement the M0-frozen encryption envelope and full key lifecycle/control-plane semantics.
- Clock quarantine, resource limits, malicious relay/peer negatives.

### M3c — Interoperability and v0.1 release

- Independent TypeScript wire peer evolved from the M0 semantic/model runner.
- Version/upgrade compatibility matrix.
- Packaging, support profile, recovery documentation, and complete Exemplar sharing/private scenarios.
- `v0.1.0` exits here.

### M4 — Two separately gated tracks

- **M4a platform:** browser storage, WASM, WebRTC, React integration.
- **M4b evolution:** schema migration, snapshots, adjacent-version rollback/upgrade, and large-payload storage/transfer implementation under the M0-approved extension/profile decision.

### M5 — Focused GA program

- **M5a operability:** backup/restore drills, observability/SLOs, and operational packaging.
- **M5b lifecycle safety:** safe GC/peer lifecycle and rolling upgrades.
- **M5c release assurance:** fuzz/soak/failure injection and severity-based security sign-off.
- The completed program is the candidate for a GA/`1.0` label; the delivery ledger must make that release decision explicit.

### Parallel post-v0.1 tracks

- **Unique-index semantics:** coordination/conflict design as an independent feature epic, or a durable decision that ZeroDB will not support distributed uniqueness.
- **Richtext:** feature release after the M0 large-value profile and M4 evolution work.
- **Formal assurance:** Lean proofs tied to the M0 semantic kernel; gate only releases that explicitly claim the proved property.
- **Comparative performance:** published benchmark track with versioned workloads, not a correctness gate.

### M6 — Portfolio, not a serial milestone

Treat mobile ABI, plugins, entity ACL research, hosted relay, admin UI, and migration tooling as independently approved epics after compatibility stability.

---

## 9. Immediate decision queue

### M0 contract and post-M0 freeze decisions

Resolve each before closing the package it shapes or authorizing a dependent format freeze:

1. Is identity a key, a device, or a stable principal containing devices?
2. How is a datastore created, and who has root authority for members, schemas, keys, and checkpoints?
3. Must peers verify author membership per operation, and what historical authorization rule applies after revocation?
4. What executable model closes C8 in M0 without requiring the SQLite backend from M1?
5. Is v0.1 encryption property-level or operation-level, and which envelope/context bytes freeze in M0?
6. What extension/blob decision prevents O1 from invalidating M0a?
7. How is HLC state made durable across restart, restore, clock rollback, and cloned keys?
8. Are equal HLC timestamps with unequal signed operations rejected as equivocation or deterministically ordered?

### Execution and release governance

Resolve these before executing the dependent release work; they need not block every independent M0 package:

9. Is L2 durable catch-up mandatory for M3/v0.1? If not, what shipped peer path replaces it?
10. Are unique indexes removed from the v0.1 schema profile or promoted into a later feature gate?
11. Who approves a normative resolution, and where are stable resolved issue IDs, evidence, and approver records retained?
12. What team/capacity assumption, DRI set, and effort band make the critical path credible?

---

## 10. Verification performed

### Repository and document review

- Reviewed `README.md`, every file under `doc/`, `plan/FINDINGS.GROK.md`, all Cargo manifests, and all Rust source files.
- Compared current text against the earlier Codex and Grok review outcomes.
- Performed independent spec, plan, and implementation-alignment passes.

### Commands

| Command/check | Result |
|---------------|--------|
| `cargo metadata --no-deps --format-version 1` | Passed; two library crates discovered. |
| `cargo test --workspace` | **Failed during compilation; zero tests completed.** Missing UUID v4 feature and `serde_json` test dependency. |
| `cargo fmt --all -- --check` | **Failed.** Formatting differences in `zerodb-core/src/hlc.rs` and import ordering in `zerodb-core/src/lib.rs`. |
| Local Markdown target scan | Existing broken target at `README.md:17` (`FINDINGS.GROK.md` moved under `plan/`). |
| Repository inventory | No CI, `conformance/`, fixture corpus, SQLite backend, CLI, operation algebra, Merkle implementation, crypto implementation, or relay implementation. |

The build/format failures predate this findings-only document. No implementation fix was attempted.

---

## 11. Conclusion

The current roadmap has the right spine. Its next correction should be smaller and more concrete than another architecture rewrite:

1. add the missing semantic and datastore authority contracts;
2. make M0's model gate compatible with M1's implementation gate;
3. require a real offline catch-up source before calling M3/v0.1 offline-first;
4. freeze only a versioned profile whose encryption and extension shapes are already known;
5. turn invariants and the Exemplar into executable acceptance material;
6. add owners, evidence, effort, and risk to the roadmap;
7. restore a green experimental baseline before using the scaffold as conformance evidence.

Until those changes are made, treat ZeroDB as **ready for M0 specification work, not ready for format freeze or product implementation**.
