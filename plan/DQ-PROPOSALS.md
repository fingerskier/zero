# DQ-1..DQ-8 — Design Proposals

**Status:** design proposals. Nothing here becomes normative except through the [SPEC §10](../doc/SPEC.md) approved-resolution checklist (normative prose + machine-readable artifact + golden vectors + Decision Log entry).

- **DQ-1..DQ-3** — **resolved 2026-07-18** via M0d exit; contracts live in [AUTH.md](../doc/AUTH.md). Text below is historical design rationale.
- **DQ-4** (below) — **direction ratified 2026-07-16**, awaiting contract prose in M0e.1.
- **DQ-5..DQ-8** — **resolved**; the contracts now live in [KERNEL.md](../doc/KERNEL.md). Only compact resolution stubs remain here (§DQ-5..§DQ-8); KERNEL is authoritative.

Each live proposal states: recommendation, rationale, rejected alternatives, and what freezes where. Decision IDs from [PLAN.md §6](PLAN.md); tracking in [LEDGER.md](LEDGER.md).

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

## DQ-5..DQ-8 — resolved (contracts in KERNEL.md)

These four resolved 2026-07-16 inside the C1/M0a closure. Their full proposal text is retired; the authoritative contract and golden vectors live in [KERNEL.md](../doc/KERNEL.md). Summary of what was ratified:

| DQ | Decision | Now normative in | Vectors |
|----|----------|------------------|---------|
| DQ-5 | **Property-level encryption.** Envelope `version(u8) ‖ key_id(16B) ‖ nonce(24B) ‖ ct+tag` (XChaCha20-Poly1305), AAD binds datastore/op/epoch/property path. Encrypted values legal for LWW/MVRegister only. *Rejected:* whole-operation encryption; deferring bytes past M0. | KERNEL §7 | ENV-001/002 + AAD negatives |
| DQ-6 | **Hard payload caps + reserved `BlobRef`.** Caps ≤64 KiB op / ≤256 KiB / ≤512 ops per batch (registry constants). `BlobRef = blake3(32B) ‖ total_size(u64) ‖ codec(u16)` byte-frozen; v0.1 parses and rejects with `BLOB_UNSUPPORTED`; transfer protocol is M4. *Rejected:* oplog chunking; unbounded inline payloads. | KERNEL §8 | CRDT-BLOB-001 |
| DQ-7 | **HLC durability rides the atomic op commit.** Recovery = `resume(max own-device timestamp in oplog)`; no separate clock file is load-bearing. Restore/clone divergence surfaces as DQ-8 equivocation ⇒ re-cert. Backend layer re-verified at M1. *Rejected:* fsynced clock file as source of truth; per-boot lease counters. | KERNEL §5 | HLC-002/005 |
| DQ-8 | **Scope-split equal timestamps.** Cross-peer ties break by total order `(physical_ms, logical, peer, op_id)` (fixes A-03); same-device equal-ts distinct ops are equivocation → both ops excluded + device quarantine. *Rejected:* arrival-order tie-break; global equal-ts rejection; tie-breaking same-device forks. | KERNEL §4.5 | CRDT-LWW-001/002 |

---

## Cross-cutting notes

- **Shared quarantine mechanism:** DQ-3 (unresolved author/grant), DQ-7 (restore/clone), DQ-8 (equivocation), and H1 (future clocks, M3b) all use one bounded, application-visible quarantine buffer — specify it once in M0d/M0e.
- **Verification-cost budget:** DQ-1+DQ-3 add cert-chain + grant-chain checks per op; chains are tiny and cacheable per (device, grant) pair — record a fixture-level budget in E11 when M0d lands.
- **Ratification path:** accept/amend per DQ → LEDGER status `in-progress` → contract prose + vectors per owning package → Decision Log + LEDGER resolved record.
