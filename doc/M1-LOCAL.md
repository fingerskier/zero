# M1 local durable store & peer exchange (experimental)

**Status:** implementation notes for the experimental M1 path (`zerodb-storage` / `zerodb-cli`). **Not** a format freeze and **not** M1 exit (`v0.1.0-local`).  
**Authority:** descriptive of current code. Normative contracts remain [KERNEL.md](KERNEL.md), [WAL.md](WAL.md), [AUTH.md](AUTH.md), [SCHEMA.md](SCHEMA.md). Exit criteria remain [SPEC §10](SPEC.md) M1 + [EXEMPLAR.md](EXEMPLAR.md) E1/E2/E4/E9.  
**Evidence:** commits `1ef16d6` (local MVP CLI + TCP), `f9cc660` (peer dataflow hardening); LAN procedure in [M1-LAN-TEST.md](M1-LAN-TEST.md).

This document exists so plan/ review notes do not have to restate settled implementation behavior.

---

## 1. Scope of the experimental slice

| In scope (shipped in code) | Out of scope (still M1 exit or later) |
|----------------------------|----------------------------------------|
| SQLite oplog + materialized nodes/props | WAL layer-2 crash injection / groups (E4, [WAL.md](WAL.md)) |
| Ed25519-signed ops; PeerId = BLAKE3(device pk) | AUTH membership / genesis control plane ([AUTH.md](AUTH.md) → M3b) |
| CRDTs: LWW (string), GCounter, PNCounter, ORSet, Flag | MVRegister, RGA, edges, full H3 delete machine (E9) |
| Export/import JSON bundles; file `sync`; TCP `serve`/`pull` | Bidirectional session; TLS; relay; Merkle wire |
| Ingress validation before materialization | Schema apply / O3 query / TS→IR compiler |
| Single-op atomic commit (append + materialize + HLC) | Multi-op group seal / named crash points |

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
8. **CreateNode + Tombstone (set-derived):** node presence requires at least one CreateNode; `deleted` is true if **any** Tombstone for that node exists in the op set. Arrival order (including tombstone-before-create) MUST NOT change the normalized projection. Orphan tombstones leave no node row. Covered by `zerodb-storage/tests/r0_stabilize.rs`.
9. **Fail-closed `init`:** `LocalStore::init` refuses when `seed`/`ds` meta or any ops/nodes already exist — no silent re-key of identity while retaining old-ds ops. There is no automatic destructive reset in this slice.

Open gaps (not closed by the above): CRDT type pin under concurrent mixed types, causal `deps` buffering, WAL named crash-injection matrix (partial: `atomic_group` exists), full H3 edge-tombstone/prop model, M0 contract amendments (R0.2).

---

## 5. Operation kinds in this slice

| kind | Meaning | Notes |
|------|---------|--------|
| 1 | CreateNode | body: `node` (16 B hex), `label` |
| 3 | SetProperty | body: `node`, `path`, `crdt`, payload fields |
| 4 | Tombstone (node) | body: `node`, `tombstone: true` — hides props locally; **not** full H3/E9 |

`grp` is always absent. Edges (kind 2) are not implemented.

Wire transport uses a JSON `WireOp` / `ExportBundle { format: 1, datastore_id, ops }` for file and TCP exchange. Signatures bind the KERNEL CBOR envelope derived from the JSON body — dual representation is experimental and may change.

---

## 6. CLI surface (experimental)

| Command | Role |
|---------|------|
| `init`, `info`, `inspect`, `nodes` | local lifecycle |
| `create-node`, `delete-node`, `set`, `get`, `inc`/`dec`, set/flag helpers | local writes |
| `replay` | full rematerialization from oplog |
| `export` / `import` / `sync --peer` | file multi-process exchange |
| `serve --listen` / `pull --from` | one-way TCP set-diff of OpIds; non-loopback needs `--allow-insecure-lan` |

`pull` is **server → client only**. `HelloOk.need` is computed by the server but unused by the client; reverse roles for two-way convergence. Transport is **plaintext**, unauthenticated — trusted private LAN / disposable DBs only. See [M1-LAN-TEST.md](M1-LAN-TEST.md).

Smoke: `powershell -File scripts/test-mvp.ps1`.

---

## 7. Relationship to M1 exit

| Exit artifact | Experimental slice | Remaining |
|---------------|--------------------|-----------|
| E1 restart/replay | partial (open/replay tests; not full exemplar) | clock-rollback, larger load, kill-not-shutdown |
| E2 model conflicts | kernel vectors + happy-path multi-peer smokes | store-level equal-ts / equivocation |
| E4 groups/crash | `atomic_group` + atomic mid-batch rollback | fine-grained WAL named crash points (append/sync/apply) optional |
| E9 delete | node tombstone set-derived + edge derived visibility | no cascade edge ops; late edges hidden; no edge tombstone props |
| Schema/query CLI | `schema-apply`, `query`, edges | interactive repl optional; full CBOR SchemaEpoch later |
| Format freeze | `storage_format_version=1` written | Decision Log freeze still required |

Live work tracking: [plan/LEDGER.md](../plan/LEDGER.md). Open review notes: [plan/FINDINGS.GROK.md](../plan/FINDINGS.GROK.md).

---

*When a behavior here conflicts with KERNEL/WAL/AUTH after those documents are amended, the normative contracts win and this file MUST be updated.*
