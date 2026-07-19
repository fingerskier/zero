# ZeroDB WAL & Group Reference Model (M0e.1)

**Version:** 0.1.0-draft
**Status:** normative (**draft-1 profile**). M0e.1 exit closed 2026-07-18 (ISSUES C8 / DQ-4). WAL-001..012 green both runners. **Layer 1** pure model; **Layer 2** SQLite at M1. Freeze only at composite M0 / explicit freeze Decision Log.
**Authority:** this document owns the abstract storage transaction boundary spanning oplog, materialized state, HLC durability, and group completeness (C8). Operation algebra comes from [KERNEL.md](KERNEL.md); authorization from [AUTH.md](AUTH.md). Delivery/ack/resume is M0e.2; version policy M0e.3.

Keywords MUST/SHOULD/MAY per RFC 2119. Invariant references (I-*) per [INVARIANTS.md](INVARIANTS.md).

---

## 1. Two-layer rule (P0-6 / SPEC §10)

1. **M0 contract-model (layer 1):** the pure state machine below, exercised by crash transcripts in both conformance runners — no I/O, no SQLite.
2. **M1 backend (layer 2):** map each named crash point onto real SQLite transaction boundaries and re-run the same transcripts with crash injection.

Closing M0e.1 does **not** require a durable backend. M1 owns backend fidelity.

---

## 2. Abstract storage primitives

The model state is a tuple:

```
State = {
  wal:      [WalRecord…],     // durable only after wal_sync
  wal_buf:  [WalRecord…],     // appended but not yet synced
  applied:  set of OpId,      // materialized op ids
  material: map OpId → body,  // opaque for C8; CRDT apply is KERNEL
  hlc:      HLCTimestamp | null,  // last durable own-device stamp
  groups:   map GroupId → GroupState,
  sealed:   set of GroupId
}
```

| Primitive | Effect |
|-----------|--------|
| `wal_append(record)` | Append `record` to `wal_buf` (not yet durable). |
| `wal_sync()` | Append `wal_buf` to `wal`; clear `wal_buf`. Durable. |
| `state_apply(op_id)` | If the corresponding record is in `wal` (synced), mark applied and store body in `material`. Idempotent. |
| `hlc_persist(ts)` | Set `hlc := ts`. **MUST** only be issued when the op carrying `ts` is already in `wal` or is co-bundled in the same atomic schedule step that includes `wal_append` for that op (DQ-7: `hlc_persist ⊆ wal_append` boundary). |
| `group_seal(group_id, manifest)` | If all member OpIds in `manifest` are applied, add `group_id` to `sealed`. Else reject with `GROUP_INCOMPLETE`. |
| `wal_truncate(up_to_index)` | Drop prefix of `wal` only if every truncated record is applied **and** every group those records belong to is sealed (or was never grouped). |

`WalRecord = { op_id: OpId, body: opaque, group: GroupId | null, author_ts: HLCTimestamp }`.

`GroupManifest = { group_id: GroupId, members: [OpId…], n: uint (= len(members)), abort: bool }`. Members MUST be unique; `n` MUST equal length. A signed manifest on the wire is M3; the model treats the manifest as an authenticated input.

---

## 3. Named crash points

A **schedule** is an ordered list of primitive calls. Between every adjacent pair of primitives, and before the first / after the last, a crash may occur.

| Crash point | Name | Meaning |
|-------------|------|---------|
| before any work | `CRASH_START` | Empty or prior durable state only |
| after `wal_append`, before `wal_sync` | `CRASH_AFTER_APPEND` | Buffered records lost |
| after `wal_sync`, before `state_apply` | `CRASH_AFTER_SYNC` | Durable wal; material may lag |
| after `state_apply`, before `hlc_persist` | `CRASH_AFTER_APPLY` | Material advanced; HLC may lag |
| after `hlc_persist` | `CRASH_AFTER_HLC` | HLC durable with applied ops |
| after `group_seal` | `CRASH_AFTER_SEAL` | Group completeness recorded |
| after `wal_truncate` | `CRASH_AFTER_TRUNCATE` | Prefix gone |

Implementations MAY insert additional named points but MUST support this set.

---

## 4. Recovery (`recover`)

```
recover(state):
  // 1. Discard unsynced buffer
  state.wal_buf := []
  // 2. Rebuild material from wal in order (idempotent state_apply)
  for record in state.wal:
    state_apply(record.op_id)   // if not already applied
  // 3. HLC resume (DQ-7 / KERNEL §5)
  state.hlc := max author_ts over wal records with author = self
               (or null if none)
  // 4. Re-evaluate group seals from manifests still referenced
  //    (incomplete groups remain unsealed — no partial materialization of group atomicity)
  return state
```

**Postconditions (must hold after every recover):**

- I-14: every acked/synced commit is present in `wal` and re-applicable.
- I-13: a group is sealed iff all members are applied; never partially sealed.
- I-3: re-apply is idempotent (duplicate `state_apply` is a no-op).
- DQ-7: `hlc` is never ahead of the durable oplog max for this device.

---

## 5. Group atomicity (C8)

- A grouped op carries `grp = GroupId` in the KERNEL envelope.
- Completeness is **peer-side**: the model knows a group is complete only via `group_seal(manifest)` with `manifest.members` fully applied.
- Relays MUST NOT buffer for group completeness (RELAY-SPEC 0.2); the model does not simulate relay buffering.
- **All-or-nothing visibility (application):** until seal, consumers MUST treat member ops as not yet group-committed (may still be individually applied for CRDT convergence tests, but group-level ACK is withheld). The conformance model exposes `sealed` for this distinction.
- **Abort:** `manifest.abort = true` marks the group failed; members stay in wal for SEC but `sealed` does not include the group; application-visible tag `GROUP_ABORTED`.

---

## 6. Conformance vectors

| `type` | Checks | Invariants |
|--------|--------|------------|
| `wal-crash` | schedule × crash_point → post-`recover` observable state | I-3, I-13, I-14, DQ-7 |
| `group-seal` | incomplete / complete / abort manifests | I-13 |

Lifecycle: red in `xfail/`, promote on both-runners-green. First twelve vectors promoted (WAL-001..012: crash matrix, group seal/abort, truncate safe/unsafe, apply idempotent). M0e.1 exit still requires the C8 checklist half for the WAL model (delivery is M0e.2).

### Vector shape (`wal-crash`)

```json
{
  "type": "wal-crash",
  "id": "WAL-001",
  "schedule": [
    { "op": "wal_append", "record": { "op_id": "…", "body": "…", "group": null, "author_ts": { "p": 1, "l": 0 } } },
    { "op": "wal_sync" },
    { "op": "state_apply", "op_id": "…" },
    { "op": "hlc_persist", "ts": { "p": 1, "l": 0 } }
  ],
  "crash_after_step": 1,
  "self_author_prefix": optional,
  "expect_after_recover": {
    "wal_len": 0,
    "applied": [],
    "hlc": null,
    "sealed": []
  }
}
```

`crash_after_step` is the 0-based index of the last completed schedule step (−1 = crash before any step).

---

## 7. Out of scope here

- Delivery cursors, anti-replay compaction, L2 durable ack (M0e.2 / H4 / H11).
- Version negotiation policy (M0e.3 / H7).
- SQLite mapping and crash injection (M1 layer 2). The experimental single-op SQLite path is described in [M1-LOCAL.md](M1-LOCAL.md); it does **not** close this layer until named crash points and groups are mapped.

---

*Draft change policy: draft-1 profile — byte-affecting changes re-run the resolution checklist until an explicit freeze Decision Log entry.*
