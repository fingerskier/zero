# DQ-1..DQ-8 — Design Proposals

**Status:** proposals awaiting ratification. Nothing here is normative: each becomes a resolution only through the [SPEC §10](../doc/SPEC.md) approved-resolution checklist (normative prose + machine-readable artifact + golden vectors + Decision Log entry). Ratifying a recommendation here means "start writing that contract," not "the contract exists."

Each proposal states: recommendation, rationale, rejected alternatives, and what freezes where. Decision IDs from [PLAN.md §6](PLAN.md); tracking in [LEDGER.md](LEDGER.md).

---

## DQ-1 — Identity: key, device, or principal?

**Recommendation: two-level identity — a *principal* root key with delegated *device* keys.**

- A **Principal** is a self-certifying Ed25519 root keypair; `PrincipalId = BLAKE3(root public key)`. The root key is cold: it signs only control artifacts, never data operations.
- A **Device** is an Ed25519 keypair authorized by a signed **device certificate** from the principal root (fields: device pubkey, principal id, issued-at, optional expiry, revocation reference). `PeerId = BLAKE3(device public key)` — unchanged from SPEC §6.1.
- **Operations are authored by device keys**; verification resolves the device cert chain to a principal. **Membership subjects are principals**, so rotating or adding a device never churns datastore membership (fixes HX-05).
- **Device rotation:** new cert from the root; old device cert revoked by a signed revocation record. **Root compromise/loss:** no in-protocol recovery in v0.1 — re-admission of a new principal by the datastore owner (out-of-band recovery is a documented limitation).

**Rationale:** device-key-only identity makes key rotation destroy membership, causal per-peer state, and history attribution (HX-05). Full multi-device principal sync (cross-signing, social recovery) is a product in itself. The two-level split gets stable subjects with one extra signature verification, and every later recovery scheme slots under the root without wire changes.

**Rejected:** *bare device keys* (rotation churn, HX-05); *stable DeviceId decoupled from keys* (introduces a mutable binding that itself needs a root of trust — same mechanism, worse naming); *full principal sync in v0.1* (scope).

**Freezes:** cert format, `PrincipalId`/`PeerId` derivations, chain-verification rule → M0d contract, encoded per M0a. Solo-device users generate both keys transparently; SDK hides the split.

---

## DQ-2 — Datastore genesis and root authority

**Recommendation: self-certifying genesis operation; owner-rooted capability delegation.**

- A datastore is created by a signed **genesis operation** authored by the founding principal: fields include founder `PrincipalId`, created-at, initial schema epoch reference (or "schemaless"), format/protocol versions, and a random salt.
- **`DatastoreId = BLAKE3(genesis preimage)`** — the id certifies its own origin; nobody can claim a datastore they didn't create, and guessed ids are unforgeable.
- The **founder is root authority**. Every control-plane operation — membership grant/revoke, schema epoch/migration, key distribution/rotation records, checkpoint approval — must chain by signature to the genesis authority: either signed by the founder or by a principal holding an explicit **admin capability** delegated (transitively, if the grant says so) from the founder.
- Ownership transfer = a signed transfer record; v0.1 has exactly one root at a time (no k-of-n multi-owner — deferred).

**Rationale:** resolves CX-02's "no root authority" gap with the simplest sound construction; self-certifying ids kill the datastore-squatting/guessing class of issues; capability delegation reuses the M0d capability format instead of inventing a role system.

**Rejected:** *server/relay-assigned ids* (relays are untrusted); *multi-sig root in v0.1* (scope; the transfer-record slot allows it later); *"whoever knows the id may join"* (that's the current broken state).

**Freezes:** genesis preimage layout and `DatastoreId` derivation → M0a bytes; authority-chain verification rule → M0d.

---

## DQ-3 — Per-operation membership verification & historical authorization

**Recommendation: mandatory peer-side verification; causal grant-time validity; revocation defeats causally-later ops only.**

- **Every peer** (not just relays) verifies, for every operation it applies: valid device→principal chain (DQ-1) **and** that the authoring principal held write membership in the op's causal past — i.e. the op's `deps` include (transitively) the grant under which it writes, and the op is **not causally after** a revocation of that grant.
- **Concurrent-with-revocation ops are accepted.** Deterministic: causal order is explicit in `deps`, so every peer computes the same answer regardless of arrival order (preserves I-1). A revoked-but-concurrent writer gets at most one final concurrent window, which the revoker's own client can display.
- Relay checks (SUBSCRIBE credential, C4 admission) remain a **bandwidth/DoS filter only** — never load-bearing for integrity or confidentiality.
- Ops failing verification are **rejected with a named outcome** (this also supplies the C5/HX-02 unresolved-author rule: an op whose author chain cannot be resolved is *pending* in a bounded quarantine buffer awaiting key/grant records, then rejected on timeout/overflow — never silently forwarded-and-materialized).

**Rationale:** CX-02's core demand — the security boundary must hold against a malicious relay, so peers must verify. Grant-time causal validity is the only order-independent rule; wall-clock-based validity would reintroduce clock attacks (H1) into authorization.

**Rejected:** *relay-enforced membership* (untrusted enforcer); *revoke-wins-over-concurrent* (non-monotone: an op flips valid→invalid when the revocation arrives, breaking SEC unless full reevaluation machinery exists — that's C6's unsolved problem, deliberately out of v0.1); *no historical rule* (delayed ops undecidable).

**Freezes:** validity predicate, quarantine bounds, rejection outcomes → M0d contract + negative vectors (E5, E7).

---

## DQ-4 — Closing C8 in M0 without the SQLite backend

**Recommendation: an executable WAL/commit reference model with enumerated crash points, run in both conformance runners.**

- M0e.1 defines abstract storage as a pure state machine: primitives `wal_append`, `wal_sync`, `state_apply`, `hlc_persist`, `group_seal(manifest)`, `wal_truncate`; a **named crash point between every adjacent pair**; and a `recover()` function.
- Conformance vectors are **crash transcripts**: an operation/group schedule × a crash point → required post-recovery observable state. The model must show: acked commits durable (I-14), groups all-or-nothing per signed manifest (I-13), recovery idempotent (I-3), oplog/state/HLC mutually consistent (DQ-7 rides the same boundary).
- Implemented as plain in-memory code in Rust and the JS runner — no I/O, no SQLite. **Layer 2 (M1)** then maps each named crash point onto real SQLite transaction boundaries and re-runs the same transcripts with actual crash injection.

**Rationale:** exactly the two-layer gate rule (P0-6); the model is the contract, the backend is an implementation of it. This is the standard formal-model/refinement move and it keeps M0 free of backend engineering.

**Rejected:** *"defer C8 tests to M1"* (recreates the CX-03 circularity); *mandating SQLite semantics in the spec* (binds the contract to one engine; IDB/OPFS backends need the same model in M4).

**Freezes:** primitive set, crash-point names, manifest fields → M0e.1; transcripts land red in `conformance/vectors/xfail` first.

---

## DQ-5 — Encryption: property-level or operation-level; what freezes in M0

**Recommendation: property-level encryption; freeze the envelope bytes in M0a; key lifecycle in M0d/M3b.**

- **Unit = property value.** Schema (M0b) annotates properties as encrypted; an encrypted value is an opaque **envelope** wherever a plaintext value would appear.
- **Envelope (frozen in M0a):** `version (u8) ‖ key_id (16B = BLAKE3-16 of the group key) ‖ nonce (24B random) ‖ ciphertext+tag`. Cipher: XChaCha20-Poly1305 (already the decided suite).
- **AAD binds context:** domain-separation tag ‖ `datastore_id` ‖ operation signed-context hash ‖ schema epoch ‖ property path — an envelope cannot be replayed into a different op, property, datastore, or epoch.
- **Group keys** per recipient set: distributed via X25519 sealed boxes inside signed key-distribution control ops (M0d format; full rotation/revocation/PCS semantics are M3b behavior over M0-frozen bytes).
- CRDT constraint (honest limitation): encrypted values merge as opaque registers — **LWW/MVRegister only** in v0.1; encrypted counters/sets are rejected by schema validation (M0b).

**Rationale:** property-level matches the column-CRDT architecture — mixed public/private properties on one entity, relay/schema-blind structure validation intact, sync and Merkle untouched. It is also what SPEC §6.2 already promises (D-10 chose the term). Metadata (which entity/property changed, sizes, timing) stays visible — recorded as an explicit limitation; operation-level wrapping can arrive later as a *new operation variant* without breaking v0.1 envelopes.

**Rejected:** *whole-operation encryption* (kills mixed visibility and column-CRDT merging of private fields — everything private collapses to one blob register; better metadata privacy can be layered later); *deferring envelope bytes past M0* (CX-05: the freeze would be invalidated at M3).

**Freezes:** envelope layout + AAD recipe → M0a golden vectors (incl. negative: wrong AAD context must fail); schema annotation → M0b; key-record format → M0d.

---

## DQ-6 — Extension/blob strategy so O1 cannot invalidate M0a

**Recommendation: hard payload caps now; a reserved content-addressed blob-reference variant, encoded in M0a, implemented in M4.**

- **O6 provisional caps (M0a):** ≤ 64 KiB encoded operation, ≤ 256 KiB / ≤ 512 ops per batch (numbers ratified with the O6 fixture set; caps are protocol constants in the version registry, raisable by a new `operation_format_version`).
- **Value variant `BlobRef` reserved and byte-frozen in M0a:** `blake3_hash (32B) ‖ total_size (u64) ‖ codec (u16 registry)`. It is a first-class value type in the operation algebra from day one; v0.1 conforming implementations **parse it and reject it** with a named "blob transfer unsupported" outcome.
- Blob **transfer/storage protocol** (chunking, relay obligations, GC interaction) is designed in M4 (O1) — it changes no operation bytes because the reference encoding already exists.
- Richtext and RGA payload pressure route through `BlobRef` or stay under the cap; no third path.

**Rationale:** the only thing M0a must guarantee is that large-value support won't force new preimage rules later. Reserving the variant costs ~40 bytes of spec and removes CX-05's blob risk entirely. Caps make H9/HX-03 resource limits enforceable pre-auth.

**Rejected:** *chunking ops across the oplog* (multiplies group/causality complexity, pollutes Merkle buckets); *unbounded inline payloads* (DoS; kills O4/WASM memory budgets); *deciding nothing* (the CX-05 freeze hazard).

**Freezes:** caps + `BlobRef` encoding → M0a; codec registry → M0e.3.

---

## DQ-7 — Durable HLC state across restart, restore, rollback, cloned keys

**Recommendation: HLC state rides the atomic commit; recovery = resume from the oplog maximum; clones are handled by DQ-8 equivocation detection.**

- **Rule:** a device's last-issued HLC timestamp is durable **in the same atomic boundary as the operation that carries it** (it is already a signed field of the op — the DQ-4 model makes `hlc_persist` part of `wal_append`'s crash-safety obligation, so no separate clock file is load-bearing).
- **Recovery:** on start, `HLC::resume(max timestamp over own-device ops in the local oplog)` (implemented in `a4266fa`); an optional clock-state record is a cache only. Wall-clock rollback is absorbed by `resume` (physical = max(wall, last)). Guarantees I-4 with zero extra fsyncs.
- **Restore from backup:** same rule restores monotonicity relative to everything the backup contains. Ops issued after the backup by the pre-restore instance make the restored instance a **potential equivocator** — that is detected and quarantined per DQ-8, and the documented recovery is: restore ⇒ issue a fresh device cert (DQ-1) rather than resurrect the old device key.
- **Cloned device keys** (same key live twice) are indistinguishable from restore-and-continue and get the same treatment: first equivocation observed ⇒ device quarantined pending re-certification.

**Rationale:** ties CX-07 to the C8 boundary instead of inventing a second durability mechanism; the oplog is already the source of truth. The clone/restore edge is unsolvable by clock rules alone — it is an *identity* event, so it routes to DQ-1/DQ-8 machinery.

**Rejected:** *separate fsynced clock file as source of truth* (second durability boundary, new crash-consistency cases, still defeated by backup restore); *monotonic lease/epoch counters per boot* (helps, but still requires equivocation handling — add later if fixtures show scan-cost problems).

**Freezes:** recovery rule + crash ordering → M0a kernel + M0e.1 crash transcripts; restore/re-cert procedure → M0d prose.

---

## DQ-8 — Equal HLC timestamps: equivocation or tie-break?

**Recommendation: both, by scope — cross-peer ties break deterministically; same-device duplicates are equivocation.**

- **Different devices:** equal `(physical_ms, logical)` is legal concurrency. Total order = `(physical_ms, logical, peer_id, op_id)` — the existing `peer_id` tiebreak, with content-addressed `op_id` last (relevant only in exotic multi-op-per-tick cases). LWW and every deterministic choice in the kernel MUST use this full total order (fixes A-03: two different values can never tie).
- **Same device:** a correct HLC never issues the same `(physical_ms, logical)` twice (I-4), so two *distinct* signed operations from one `PeerId` with equal timestamps are **equivocation**: both ops and the device enter quarantine (bounded store, surfaced to the application; device exits quarantine only via re-certification per DQ-1/DQ-7). Byte-identical duplicates are ordinary dedup (I-3), not equivocation.
- Deterministic outcome either way ⇒ I-1 and I-7 hold; the fork is *detected* rather than silently absorbed, which restore/clone scenarios (DQ-7) require.

**Rationale:** pure tie-break-everything hides device forks (a cloned key silently interleaves forever); pure reject-everything turns legal cross-peer concurrency into a fault. Splitting by scope gives convergence *and* fork detection at the cost of one quarantine mechanism M0d already needs (DQ-3's pending buffer reuses it).

**Rejected:** *local-wins/arrival-order anything* (the A-03 bug, order-dependent); *global equal-ts rejection* (breaks legitimate concurrency); *tie-break same-device forks* (accepts equivocation as normal, poisoning per-peer causal assumptions).

**Freezes:** total-order definition + equivocation predicate + quarantine outcomes → M0a kernel vectors (E2's equal-timestamp case, permutation suites).

---

## Cross-cutting notes

- **Shared quarantine mechanism:** DQ-3 (unresolved author/grant), DQ-7 (restore/clone), DQ-8 (equivocation), and H1 (future clocks, M3b) all use one bounded, application-visible quarantine buffer — specify it once in M0d/M0e.
- **Verification-cost budget:** DQ-1+DQ-3 add cert-chain + grant-chain checks per op; chains are tiny and cacheable per (device, grant) pair — record a fixture-level budget in E11 when M0d lands.
- **Ratification path:** accept/amend per DQ → LEDGER status `in-progress` → contract prose + vectors per owning package → Decision Log + LEDGER resolved record.
