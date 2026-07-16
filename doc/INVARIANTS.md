# ZeroDB — Invariants

Falsifiable system invariants. Every conformance fixture, model test, and [Exemplar scenario](EXEMPLAR.md) cites the invariant IDs it exercises; a milestone gate that claims an invariant must point at a test whose failure would falsify it.

**Status:** v0.1 candidate list (PLAN P0-2). Statements are binding intent; the precise formal versions are produced by the M0 packages named in each entry. Lean proof obligations (M5+ assurance track) are drawn from this list.

Format — each invariant states: the property, **Falsified when** (the concrete observation that disproves it), the owning contract, and the first milestone gate that runs a test for it.

---

## Convergence & semantics

### I-1 — Strong eventual consistency (SEC)
Two peers that have applied the same set of operations (any arrival order, any duplication), under the same schema epochs, materialize byte-identical state.
**Falsified when:** any permutation or re-delivery of a fixture operation set yields differing materialized state on two model runs.
Contract: C1/C2 (M0a, M0b). First gated: M0a model vectors; end-to-end at M1, multi-peer at M3a.

### I-2 — CRDT merge algebra
State-based merge for every CRDT type is commutative, associative, and idempotent.
**Falsified when:** `merge(a,b) ≠ merge(b,a)`, `merge(merge(a,b),c) ≠ merge(a,merge(b,c))`, or `merge(a,a) ≠ a` for any reachable states.
Contract: M0a semantic kernel. First gated: M0a. *Note: this does **not** make operation application idempotent — that is I-3's job.*

### I-3 — Exactly-once application effect
Applying an operation more than once (same `(datastore, OpId)`) has no additional effect on materialized state, for every CRDT type including counters.
**Falsified when:** replaying any fixture op changes a materialized value (e.g. a counter increments twice).
Contract: M0a kernel + H4 anti-replay (M0e.2). First gated: M0a; after dedup-state loss at M3a.

### I-4 — HLC per-peer monotonicity (lifetime)
Timestamps issued by one peer strictly increase over the peer's entire lifetime — including process restart, wall-clock rollback, and restore from backup.
**Falsified when:** a peer issues an operation whose timestamp is ≤ any timestamp it previously persisted.
Contract: M0a kernel + CX-07 durable-state rule; backend at M1. First gated: M0a model; crash-injection at M1.

### I-5 — HLC causality on receive
After receiving a remote timestamp within the drift bound, the next local timestamp orders strictly above both the remote timestamp and all prior local timestamps.
**Falsified when:** `recv(r)` or a subsequent `now()` yields a timestamp ≤ `r` (including saturated-counter inputs).
Contract: M0a kernel. First gated: M0a (scaffold test exists: `recv_remote_logical_overflow_stays_above_remote`).

### I-6 — Deterministic content addressing
`OpId = BLAKE3(canonical preimage)`; the same logical operation encodes to the same bytes and the same `OpId` in every conforming implementation; `id` and `signature` are excluded from their own preimages; decode→re-encode is byte-identical.
**Falsified when:** the Rust and TypeScript encoders disagree on any golden vector, or a round-trip changes bytes.
Contract: C1 (M0a). First gated: M0a golden vectors.

### I-7 — Equal-timestamp determinism
Two distinct operations never leave state dependent on arrival order, even with equal HLC timestamps: the kernel either rejects equal-timestamp/different-content pairs from one peer as equivocation or applies a canonical total-order tie-break.
**Falsified when:** merging the same two ops in opposite orders yields different state (current scaffold LWW fails this — tracked A-03).
Contract: M0a kernel, DQ-8. First gated: M0a.

## Authenticity & authorization

### I-8 — Operation authenticity
Every synced operation verifies under its author's public key before it can materialize; tampering with any signed field invalidates it.
**Falsified when:** a forged or modified operation reaches materialized state on any peer.
Contract: C5 (M0d); enforcement M3b. First gated: M0d negative vectors; live at M3b.

### I-9 — Datastore boundary
An operation signed for datastore D is never accepted into D′ ≠ D, and only operations authored by a current-or-historically-authorized member of D materialize — verified **peer-side**, independent of any relay.
**Falsified when:** a cross-datastore replay or non-member authorship is accepted by a conforming peer.
Contract: C4 (M0a context + M0d). First gated: M0d negative vectors; live at M3b.

### I-10 — Encrypted-value confidentiality
No party outside the recipient set — including any relay — can recover the plaintext of an encrypted value from any protocol artifact (operations, sync messages, snapshots, logs).
**Falsified when:** a non-recipient reconstructs plaintext in the E2E test harness.
Contract: H10 envelope (frozen in M0a/M0b per CX-05). First gated: M3b.

## Sync & delivery

### I-11 — Merkle faithfulness
Equal oplogs produce equal Merkle roots; unequal roots are traversable to a delta that, once exchanged, makes the oplogs (and roots) equal. Traversal terminates.
**Falsified when:** two equal fixture oplogs hash to different roots, or a traversal transcript completes while an operation remains missing.
Contract: C3 (M0c). First gated: M0c transcripts; wire at M3a.

### I-12 — At-least-once delivery with resumable cursors
A resumed sync (after disconnect, crash, or partition) never permanently skips an operation the peer has not applied; late-arriving ops behind a cursor are still delivered.
**Falsified when:** any loss/reorder/partition schedule in the delivery model leaves a peer permanently missing an op it is entitled to.
Contract: H4/H11 (M0e.2). First gated: M0e.2 model; live at M3a.

### I-13 — Group atomicity
Operations in one group materialize all-or-nothing per the signed group manifest, on every peer and across crash/recovery.
**Falsified when:** a partial group is visible in materialized state at any observable point after recovery.
Contract: C8 (M0e.1 model); backend M1. First gated: M0e.1; crash-injection at M1.

## Durability & lifecycle

### I-14 — Commit durability and idempotent recovery
Once a local commit is acknowledged, the operation and its state effects survive a crash at any named crash point; recovery replays are idempotent (I-3 applies to recovery too).
**Falsified when:** crash injection at any boundary loses an acked op, duplicates its effect, or leaves oplog/state/HLC mutually inconsistent.
Contract: C8/M0e.1 model; SQLite layer 2. First gated: M0e.1 model; M1 backend.

### I-15 — No premature garbage collection
No operation is physically deleted unless a causal frontier covering it is durably acknowledged per the C7 contract. Until C7 resolves and its tests pass (M5b), the invariant degenerates to: **nothing is ever deleted**.
**Falsified when:** any operation is removed from any store pre-C7, or post-C7 removal precedes its frontier coverage.
Contract: C7 (M0f contracts). First gated: M0f model; enforcement audit continuously.

### I-16 — Deterministic delete visibility
Tombstone and referential-integrity semantics are a deterministic function of the operation set: same ops ⇒ same visible graph, including dangling-edge and late-arriving-edge cases; no peer generates divergent cascade operations from its local view.
**Falsified when:** two peers with equal op sets disagree on entity visibility, or a cascade emits ops that other peers do not deterministically reproduce.
Contract: H3 (M1 release-blocking). First gated: M1.

### I-17 — Schema-epoch determinism
Every data operation binds to an immutable schema epoch; replaying history across a migration yields the same state on every peer regardless of when it learned the migration.
**Falsified when:** an epoch-bound replay vector (including a CRDT type change) differs across model runs or peers.
Contract: C2 (M0b). First gated: M0b replay vectors; cross-peer at M4b.

---

*Additions require a falsification condition and an owning contract; anything that cannot name both is a goal, not an invariant.*
