# M1 local durable store & peer exchange (experimental)

**Status:** implementation notes for the experimental M1 path (`zerodb-storage` / `zerodb-cli`). M1 experimental exit closed 2026-07-25 (`v0.1.0-local`). **Not** a format freeze — all layouts stay draft-1 until an explicit Decision Log freeze.  
**Authority:** descriptive of current code. Normative contracts remain [KERNEL.md](KERNEL.md), [WAL.md](WAL.md), [AUTH.md](AUTH.md), [SCHEMA.md](SCHEMA.md). Honesty leftovers (E1/E4 wording, same-id resurrection, HLC-meta) live under M2a in [LEDGER.md](../plan/LEDGER.md).  
**Evidence:** tag `v0.1.0-local` @ `3d5ae48`; LAN procedure in [M1-LAN-TEST.md](M1-LAN-TEST.md).

This document exists so plan/ review notes do not have to restate settled implementation behavior.

---

## 1. Scope of the experimental slice

| In scope (shipped in code) | Out of scope (still M1 exit or later) |
|----------------------------|----------------------------------------|
| SQLite oplog + materialized nodes/props | WAL layer-2 crash injection / groups (E4, [WAL.md](WAL.md)) |
| Ed25519-signed ops; PeerId = BLAKE3(device pk) | AUTH membership / genesis control plane ([AUTH.md](AUTH.md) → M3b) |
| CRDTs: LWW (string), GCounter, PNCounter, ORSet, Flag; edges + H3 delete machine (E9) | MVRegister, RGA |
| Export/import JSON bundles; file `sync`; TCP `serve`/`pull` (two-way session, protocol v2) | TLS; relay; Merkle wire |
| Ingress validation before materialization | Schema apply / O3 query / TS→IR compiler |
| Single-op atomic commit (append + materialize + HLC); multi-op `atomic_group`; named layer-2 crash points (E4, `tests/e4_crash_matrix.rs`) | WAL layer-1 truncate mapping (post-M1) |

---

## 2. Local identity & genesis (not AUTH genesis)

On `zerodb init`:

- Device seed: 32-byte Ed25519 signing key from **OS CSPRNG** (`getrandom`).
- `PeerId` / author = `BLAKE3(Ed25519 public key)` (32 bytes) — matches KERNEL §2.
- **Experimental local datastore id** (not the AUTH §2 genesis preimage):

  `ds = BLAKE3("zerodb-local-ds-v1" ‖ author ‖ salt)` with random 16-byte salt from OS CSPRNG.

Peers that start empty may **adopt** a remote `ds` only after a nonempty bundle fully prevalidates (see §4). Nonempty datastores reject foreign `ds` with a mismatch error.

Until AUTH genesis is wired into storage, treat local `ds` as disposable experiment identity — not product root authority.

---

## 3. Persistence layout (experimental)

SQLite with `PRAGMA journal_mode=WAL`:

| Table | Role |
|-------|------|
| `meta` | `seed`, `ds`, `salt`, `hlc_p`, `hlc_l`, `storage_format_version` (=1) |
| `ops` | durable oplog: id, author, author_pk, HLC, kind, `body_json`, sig, `wire_json` |
| `nodes` | id, label, deleted |
| `props` | entity, path, crdt, value_json — rebuilt from the op set |

KERNEL names `storage_format_version` as an M1 namespace. The experimental store writes `storage_format_version = 1` at init and backfills it on open of legacy DBs. Layouts may still change without a Decision Log freeze. Do not retain production data on this path.

**HLC durability (DQ-7 backend):** on `open` and after `replay_all`, the HLC high-water is the max `(physical_ms, logical)` over the durable oplog (meta is a cache rewritten when stale).

The Ed25519 **private seed is stored in `meta`** in the clear. Any copy of the SQLite file is full key compromise.

---

## 4. Ingress & materialization rules (hardened)

These rules were accepted for the LAN dataflow slice and are enforced by tests under `zerodb-storage/tests/`:

1. **Prevalidate the complete bundle** before adoption or any write: bundle format, candidate `ds`, every op’s `ds`, version, body shape, author = BLAKE3(author_pk), recomputed OpId (KERNEL preimage via JSON→CBOR body map), Ed25519 signature.
2. **Empty-local adoption** happens only after prevalidation of a **nonempty** remote bundle. Failed or zero-op bootstrap leaves local identity and state unchanged.
3. **One SQLite transaction** for: optional `ds` adopt, all accepted inserts, rematerialization, HLC persist. Semantic or crypto failure rolls back identity and every prefix.
4. **Dedup** by OpId; duplicates count as skipped, not errors.
5. **Remote HLC:** receive rule with **60 s** max forward drift; overflow checked; failed ingress does not poison in-memory HLC.
6. **Property-before-CreateNode:** remote SetProperty ops for unknown nodes are retained in the oplog and rematerialized as **shadow** property state; they MUST NOT invent a visible placeholder node. The CreateNode supplies the label regardless of arrival order, restart, or `replay`. Local mutation against an unknown node is rejected.
7. **`replay`:** atomically wipe and rebuild `nodes`/`props`/`edges` from the oplog only (no orphan materialization).
8. **CreateNode + Tombstone (set-derived):** node presence requires at least one CreateNode; `deleted` is true if **any** Tombstone for that node exists in the op set. Arrival order (including tombstone-before-create) MUST NOT change the normalized projection. Orphan tombstones leave no node row. Covered by `zerodb-storage/tests/r0_stabilize.rs`. **Edges are set-derived the same way** (E9/H3): a kind-4 Tombstone with body `{ edge, tombstone: true }` deletes the edge order-independently; orphan edge tombstones leave no edge row until the CreateEdge arrives. Edge **visibility** stays derived on read: visible iff the edge is not tombstoned AND both endpoints exist and are not deleted — node delete emits **no cascade ops**, late edges to dead endpoints are hidden, and re-creation under a new id resurrects nothing. Properties of tombstoned/hidden entities remain in the oplog but are excluded from materialized visibility and query. Covered by `zerodb-storage/tests/e9_delete_machine.rs`.
9. **Fail-closed `init`:** `LocalStore::init` refuses when `seed`/`ds` meta or any ops/nodes already exist — no silent re-key of identity while retaining old-ds ops. There is no automatic destructive reset in this slice.

Open gaps (not closed by the above): CRDT type pin under concurrent mixed types, causal `deps` buffering, M0 contract amendments (R0.2). (Closed since: layer-2 named crash-injection matrix — `e4_crash_matrix`; H3 edge tombstone + derived visibility — `e9_delete_machine`.)

---

## 5. Operation kinds in this slice

| kind | Meaning | Notes |
|------|---------|--------|
| 1 | CreateNode | body: `node` (16 B hex), `label` |
| 3 | SetProperty | body: `node`, `path`, `crdt`, payload fields |
| 2 | CreateEdge | body: `edge`, `label`, `src`, `dst` (16 B hex ids) — set-derived with edge tombstones |
| 4 | Tombstone (entity ref) | body: exactly one of `node` / `edge` (16 B hex), plus `tombstone: true`. Set-derived delete; node tombstone hides props and (derived) incident edges; edge tombstone deletes only the edge (H3/E9) |

`grp` is present on `atomic_group` members (16 B hex GroupId), absent otherwise.

Wire transport uses a JSON `WireOp` / `ExportBundle { format: 1, datastore_id, ops }` for file and TCP exchange. Signatures bind the KERNEL CBOR envelope derived from the JSON body — dual representation is experimental and may change.

---

## 6. CLI surface (experimental)

| Command | Role |
|---------|------|
| `init`, `info`, `inspect`, `nodes` | local lifecycle |
| `create-node`, `delete-node`, `set`, `get`, `inc`/`dec`, set/flag helpers | local writes |
| `replay` | full rematerialization from oplog |
| `export` / `import` / `sync --peer` | file multi-process exchange |
| `serve --listen` / `pull --from` | two-way TCP set-diff of OpIds; non-loopback needs `--allow-insecure-lan` |

`pull` is a **two-way session** (protocol v2, `zerodb_storage::sync`): the server sends the ops the client lacks, then the client sends back the ops listed in `HelloOk.need` and waits for the server's `OpsAck`; the server ingests them through the same prevalidated `import_bundle` path (§4 rules hold: signature/OpId/ds checks, atomic txn, dedup, empty-adoption only from a nonempty remote). One session converges both peers. The session logic is transport-generic (`Read + Write`); the CLI only opens sockets. Transport is **plaintext**, unauthenticated — trusted private LAN / disposable DBs only. See [M1-LAN-TEST.md](M1-LAN-TEST.md).

**Push capability (still protocol v2 — no version bump):** a client may set `push: true` in its Hello; a push-capable server acks with `push: true` in HelloOk. Both fields are `serde(default)` and unknown fields are ignored, so old↔new peers interoperate: either side omitting or declining the flag yields the plain one-shot session above. When both opt in, the session **stays open** after the OpsAck: either side sends further `OpsMsg` frames whenever new local ops land (each answered by an `OpsAck`), tracking the peer's op set from the initial exchange onward so only missing ops are sent. The wire is unchanged JSON `WireOp` frames — the canonical-CBOR wire remains reserved for protocol v3 per the 2026-07-25 Decision Log entry. Implemented in `zerodb_storage::sync::{serve_push, pull_push}` (store locked only per exchange, never across waits); consumed by the NAPI `serve`/`autoConnect` and the browser JS driver's `connectPush`. The CLI `serve`/`pull` remain one-shot.

Smoke: `powershell -File scripts/test-mvp.ps1`.

---

## 7. Relationship to M1 exit

| Exit artifact | Experimental slice | Remaining |
|---------------|--------------------|-----------|
| E1 restart/replay | done (`e1_e2_acceptance`, `e1_restart_replay`, `e1_kill_clock`) | — (kill-not-shutdown + 1h clock-rollback covered by `tests/e1_kill_clock.rs`) |
| E2 model conflicts | kernel vectors + happy-path multi-peer smokes | store-level equal-ts / equivocation |
| E4 groups/crash | done — `atomic_group` + named layer-2 crash matrix: 5 failpoints (`before-txn`, `after-op-insert`, `before-hlc-persist`, `after-hlc-persist`, `before-commit`) × 3 commit paths (`commit_local`, `atomic_group`, `import_bundle`) in `tests/e4_crash_matrix.rs`, mapped to WAL.md §3 points | — |
| E9 delete | done — set-derived node **and edge** tombstones, derived visibility, no cascade ops, late-edge + permutation + replay identity + query exclusion in `tests/e9_delete_machine.rs` | edge properties (kind 3 on edges) remain out of this slice |
| Schema/query CLI | `schema-apply`, `query`, edges | interactive repl optional; full CBOR SchemaEpoch later |
| Format freeze | `storage_format_version=1` written | Decision Log freeze still required |

Live work tracking: [plan/LEDGER.md](../plan/LEDGER.md). Historical July reviews: [plan/archive/](../plan/archive/).

---

*When a behavior here conflicts with KERNEL/WAL/AUTH after those documents are amended, the normative contracts win and this file MUST be updated.*
