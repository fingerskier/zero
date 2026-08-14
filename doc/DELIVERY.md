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

`Cursor = { last_acked_op_id: OpId | null, epoch: uint }` (opaque to relays).

- After a disconnect, the receiver includes its cursor in the next pull/subscribe.
- Sender retransmits ops **not** covered by anti-replay at the receiver (model: all ops whose OpId is not in the receiver's seen set and that the sender still holds).
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
{ seen: set OpId, cursor: Cursor, inbox: [op…], outbox: [op…] }
```

Steps: `send(op)`, `deliver(op)` (may drop/reorder in adversarial schedules), `ack(outcomes)`, `resume(cursor)`.

Vectors exercise: reorder → same materialization; drop → retransmit on resume; duplicate after ack → `DUPLICATE`.

## 7. Conformance

| `type` | Checks |
|--------|--------|
| `delivery-schedule` | schedule of send/deliver/ack/resume → final seen + outcomes |

---

*Wire encoding of acks/cursors is M3; this document is the contract-model layer.*
