# ZeroDB — Path-to-MVP Execution Plan

**Date:** 2026-08-28 (refreshed after perf/DX Stage 0+1)
**Status:** active — composite M0, M1, M2, experimental **M3a durable convergence**, and M3b E5–E8 live evidence (membership, authenticity, clock quarantine, encrypted notes) are done. Current: **perf/DX Stage 0+1**, then **M3c**. M3b remainder is pinned (not closed). Formats remain draft/unfrozen. **Not** M3b exit.
**Authority:** delivery/tracking only. [SPEC §10](../doc/SPEC.md) is the normative roadmap; [ISSUES.md](../doc/ISSUES.md) the issue ledger; [LEDGER.md](LEDGER.md) the live work tracker. On conflict, SPEC wins.

Completed P0 readiness, M0 packages, M1 experimental exit, and dispositioned review findings are recorded in the [ISSUES Decision Log](../doc/ISSUES.md) and are **not** re-listed here. July 2026 review files live in [plan/archive/](archive/).

---

## 1. Release meaning

| Term | Definition |
|------|------------|
| **MVP** | `v0.1.0-local` — M1 exit: offline single-peer Rust core + SQLite + CLI |
| **First shippable product** | `v0.1.0` — M3c exit: secure multi-peer sync with offline catch-up |

Roadmap M0–M6 (including M3a/b/c, M4a/b, M5a/b/c) is normative in [SPEC §10](../doc/SPEC.md).

---

## 2. Where we are

| Gate | Status |
|------|--------|
| P0 readiness | **done** |
| Composite M0 (contract-model, draft-1, 109 vectors) | **done** 2026-07-18 — CX-03..06 amended 2026-08-14; CX-08 dual-root direction in RELAY 0.2.1-draft |
| Format freeze | **not done** — draft-1, unfrozen until an explicit Decision Log freeze names a versioned profile |
| Experimental M1 local store + file/TCP peer exchange | **done** (experimental) — [M1-LOCAL.md](../doc/M1-LOCAL.md) |
| M1 exit (`v0.1.0-local`) | **done** 2026-07-25 — experimental format |
| M2 Node/NAPI vertical | **done** 2026-08-14 — experimental `v0.1.0-sdk`; not SPEC-complete M2 |
| M2a stabilize + schema | **done** — [LEDGER.md](LEDGER.md) |
| M3a L2 relay | **done** 2026-08-15 — process + client, frozen-snapshot subtree/leaf Merkle walk, and full 1,000-write EXEMPLAR E3 with B hard-abort/reopen and C relay-only catch-up. |

---

## 3. Ground rules

- **No wire or persistent format freeze** without a Decision Log entry naming a versioned profile (composite M0 closed draft-1 only).
- **GC disabled** until C7 safety tests pass (M5b).
- **`zerodb-core` / `zerodb-storage` experimental** until freeze; types are not normative.
- **Approved-resolution checklist** (SPEC §10) is the only way a C/H issue closes.
- Lean proofs do not gate M0 or v0.1.
- Experimental multi-process TCP is **non-gating** for M1 exit unless SPEC/LEDGER say otherwise ([M1-LOCAL.md](../doc/M1-LOCAL.md)).

---

## 4. Open decisions

| ID | Decision | Blocks | Status |
|----|----------|--------|--------|
| DQ-9 | L2 durable catch-up mandatory for `v0.1.0`? | M3a | plan default **yes** (in SPEC §10) |
| DQ-11 | Approver + records location | process | plan default: **LEDGER** + ISSUES Decision Log |
| DQ-12 | Capacity / effort bands | schedule | **open** |

Resolved DQ-1..DQ-8, DQ-10 live in AUTH / KERNEL / SCHEMA / WAL — not tracked here.

---

## 5. Immediate next actions (ordered)

**Current work package:** **perf/DX Stage 0+1** (then **M3c**). E6 is done; M3b itself stays open.

1. ~~Golden two-language frames~~ **done**.
2. ~~L2 relay process (`zerodb-relay`)~~ **done** (`9603a6c`).
3. ~~RELAY 0.2.2 client (`connectRelay`)~~ **done** (`73a6b6d`).
4. ~~E2-live / E3-lite on that client~~ **done** (`relay_client` + `m3a-relay.test.mjs`). Concurrent LWW/ORSet/Flag/PNCounter; 3-peer C-offline catch-up; B sqlite close/reopen (not process death); resume `received=0`.
5. ~~Frozen-snapshot Merkle subtree/leaf walk~~ **done** (`merkle-walk-v1`).
6. ~~Full 1,000-write EXEMPLAR E3~~ **done**: 3 peers, C offline, B hard-abort/reopen, relay-only catch-up, equal roots, resume no-redelivery.
7. ~~Relay signature / OpId / datastore admission (M3b-sig)~~ **done** (`admit_experimental_op`, `m3b_admission`).
8. ~~AUTH membership + executable E5~~ **done** (first-class grant/revoke ops, peer AUTH §4 on persist/import/ingest, honest-relay REJECT/AUTHZ, colluding-relay peer reject; `e5_membership` in storage + relay).
9. ~~E7 remainder (forged/replay + colluding)~~ **done** (peer `AUTH_SIG_INVALID` / `Duplicate`; honest relay `REJECT/SIG` + `DUPLICATE`; colluding relay forwards forged/tampered; wipe-dedup replay has no double effect; `e7_forged_replay` in storage + relay).
10. ~~E8 clock quarantine (H1)~~ **done** (peer `CLOCK_DRIFT` hold + release; C +30d LWW does not silently win on A/B; after the window A/B/C converge; honest/colluding relays persist/forward; `e8_clock_quarantine` in storage + relay).
11. ~~E6 encrypted private notes (I-10)~~ **done** (KERNEL §7 seal/open on schema-encrypted LWW; `KeyRecord` `kr = 2` wrap/rotate; R/C blind including decrypt oracle; SQLite key persist; `e6_encrypted_notes` in storage + relay).
12. **perf/DX Stage 0+1** — current: fixtures/phase counters; advertised payload/batch limit enforcement; import≡replay then drop redundant push `replay_all`; batched relay inserts; request-id NAPI drain; clone-free chunking; order indexes + bulk props; one export in relay client sync. See [PERF.md](PERF.md).
13. **M3c** — interop TS wire peer + release. Include signed `SchemaEpoch` here so encrypted notes stop being a two-step ritual (apply schema, then write). **Do not implement SchemaEpoch in the Stage 0+1 PR.**

**Shipped already (do not re-open as next work):** M1 experimental exit; M2a; M2-parity; `v0.1.0-sdk`; M3a relay/client, E2-live, Merkle walk, and full E3. Equal-ts E2 remains covered at model level; it is not an M3a blocker. E5–E8 live evidence is on; that is **not** M3b exit.

**Deferred with triggers:**
- **perf Stage 2** (targeted projections: derived `op_targets`, AUTH control projection, single-pass replay, persisted CRDT accumulators) — trigger: Stage 0 fixtures still show hot-write/replay dominated by broad oplog scans after Stage 1.
- **perf Stage 3** (bounded reconciliation: replace full OpId manifests, missing-only relay upload, compact Merkle snapshot cache) — trigger: equal-replica / one-op-delta wire bytes still scale with full history after Stage 1.
- **H10 leftovers** (offline revoke, two-key wrap, wrap-body freeze).
- Handshake/TLS/resource hardening **beyond advertised-limit enforcement**; two-key principal/device split.
- M2-crdts (until an app needs MVRegister/RGA/LWWMap); E11 budgets; query-scoped subscribe; interactive `repl`; CBOR wire (protocol v3); OPFS/sqlite-wasm; WebRTC.

Format freeze remains a separate Decision Log act.

Live rows: [LEDGER.md](LEDGER.md). Historical July reviews: [plan/archive/](archive/).
