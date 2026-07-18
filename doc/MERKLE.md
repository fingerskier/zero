# ZeroDB Merkle Sync Tree (M0c)

**Version:** 0.1.0-draft
**Status:** normative (**draft-1 profile**). The M0c exit checklist closed 2026-07-18 ([ISSUES Decision Log](ISSUES.md)): root vectors + mismatch-recovery transcripts are green in **both** conformance runners and CI-blocking. Wire framing of walk messages ships M3. Byte-level **freeze** still happens only at the composite M0 gate ([SPEC §10](SPEC.md)).
**Authority:** this document owns the canonical **Merkle sync tree** (derived structure for delta sync) and the abstract mismatch-recovery state machine (ISSUES C3). Operation identity and total order come from [KERNEL.md](KERNEL.md). [SPEC §2.6](SPEC.md) is the informative overview; on conflict this document wins for tree bytes and traversal.

Keywords MUST/SHOULD/MAY per RFC 2119. Invariant references (I-*) per [INVARIANTS.md](INVARIANTS.md).

---

## 1. Role

The Merkle sync tree is a **derived, rebuildable** structure over a datastore's accepted operation set. It is **not** the causal graph. Equal accepted op sets MUST produce equal roots (I-11). Unequal roots MUST be traversable to a concrete delta via the §4 state machine.

---

## 2. Versioning

| Constant | Value | Binding |
|----------|-------|---------|
| `merkle_format_version` | 1 | Domain-separated into every leaf/node hash; not an operation preimage field |
| `bucket_width_ms` | 60_000 | Fixed for v1 profile (1-minute buckets of `ts.physical_ms`) |

A future change to either constant is a new format generation with new vectors.

---

## 3. Canonical tree construction

### 3.1 Inputs

- The set of **accepted** operations in one datastore (after decode, signature, authz, dedup, equivocation exclusion — KERNEL pipeline steps 1–5).
- Each op contributes: `OpId` (32 B), `ts.physical_ms` (u64), and for leaf order within a bucket the KERNEL §4.5 total-order key `(physical_ms, logical, author, OpId)`.

### 3.2 Buckets

```
bucket_index = physical_ms / bucket_width_ms   // integer division, u64
```

Ops with equal `bucket_index` share a leaf. The set of **active** bucket indices is the sorted unique indices present in the op set (no spanning empty interior buckets as active leaves — empty padding only appears inside the power-of-two tree as synthetic empty leaves, §3.4).

### 3.3 Leaf hash

For a non-empty bucket with ops sorted by KERNEL §4.5 total order:

```
leaf_preimage = domain("merkle_leaf") ‖ merkle_format_version (u8 BE)
                ‖ bucket_index (u64 BE)
                ‖ OpId₀ ‖ OpId₁ ‖ … ‖ OpIdₖ−1
leaf_hash = BLAKE3(leaf_preimage)
```

`domain("merkle_leaf")` = registry string `zerodb-merkle-leaf-v1` (UTF-8).

**Empty leaf** (padding only):

```
empty_leaf = BLAKE3(domain("merkle_empty") ‖ merkle_format_version (u8 BE))
```

`domain("merkle_empty")` = `zerodb-merkle-empty-v1`.

### 3.4 Binary tree and root

1. Let `L = [leaf_hash for each active bucket in ascending bucket_index order]`.
2. If `L` is empty: `root = empty_leaf` (same constant as empty leaf).
3. Else let `n = |L|`, `p = next_power_of_two(n)` (1 if n=0 already handled). Pad `L` on the **right** with `empty_leaf` until length `p`.
4. Bottom-up levels: for each pair `(left, right)` at the current level,

```
node = BLAKE3(domain("merkle_node") ‖ merkle_format_version (u8 BE) ‖ left ‖ right)
```

`domain("merkle_node")` = `zerodb-merkle-node-v1`.

5. The single hash at the top is the **Merkle root** (32 B).

### 3.5 Properties

- Inserting/removing an op changes exactly one leaf and the path to the root.
- Equal op sets ⇒ equal roots (I-11), independent of arrival order (set-based).
- Empty datastore root is the fixed `empty_leaf` constant.

---

## 4. Mismatch-recovery state machine (abstract)

Peers exchange **roots** first. If equal → done. If unequal, they walk down the tree.

**Messages (abstract, not wire):**

| Message | Payload |
|---------|---------|
| `RootOffer` | `root`, `merkle_format_version`, `bucket_width_ms` |
| `NodeRequest` | `path` (bitstring / level+index) or `node_hash` |
| `NodeResponse` | `node_hash`, `left_hash`, `right_hash` (or leaf payload) |
| `LeafRequest` | `bucket_index` |
| `LeafResponse` | `bucket_index`, sorted `OpId[]` (or full ops — product choice; model uses OpIds) |
| `Delta` | ops the requester is missing |

**Walk rule:** at an internal node, if the peer's child hash matches local, skip that subtree; else recurse. At a leaf mismatch, exchange OpId sets and request missing ops by id (full op body from local oplog / peer). After applying missing ops (full KERNEL pipeline), recompute root; if still unequal, continue (or fail with `MERKLE_DIVERGENCE` after a bounded number of rounds).

**Concurrent writes:** either peer may append ops during the walk; the model treats the walk over a **frozen snapshot** of each peer's op set at walk start, then a final root re-compare. Live product behavior may restart the walk (M3).

A full two-way **transcript** fixture records the ordered message list for unequal fixtures until roots match.

---

## 5. Conformance vectors

| `type` | Checks | Invariants |
|--------|--------|------------|
| `merkle-root` | op set description → root hex; permutation-invariant; empty set; single op; multi-bucket | I-11 |
| `merkle-transcript` | two peer op sets → message transcript → converged roots | I-11 |

Lifecycle: red in `xfail/`, promote on both-runners-green. Root family MERKLE-001..004 and transcript family MERKLE-T-001..004 promoted 2026-07-18; M0c exit closed with them (Decision Log + `plan/LEDGER.md`).

---

## 6. Out of scope (deliberate)

- WebSocket/CBOR framing of messages (RELAY-SPEC / M3).
- Checkpoint/compaction interaction with roots (M0f).
- Adaptive bucket widths (post-v0.1).

---

*Draft change policy: until composite M0, ordinary review; after freeze, format-version bump required for byte-affecting changes.*
