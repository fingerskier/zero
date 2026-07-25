# ZeroDB Version Policy (M0e.3)

**Version:** 0.1.0-draft
**Status:** normative (**draft-1 profile**). Closes ISSUES H7 / H9 registry half for M0. Rolling upgrade tests are M4.
**Authority:** version namespaces and compatibility windows. Constants live in [`conformance/registry.json`](../conformance/registry.json); [KERNEL.md](KERNEL.md) §1 owns operation-format binding.

---

## 1. Namespaces (HX-11)

| Namespace | Authority | In preimage? | Wire negotiation? |
|-----------|-----------|--------------|-------------------|
| `operation_format_version` | KERNEL.md | yes (every op) | no — ops carry it |
| `schema_ir_format_version` | SCHEMA.md | via schema IR bytes | no |
| `merkle_format_version` | MERKLE.md | tree hashes only | RootOffer |
| `snapshot_format_version` | FRONTIER.md | snapshot artifacts | snapshot messages |
| `storage_format_version` | zerodb-storage (M1) | never | never |
| `relay_protocol_version` | RELAY-SPEC | never | HELLO/WELCOME |

Document versions (`0.x-draft`) are **not** wire versions. Wire `protocol_version: 1` will map to RELAY-SPEC 0.2 + this registry snapshot at freeze; all formats are draft-1, unfrozen until an explicit Decision Log freeze names a versioned profile.

## 2. Compatibility window (v0.1)

- A peer MUST reject ops with unknown `operation_format_version` (`FORMAT_UNSUPPORTED`).
- A peer MUST accept the current version only in v0.1 (window size 1). Multi-version windows are post-v0.1.
- Merkle RootOffer with mismatched `merkle_format_version` or `bucket_width_ms` → abort walk with `MERKLE_VERSION_MISMATCH`.

## 3. Decode limits (O6 provisional, ratified for draft-1)

Registry `limits` (current):

- `max_operation_bytes` = 65536  
- `max_batch_bytes` = 262144  
- `max_batch_ops` = 512  
- `max_cbor_depth` = 16  
- `max_deps_per_op` = 64  

Violations are **pre-auth** decode errors (do not require signature verification to reject). Evidence: KERNEL §3 + OP-NEG vectors + registry.

## 4. Identifier encodings

Registry `identifier_encodings` is normative for lengths; KERNEL §2 for derivation. H9 full wire schema generation is M3 harness work; the registry is the M0 machine-readable seed.

---

*These constants are draft-1, unfrozen: freeze only via an explicit Decision Log entry naming a versioned profile.*
