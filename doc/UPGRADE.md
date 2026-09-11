# ZeroDB version / upgrade matrix (v0.1 window)

**Status:** packaging matrix for the draft-1 / unfrozen M3c slice. **Not** a format freeze. **Not** the M4 adjacent-version rollback/upgrade matrix (SPEC §10 M4b) and **not** M4/M5 rolling-upgrade tests (VERSIONS.md; SPEC M5b).

**Authority:** [VERSIONS.md](VERSIONS.md) (policy) + [`conformance/registry.json`](../conformance/registry.json) (current constants). On conflict, VERSIONS / KERNEL / SCHEMA / MERKLE / FRONTIER / RELAY-SPEC win over this file; this file must be edited to match them.

Window size for **v0.1** is **1**: accept the current draft-1 value of each namespace only. Multi-version windows are post-v0.1.

---

## 1. Current draft-1 values

| Namespace | Current | Source | Binding |
|-----------|---------|--------|---------|
| `operation_format_version` | `1` | registry `version_namespaces`; KERNEL §1 | Signed into every op preimage (`v`). Not negotiated. |
| `schema_ir_format_version` | `1` | SCHEMA.md §2 (`v`); registry notes it on `schema_epoch` | Inside schema IR bytes. Not a global epoch. |
| `schema_epoch` | per-datastore (`null` as a global) | registry `schema_epoch` | Causal sequence on data ops; n=1 / empty migration in this slice. |
| `merkle_format_version` | `1` | registry `merkle`; MERKLE.md §2 | Tree hashes + RootOffer. |
| `bucket_width_ms` | `60000` | registry `merkle` | Fixed with merkle v1; mismatch aborts the walk. |
| `snapshot_format_version` | `1` | registry `version_namespaces`; FRONTIER.md §5 | Snapshot/checkpoint identity only. Shipping is M4. |
| `storage_format_version` | experimental `1` written by `LocalStore` (registry `current`: `null`) | registry; [M1-LOCAL.md](M1-LOCAL.md) | Local SQLite meta only. Never on the wire or in a preimage. |
| `relay_protocol_version` | `1` | registry `version_namespaces` + `relay_wire.protocol_version` | HELLO/WELCOME only. Document 0.2.2-draft ≠ wire `1`. |

Document versions (`0.x-draft`) are **not** wire versions. `operation_format_version = 1` names this draft; it is not frozen.

`schema_epoch` is listed because the registry treats it as a namespace; it is **not** a global constant and is not an upgrade-window peer of the six VERSIONS.md format namespaces.

---

## 2. v0.1 window (size 1)

A peer MUST accept the current value only. Unknown or other values are rejected — not rewritten, not silently tolerated.

| Namespace | Accept | Reject | Named outcome |
|-----------|--------|--------|----------------|
| `operation_format_version` | `1` | any other / unknown | `FORMAT_UNSUPPORTED` (VERSIONS §2). Store ingress today errors `unsupported operation version {v}` (`zerodb-storage` `validate_wire_for_ds`). |
| `schema_ir_format_version` | `1` | `v` ≠ 1 | `IR_VERSION_UNSUPPORTED` (SCHEMA.md §2; SCHEMA-NEG-004). |
| `merkle_format_version` | `1` | mismatch on RootOffer | `MERKLE_VERSION_MISMATCH` (VERSIONS §2). Clients today fail closed with `unsupported merkle_format_version` (Rust `relay_client`, TS peer). |
| `bucket_width_ms` | `60000` | any other on RootOffer | same walk abort (`MERKLE_VERSION_MISMATCH` / `unsupported merkle bucket_width_ms`). |
| `snapshot_format_version` | `1` | other | Snapshot identity is content-addressed at v1; there is no v0.1 shipping/upgrade path (M4). |
| `storage_format_version` | experimental `1` | other written value | `unsupported storage_format_version {v}` on open. Missing meta is **backfilled** to `1` (legacy DBs) — not a wire upgrade. Layouts may still change while unfrozen. |
| `relay_protocol_version` | `1` | other HELLO | Connection negotiation reject (RELAY ERROR). Not an op-format error. |

`FORMAT_UNSUPPORTED` and `MERKLE_VERSION_MISMATCH` are the **policy names**. Implementations may still surface a descriptive string; do not treat string drift as a new format generation.

---

## 3. What v0.1 does *not* do

- No adjacent-version accept (vN talks to vN−1 / vN+1). That matrix is **M4b** (SPEC §10: “Adjacent-version rollback/upgrade matrix”).
- No rolling-upgrade tests across released binaries. VERSIONS.md: “Rolling upgrade tests are M4.” SPEC M5b owns production rolling upgrades.
- No wrap-body freeze; no claim that `storage_format_version = 1` is a frozen on-disk layout.
- No schema migration DSL across mixed-version peers (SCHEMA.md: cross-peer shipping remains M4). This slice’s SchemaEpoch is n=1 / empty `migration`.
- No `BlobRef` materialization (`BLOB_UNSUPPORTED` at operation_format 1).

Until an explicit Decision Log freeze names a versioned profile, a byte-affecting change **re-runs the approved-resolution checklist** rather than bumping a namespace and keeping old bytes readable.

---

## 4. Forward pointer (M4 / M5)

When M4b starts, extend this file (or a successor) with an adjacent-version table: which namespaces may coexist, rollback rules, and fixture IDs. Do not fill that table now. M5b rolling upgrades across adjacent **released** versions stay later still.

---

*Draft-1 / unfrozen. Constants copied from registry + VERSIONS; edit those first if the numbers move.*
