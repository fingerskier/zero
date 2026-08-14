# ZeroDB Frontiers, Checkpoints & Snapshots (M0f)

**Version:** 0.1.0-draft
**Status:** normative (**draft-1 profile**). Closes ISSUES C7 / O7 at the **contract** layer. **GC remains disabled** until M5 tests pass. Snapshot shipping is M4. All formats are **draft-1, unfrozen** until an explicit Decision Log freeze names a versioned profile.
**Authority:** causal frontiers, peer acks/retirement, checkpoint/snapshot identity. Merkle roots from [MERKLE.md](MERKLE.md); delivery cursors from [DELIVERY.md](DELIVERY.md).

---

## 1. Causal frontier

A **frontier** summarizes which ops are included without listing the full set:

```
Frontier = map PeerId → OpId
```

Interpretation: for each peer P, `Frontier[P]` is the greatest OpId (KERNEL §4.5 total order among P's ops) such that **all** of P's ops ≤ that OpId in §4.5 order are in the accepted set. Peers absent from the map have no known ops.

**Compactness (O7):** `|Frontier|` equals the number of distinct authors with accepted ops (not per-op). Checkpoint translation: when compacting, replace a set of OpIds with the frontier of that set.

## 2. Peer acknowledgement

`PeerAck = { peer: PeerId, frontier: Frontier, at_ts: HLCTimestamp }`

- Durable peer acks ride the WAL (same crash boundary as ops) when persisted locally.
- "All known peers have acked through F" means every peer in the **active membership set** has a durable PeerAck whose frontier dominates F (pointwise: for each author, ack's OpId ≥ F's OpId in §4.5 order among that author's ops).

## 3. Peer retirement / lease

- Each peer has `last_seen` wall/HLC and a **lease** duration (default 30 days wall-clock for product; model uses fixture `lease_expiry_ms`).
- After lease expiry without reconnect, the peer is **retired**: removed from the active membership set for GC *planning* only.
- **GC stays disabled** (C7 / SPEC): retirement does not delete ops or advance a GC watermark in v0.1/M0.

## 4. Late ops

An op that arrives with `deps` or author history implying it should have been below a published frontier used for a checkpoint is tagged `LATE_OP`: accept into the set (SEC), recompute roots, **do not** silently drop. Compaction that would make late ops undecidable is forbidden while GC is disabled.

## 5. Checkpoint / snapshot identity

```
SnapshotId = BLAKE3(
  domain("snapshot") ‖ snapshot_format_version (u8)
  ‖ DatastoreId ‖ FrontierCanonicalCBOR ‖ MerkleRoot ‖ tail_boundary_OpId
)
```

- `domain("snapshot")` = `zerodb-snapshot-v1`
- `snapshot_format_version` = 1
- **Tail boundary:** greatest OpId in the snapshot's accepted set under §4.5 total order (or null if empty).
- L2 relays **store peer-produced authenticated snapshots**; they do not materialize schema-blind state into application objects (decision for v0.1).

## 6. Root comparison across checkpoints

Two peers compare: if Merkle roots equal → same op set. If not, either run MERKLE walk **or** if both hold a snapshot with equal `SnapshotId`, bootstrap from snapshot + tail. Unequal SnapshotIds with equal roots are impossible by construction.

## 7. Conformance

| `type` | Checks |
|--------|--------|
| `frontier-build` | op set → frontier map |
| `snapshot-id` | frontier + root + tail → SnapshotId |
| `late-op` | op vs published frontier → `LATE_OP` tag or accept path |

GC implementation vectors are **out of scope** (M5).

---

*Draft-1, unfrozen until an explicit Decision Log freeze names a versioned profile.*
