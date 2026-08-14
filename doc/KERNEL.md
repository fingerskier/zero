# ZeroDB Kernel Specification — Operations, Encoding & Semantics (M0a)

**Version:** 0.1.0-draft
**Status:** normative (**draft-1 profile**). The M0a exit checklist closed 2026-07-16 ([ISSUES Decision Log](ISSUES.md)): every rule below is backed by golden vectors green in **both** conformance runners and CI-blocking ([conformance/](../conformance/README.md)); the corpus grows as later packages land. All formats are **draft-1, unfrozen** until an explicit Decision Log freeze names a versioned profile — closing composite M0 (or M1) does not imply freeze. Until then, a byte-affecting change re-runs the resolution checklist rather than bumping `operation_format_version`.
**Authority:** this document owns the operation algebra, canonical encoding, preimages, identifier encodings, HLC state machine, and the CRDT semantic kernel (ISSUES C1, C4-context, plus the DQ-5/DQ-6/DQ-7/DQ-8 ratified directions). [SPEC.md](SPEC.md) owns architecture and roadmap; [RELAY-SPEC.md](RELAY-SPEC.md) owns relay behavior. The machine-readable constants live in [`conformance/registry.json`](../conformance/registry.json) — on any disagreement, the registry is wrong and MUST be fixed to match this document, never silently vice versa.

Keywords MUST/SHOULD/MAY per RFC 2119. Invariant references (I-*) per [INVARIANTS.md](INVARIANTS.md).

---

## 1. Version namespaces (HX-11)

Five independent version namespaces exist. They MUST never be conflated:

| Namespace | Authority | Binding |
|-----------|-----------|---------|
| `operation_format_version` | this document | Signed and hashed into **every operation preimage**. Persistent forever. Incrementing it is a new format generation with its own vectors. |
| `schema_epoch` | SCHEMA.md §3 (M0b ✓) | Per-datastore causally ordered sequence bound into every **data** operation. Not a global constant. |
| `snapshot_format_version` | SPEC §7 (M0f) | Snapshot/checkpoint artifacts only. |
| `storage_format_version` | zerodb-storage (M1) | Local on-disk layout only. MUST NOT appear on the wire or in any preimage. Experimental layout notes: [M1-LOCAL.md](M1-LOCAL.md). |
| `relay_protocol_version` | RELAY-SPEC | Connection negotiation (HELLO/WELCOME) only. MUST NOT enter any operation preimage or persistent artifact. |

The current values and statuses are in the registry. `operation_format_version = 1` names *this draft*; it is not frozen.

## 2. Identifiers & primitives

All lengths in bytes. Registry `identifier_encodings` mirrors this table.

| Type | Len | Derivation |
|------|-----|------------|
| `OpId` | 32 | `BLAKE3(domain("op_id") ‖ id-preimage)` — §4.4 |
| `PeerId` | 32 | `BLAKE3(Ed25519 device public key)` (DQ-1; full hash, no truncation) |
| `PrincipalId` | 32 | `BLAKE3(Ed25519 principal root public key)` (DQ-1) |
| `DatastoreId` | 32 | `BLAKE3(domain("genesis") ‖ genesis-preimage)` (DQ-2) — §4.6 |
| `NodeId`, `EdgeId`, `GroupId` | 16 | UUIDv7 (RFC 9562) |
| `KeyId` | 16 | `BLAKE3(group symmetric key)` truncated to 16 (DQ-5) |
| Ed25519 public key / signature | 32 / 64 | RFC 8032 |
| `HLCTimestamp` | — | struct: `physical_ms: u64`, `logical: u16`, `peer: PeerId` (§5) |

Domain-separation strings (registry `domain_separation`) are prepended as raw UTF-8 bytes to every hash/signature preimage; no two preimage kinds share a domain string.

## 3. Deterministic CBOR profile

Canonical encoding is **RFC 8949 Core Deterministic Encoding**, further restricted:

1. Definite-length encoding only (no indefinite arrays/maps/strings).
2. Map keys MUST be text strings, unique, ordered bytewise-lexicographically by their encoded form. Decoders MUST reject duplicate keys (I-6).
3. Integers in shortest form; no bignums where a fixed-width field is specified.
4. **No floating point** anywhere in operation encodings. Application float values are carried as IEEE-754 binary64 **bytes** inside a tagged value structure (M0b defines the value-model detail), never as CBOR floats.
5. No CBOR tags except those this document registers (currently: none).
6. Byte strings for all identifiers/hashes/keys/signatures — never hex/base64 text.
7. Decoder resource limits (registry `limits`): max depth 16, max operation 64 KiB, max batch 256 KiB / 512 ops, max `deps` 64. Exceeding any limit is a decode **error**, not a truncation. (Full pre-auth decode profile: M0e.3.)
8. Unknown map keys: **reject** in operation encodings (forward compatibility happens via `operation_format_version`, not silent field tolerance).

Round-trip rule (I-6): decode → re-encode MUST be byte-identical; every golden vector is checked in both directions.

## 4. Operation algebra

### 4.1 Common structure

Every operation is a CBOR map with exactly these keys (order per §3):

```
{
  "v":    operation_format_version (uint)
  "ds":   DatastoreId                       // DQ-2; C4 signed context
  "ep":   schema_epoch (uint)               // 0 until M0b activates epochs
  "author": PeerId                          // authoring device
  "ts":   HLCTimestamp {p: u64, l: u16}     // author's clock; peer implied by "author"
  "deps": [OpId, ...]                       // causal dependencies (≤ 64)
  "grp":  GroupId | null                    // atomic-group membership (C8/M0e.1)
  "kind": uint                              // variant tag, §4.2
  "body": <variant-specific map>            // §4.2/§4.3
  "id":   OpId                              // §4.4 — excluded from both preimages
  "sig":  Ed25519Signature                  // §4.4 — excluded from both preimages
}
```

`ds`, `v`, and `ep` inside the signed region close the C4 context half: an operation cannot be replayed across datastores, format generations, or epochs without breaking its signature (I-9).

### 4.2 Variants

| `kind` | Variant | Body (summary) | Class |
|--------|---------|----------------|-------|
| 0 | `Genesis` | founder `PrincipalId`, salt (16B random), initial epoch ref, format versions — body in [AUTH.md §2](AUTH.md) | control |
| 1 | `CreateNode` | `NodeId`, label (text ≤ 256B) | data |
| 2 | `CreateEdge` | `EdgeId`, label, `source: NodeId`, `target: NodeId` | data |
| 3 | `SetProperty` | entity ref (`NodeId`/`EdgeId` + tag), property path (text), CRDT payload (§6) | data |
| 4 | `Tombstone` | entity ref | data |
| 5 | `SchemaEpoch` | epoch record — body defined in [SCHEMA.md §3](SCHEMA.md) | control |
| 6 | `CapabilityGrant` | subject `PrincipalId`, scopes, expiry, delegable flag — body in [AUTH.md §3](AUTH.md) | control |
| 7 | `CapabilityRevoke` | reference to grant `OpId`, reason code — body in [AUTH.md §3](AUTH.md) | control |
| 8 | `KeyRecord` | device certificate / rotation / group-key distribution — body in [AUTH.md §1](AUTH.md) | control |
| 9 | `Checkpoint` | reserved for M0f | control |

Reserved variants (5, 6, 7, 8, 9) have their **tags and preimage participation** fixed here so later packages cannot invalidate M0a; their body schemas land with their owning package. Decoders at `operation_format_version 1` MUST parse the envelope of every variant and MAY reject not-yet-specified bodies with `VARIANT_UNSUPPORTED`.

### 4.3 Value model (payload primitives)

CRDT payloads (§6) carry values from this closed set: `null`, `bool`, `int (i64)`, `float64-bytes`, `text`, `bytes`, `EncryptedValue` (§7), `BlobRef` (§8). Composite values (arrays/maps) are M0b schema territory; the kernel treats them as opaque `bytes` until then.

### 4.4 Preimages, `OpId`, signature

- **id-preimage** = `domain("op_id") ‖ canonical CBOR of the map §4.1 without keys "id" and "sig"`.
- `OpId = BLAKE3(id-preimage)` (32B).
- **sig-preimage** = `domain("op_signature") ‖ OpId`. `sig = Ed25519-sign(device key, sig-preimage)`.
- Verification: recompute `OpId` from received bytes (MUST match `"id"`), verify `sig` against the author's device public key resolved per the DQ-1 certificate chain (M0d). Either failure ⇒ the operation is invalid and MUST NOT be forwarded as valid, stored as valid, or applied (I-8).

Signing the `OpId` (rather than the raw preimage) makes signature verification independent of re-serialization once the id is checked, and binds the signature transitively to every signed field.

### 4.5 Duplicates, equal timestamps, equivocation (DQ-8)

- **Total order** over operations: `(ts.physical_ms, ts.logical, author, id)` — bytewise comparison for the last two. Every deterministic choice in the kernel (LWW, arbitrary-but-deterministic iteration) MUST use this order and nothing else.
- **Duplicate** = byte-identical operation (same `OpId` in the same datastore): idempotent no-op on re-application (I-3).
- **Equivocation** = two or more operations with the same `author` and equal `(physical_ms, logical)` but distinct `OpId`s (an *equivocation group*). Exclusion is a pure function of the operation set, so every peer converges (I-1): **every operation in an equivocation group is excluded from materialized state**, regardless of arrival order or when the group was detected. Detection additionally raises an advisory device-quarantine signal to the application; lifting it (re-certification) is M0d policy and never changes the exclusion rule above. Model outcome tag: `EQUIVOCATION`.

### 4.6 Genesis (DQ-2)

`Genesis` is the only operation whose `ds` field is all-zero at signing time; `DatastoreId = BLAKE3(domain("genesis") ‖ id-preimage-of-genesis)` and every subsequent operation MUST carry that id in `ds`. A datastore has exactly one valid genesis; peers MUST reject a second genesis for the same id (trivially unforgeable by construction).

## 5. HLC state machine

State: `latest: HLCTimestamp` (this device's last issued/observed-max), `max_drift_ms` (default 60 000).

| Transition | Rule |
|------------|------|
| `local()` | `p = max(wall, latest.p)`. If `p == latest.p`: `l = latest.l + 1`; on u16 overflow `p += 1, l = 0`. Else `l = 0`. Result strictly > `latest`; becomes `latest`. |
| `recv(r)` | Reject with `DRIFT_EXCEEDED` if `r.p > wall + max_drift_ms` (peer-side acceptance policy beyond rejection: H1/M3b). Else `p = max(wall, latest.p, r.p)`; base counter = max of the counters of whichever of {latest, r} tie `p` (absent if wall alone is max ⇒ `l = 0`); `l = base + 1`, on overflow `p += 1, l = 0`. Result strictly > both `latest` and `r` (I-5). |
| `restart` (DQ-7) | `latest := max HLCTimestamp over own-device operations in the durable oplog` (an auxiliary clock record MAY serve as a cache but is never authoritative). Guarantees I-4 across restart, wall-clock rollback, and restore — a restored-then-diverged device manifests as equivocation (§4.5), not as clock regression. |
| durability | `latest` is durable **in the same atomic boundary** as the operation carrying it (M0e.1 crash model obligation `hlc_persist ⊆ wal_append`). |

Reference implementation of `local`/`recv`/`restart`: `zerodb-core/src/hlc.rs` (experimental until vectors green).

## 6. CRDT semantic kernel (M1 profile)

Application pipeline for every arriving operation, in order: **(1)** structural decode (§3 limits) → **(2)** id + signature verification (§4.4) → **(3)** authorization check (M0d; kernel model stubs this as a provided predicate) → **(4)** dedup by `(ds, id)` (I-3) → **(5)** equivocation check (§4.5) → **(6)** causal readiness: all `deps` applied, else buffer (bounded; overflow policy M0e.2) → **(7)** apply per the variant/CRDT rules below. Steps 1–6 are identical for every CRDT; SEC (I-1) follows from 4–7 being order-independent.

Per-type state and apply rules (payloads are `body.payload` of `SetProperty`):

| CRDT | State | Payload ops | Apply rule |
|------|-------|-------------|------------|
| `LWW<T>` | `(value, winning-op total-order key)` | `set(value)` | Keep the operation greater in the §4.5 total order. Equal full keys are impossible (§4.5). Fixes A-03. |
| `GCounter` | map `PeerId → u64` | `inc(n>0)` | `state[author] += n` — but dedup (step 4) means each *operation* applies exactly once; the per-peer entry is the sum of that peer's applied ops. Overflow saturates at u64::MAX with model tag `COUNTER_SATURATED`. |
| `PNCounter` | two GCounter maps (pos, neg) | `inc(n)`, `dec(n)` | As GCounter per map; value = pos − neg (i128 read). |
| `ORSet<T>` | map `element → set of dots`, plus tombstoned dots | `add(elem)` (dot = `(author, ts)` of the op), `remove(elem, observed-dots)` | `add` inserts its dot; `remove` tombstones exactly the dots it observed (carried in payload). Element present iff it has ≥ 1 non-tombstoned dot. Concurrent add wins (I-1, add/remove commute on distinct dots). |
| `Flag` | ORSet of unit | `enable` / `disable(observed)` | Enable-wins by the ORSet rule. |

Out-of-order and permutation behavior: for any set of valid operations, every arrival order and interleaving of steps 4–7 yields identical state (I-1) — this is the property the permutation/replay vector suites falsify.

`MVRegister`, `LWWMap`, `RGA` (M2) extend this kernel in place; their tags in the payload envelope are reserved with the M0b schema work.

## 7. Encrypted values (DQ-5)

`EncryptedValue` envelope, byte layout inside a CBOR byte string:

```
version (u8, = 1) ‖ key_id (16B) ‖ nonce (24B) ‖ ciphertext+tag (XChaCha20-Poly1305)
```

AAD is bound to a **slot context that excludes ciphertext and OpId** (CX-03; Decision Log 2026-08-14), so seal can complete before `OpId` exists:

```
slot_preimage = domain("value_slot") ‖ ds ‖ ep (u64 BE) ‖ path (UTF-8)
                ‖ author ‖ physical_ms (u64 BE) ‖ logical (u16 BE)
SlotId        = BLAKE3(slot_preimage)     (32B)
AAD           = domain("value_aad") ‖ SlotId
```

`domain("value_slot")` = `zerodb-value-slot-v1`. An envelope authenticates datastore, author, timestamp, epoch, and property path — replay into any other slot, or onto a different author/timestamp, fails decryption (I-10). Construction order is: form the slot context from envelope fields that are known before encryption → seal → place the envelope in `body` → hash the body to `OpId` → sign. Negative vectors MUST include: wrong AAD component (each slot field), truncated envelope, unknown version, unknown `key_id`. At least one vector MUST construct a complete operation (plaintext → envelope → body → OpId) without supplying an external `OpId`.

## 8. Large values & limits (DQ-6)

- Limits per registry `limits` (provisional, O6): operation ≤ 64 KiB, batch ≤ 256 KiB / 512 ops, `deps` ≤ 64, CBOR depth ≤ 16. Violations are decode errors.
- `BlobRef` value: `blake3 (32B) ‖ total_size (u64 BE) ‖ codec (u16 BE, registry blob_codecs)`. At `operation_format_version 1`, conforming implementations MUST parse `BlobRef` and MUST reject materialization with `BLOB_UNSUPPORTED`. Transfer/storage protocol: M4 (O1). This reservation is what keeps O1 from invalidating this format generation.

## 9. Conformance vectors

Vector types this document owns (registered in both runners' dispatch tables as they are implemented):

| `type` | Checks | Invariants |
|--------|--------|------------|
| `op-encoding` | golden bytes ↔ decoded form, round-trip, negative decodes (dup keys, floats, limits, unknown keys) | I-6 |
| `op-preimage` | `OpId`/signature derivation from golden bytes; tamper negatives | I-6, I-8 |
| `hlc-transition` | `local`/`recv`/`restart` step sequences → expected timestamps or error tags | I-4, I-5 |
| `crdt-apply` | operation sets applied under stated orders/permutations → expected state; duplicate/replay; equivocation outcomes | I-1, I-2, I-3, I-7 |
| `envelope` | §7 encrypt/decrypt vectors incl. AAD negatives | I-10 |

Lifecycle per the [promotion policy](../conformance/README.md): vectors land red in `vectors/xfail/` and promote to `vectors/required/` (CI-blocking) as soon as they are green in **both** runners; the M0a gate requires the entire suite promoted. `op-encoding`/`op-preimage`/`envelope` golden bytes are emitted by the first codec implementation and cross-checked by the second; `hlc-transition` and `crdt-apply` vectors are hand-authored from this document (first set ships with this draft).

---

*Draft change policy: until an explicit Decision Log freeze names a versioned profile, edits to this document require only ordinary review; after a freeze is declared, any byte-affecting change requires a new `operation_format_version`.*
