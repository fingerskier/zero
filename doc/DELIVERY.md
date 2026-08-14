# ZeroDB Delivery, Ack & Resume (M0e.2)

**Version:** 0.1.0-draft
**Status:** normative (**draft-1 profile**). M0e.2 contract for ISSUES H4 / H11 (contract layer). On-wire enforcement ships M3. All formats are **draft-1, unfrozen** until an explicit Decision Log freeze names a versioned profile.
**Authority:** delivery semantics, anti-replay, batch outcomes, resume cursors, receipt vs durable ack. WAL atomicity is [WAL.md](WAL.md). Relay framing is RELAY-SPEC.

---

## 1. Delivery contract

- **At-least-once:** a sender MAY retransmit any op not yet covered by a durable anti-replay commitment at the receiver.
- **Idempotent apply:** KERNEL dedup by `(ds, OpId)` (I-3) makes retransmit safe.
- **Order:** within a batch, receivers process ops in array order for acknowledgements, but materialization is set-based (I-1).

## 2. Anti-replay

`AntiReplayState = set of OpId` (logical). Implementation MAY compact to a **causal frontier** (M0f) once all ops below that frontier are durable; compaction MUST NOT drop an OpId still eligible for late retransmit within the protocol lifetime.

Named outcomes when an op arrives:

| Outcome | Meaning |
|---------|---------|
| `ACCEPT` | new OpId; enter apply pipeline |
| `DUPLICATE` | OpId already in anti-replay / applied |
| `REJECT` | failed decode/authz/sig (with reason tag) |

## 3. Batch outcomes

A batch of N ops yields N ordered outcomes. Partial success is allowed: earlier ops may `ACCEPT` while a later op `REJECT`s. The batch ack lists per-op outcomes; the sender retransmits only non-`ACCEPT`/`DUPLICATE` failures that are retryable.

## 4. Resume cursor

`Cursor = { frontier: Frontier, epoch: uint }` where `Frontier` is the M0f map `PeerId → {op_id, physical_ms, logical}` (CX-05). An OpId-only `last_acked_op_id` cannot represent reorderable receipt.

- After a disconnect, the receiver includes its cursor in the next pull/subscribe.
- Sender and receiver are **independent** state machines. The sender holds ops with author/HLC; the receiver publishes a frontier built from ops it has accepted.
- Sender retransmits held ops **not covered** by the receiver frontier: for author A, an op is covered iff `Frontier[A]` exists and `order(op) ≤ order(Frontier[A])` under KERNEL §4.5.
- A late op behind the tip is therefore not retransmitted (it is already implied by the cursor). Set-difference of two in-process id sets is **not** a conforming resume model.
- Cursor advance is monotonic per peer: never rewinds without an explicit reset (datastore re-bootstrap).

## 5. Receipt vs durable ack (H11)

| Ack kind | When | Guarantees |
|----------|------|------------|
| `RECEIPT` | after structural accept into inbound buffer | may be lost on crash before WAL sync |
| `DURABLE` | after `wal_sync` of the op (WAL.md) | survives crash (I-14) |

L1 relays may only emit `RECEIPT`. L2 relays that claim catch-up backup MUST emit `DURABLE` only after L2 persistence. Product enforcement M3+.

## 6. Loss / reorder / resume model

Reference model state per peer link:

```
Sender   { held: map OpId → {author, ts} }
Receiver { seen: map OpId → {author, ts} }
Cursor   { frontier: Frontier, epoch }
```

Steps: `hold(op)`, `deliver(op)` (may drop/reorder), `resume` (sender minus receiver cursor).

Vectors exercise: reorder → same materialization; drop → retransmit on resume; duplicate after ack → `DUPLICATE`.

## 7. Conformance

| `type` | Checks |
|--------|--------|
| `delivery-schedule` | schedule of send/deliver/ack/resume → final seen + outcomes |

---

*Wire encoding of acks/cursors is M3; this document is the contract-model layer.*
