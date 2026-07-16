# ZeroDB — Path-to-MVP Execution Plan

**Date:** 2026-07-16
**Status:** active delivery plan (pre-MVP)
**Authority:** This is the delivery/tracking plan. [SPEC §10](../doc/SPEC.md) remains the normative roadmap and [ISSUES.md](../doc/ISSUES.md) the normative issue ledger. Where this plan and SPEC conflict, SPEC wins until amended; amendments this plan calls for are themselves tracked work items below.

---

## 1. Status & verdict

Two independent reviews — [FINDINGS.GROK.md](FINDINGS.GROK.md) (2026-07-15) and [FINDINGS.CODEX.md](FINDINGS.CODEX.md) (2026-07-16) — agree:

- The M0→M6 sequencing thesis is sound and should be preserved.
- The project is **ready for M0 contract/specification work and experimental algorithm work only**.
- It is **not** ready for a format freeze, product implementation, or an MVP release.

**Release meaning in this plan:**

| Term | Definition |
|------|------------|
| **MVP** | `v0.1.0-local` — M1 exit: offline single-peer Rust core + SQLite + CLI |
| **First shippable product** | `v0.1.0` — M3c exit: secure multi-peer sync with offline catch-up |

**Current blockers to starting the MVP path** (detail in §3–§4):

1. No package owns executable CRDT/HLC state-machine semantics (CX-01).
2. Datastore membership is relay-checked only; no end-to-end authorization model (CX-02).
3. M0/M1 gate language is circular and non-auditable (CX-03).
4. M3/v0.1 as planned has no guaranteed offline catch-up path (CX-04).
5. Composite M0 can authorize a freeze before format-shaping decisions (encryption envelope, blobs) exist (CX-05).
6. Acceptance artifacts (EXEMPLAR scenarios, INVARIANTS, `conformance/`, CI) do not exist (CX-06).
7. HLC has no durable restart/rollback contract (CX-07).
8. The workspace does not compile under `cargo test --workspace` (A-01) and fails `cargo fmt --check`.

---

## 2. Review disposition

`FINDINGS.GROK.md` is **historical**: its C-P1/C-P2/C-P3 headline items were substantially resolved by commit `3aba801` (M0a–M0f split, pre-M0 implementation policy, release labels). Its still-open items are superseded by the newer Codex findings mapped below (per Codex §7 disposition table).

| Finding | Planned work item |
|---------|-------------------|
| CX-01 semantic kernel unowned | §4 M0a: semantic & operation kernel sub-gate |
| CX-02 datastore authority missing | §4 M0d: end-to-end datastore control plane |
| CX-03 M0/M1 gate circularity | §3 P0-6: model-gate vs backend-gate language |
| CX-04 no offline catch-up in v0.1 | §5 M3a: L2 reference relay required before `v0.1.0` |
| CX-05 freeze before format decisions | §4 M0a/M0b: envelope, extension, blob decisions pre-freeze; DQ-5/DQ-6 |
| CX-06 acceptance artifacts missing | §3 P0-2/P0-3/P0-5 |
| CX-07 HLC durability | §4 M0a semantic kernel + §5 M1 atomic boundary; DQ-7/DQ-8 |
| HX-01 relay signaling impersonation | §5 M3b signed peer handshake; narrow RELAY §12 claim in P0-7 hygiene pass |
| HX-02 contradictory relay validation MUSTs | §4 M0d (C5 unresolved-author outcome) |
| HX-03 no resource-safe decoding limits | §4 M0e.3 restricted CBOR profile |
| HX-04 HLC ranges as delivery cursors | §4 M0e.2 delivery/resume cursor semantics |
| HX-05 key rotation vs `PeerId` identity | §4 M0d principal/device model; DQ-1 |
| HX-06 unique indexes before semantics | §4 M0b: reject `unique` in v0.1 profile; DQ-10 |
| HX-07 no independent second peer | §5 M2/M3c: independent TS model runner → wire peer |
| HX-08 M3/M5 big-bang | §5 M3a/b/c and M5a/b/c splits |
| HX-09 no delivery-control model | §3 P0-4 delivery ledger |
| HX-10 resolved-issue traceability | §3 P0-4: `status: resolved` records kept in ledger |
| HX-11 version-namespace ambiguity | §4: version registry precedes M0a fixtures |
| HX-12 receiver limits never advertised | §4 M0e.3 symmetric limits or peer capability message |
| A-01 workspace doesn't compile | §3 P0-1 |
| A-02 `HLC::recv` overflow bug | §3 P0-1 (fix with test) then M0a kernel vectors |
| A-03 LWW equal-timestamp equivocation | §4 M0a kernel; DQ-8 |
| A-04 experimental shapes vs spec | remains experimental per SPEC §10 policy; regenerate from M0a artifacts |
| A-05 flaky/incomplete tests | §3 P0-1 baseline + P0-5 CI |
| D-01…D-16 cross-doc corrections | §3 P0-7 single hygiene pass over SPEC/RELAY/ISSUES/README |
| GROK C-P4/C-P5/H-P4/H-P6/H-P7 (still open) | absorbed by CX-06, HX-08, HX-09, P0-2, CX-04 respectively |

---

## 3. P0 — Planning & conformance readiness

**Outcome:** for every planned gate, a contributor can identify the contract owner, the failing test, and the evidence needed to close it. P0 precedes any M0 package exit claim.

| ID | Work item | Exit evidence |
|----|-----------|---------------|
| P0-1 | **Green baseline.** Fix A-01 (uuid `v4` feature, `serde_json` dev-dep), A-02 (recv overflow, with test), fmt; add restart/rollback and receive-overflow tests | `cargo test --workspace` and `cargo fmt --all -- --check` pass |
| P0-2 | **INVARIANTS.md populated.** Invariant IDs with falsification conditions (SEC, HLC monotonicity, signature meaning, membership meaning, no-GC-without-frontier, …) | Each invariant has an ID cited by ≥1 scenario or fixture |
| P0-3 | **EXEMPLAR rewritten** as versioned Given/When/Then scenarios (E1 single-peer CRUD+restart, E2 conflict merges, E3 membership sharing+denial, E4 encrypted properties, E5 partition/rejoin, …) with topology, fault schedule, expected state, and milestone/invariant traceability. Sharing defined as datastore-per-list/item for v0.1; admin/entity-ACL items marked post-v0.1 | Every milestone exit gate cites concrete scenario IDs |
| P0-4 | **Delivery ledger** (`plan/LEDGER.md`): ID, DRI, status, dependencies, effort band, target, entry gate, exit evidence, risks, release. Resolved issues retain `status: resolved` records with evidence + approver (HX-10) | Critical path derived from ledger, not document order |
| P0-5 | **`conformance/` + CI.** Rust and independent TypeScript model runners; default lane green; newly activated failing fixtures in an expected-failure lane until promoted at their gate | CI runs on every push; fixture promotion policy written |
| P0-6 | **Gate language fix (CX-03).** SPEC §10 amended to separate *M0 contract-model conformance* (executable models green) from *M1 backend conformance* (SQLite crash tests green) | No milestone both requires and defers the same artifact |
| P0-7 | **Doc hygiene pass.** Apply D-01…D-16 (LiveOp→OPS, snapshot milestone, resumption text, TCP scope, Phase-3 wording, O5 log entry, C5/C8 stale facts, censorship claim, encryption terminology, crate version/`publish=false`, README Grok link → `plan/FINDINGS.GROK.md`, GROK historical banner, `_meta.signature`, L2 retention, §9.2 convergence claim) | One commit; each D-item checked off |

---

## 4. M0 — Executable contracts (revised packages)

Preserves the SPEC §10 M0a–M0f structure with the Codex corrections. **Version registry skeleton precedes M0a fixtures** (HX-11): distinct `operation_format_version`, `schema_epoch`, `snapshot_format_version`, `storage_format_version`, `relay_protocol_version`.

| Package | Contracts | Required outcome before exit |
|---------|-----------|------------------------------|
| **M0a — Semantic & operation kernel** | C1; H1 contract; HX-11 | Operation algebra **plus executable state machines** for HLC and every M1 CRDT: state/op algebra, preconditions, causal context, duplicate/out-of-order behavior, equal-timestamp/equivocation rule, HLC local/receive/overflow/restart/clock-rollback transitions. Deterministic CBOR, preimages, ID encodings. Encrypted-value envelope/AAD/key-reference bytes and large-value extension framing decided (CX-05). Permutation/replay/boundary vectors green in both model runners. |
| **M0b — Schema/query profile** | C2, O2, O3 | Canonical IR, epochs, migration DSL, minimal query grammar. Encryption annotations. **`unique` rejected from the v0.1 profile** (HX-06) unless DQ-10 decides otherwise. Extension/version rules. |
| **M0c — Merkle/sync model** | C3 | Canonical tree, adversarial timestamps, traversal state machine, mismatch-recovery transcripts, checkpoint hooks. |
| **M0d — Datastore, identity & authorization** | C4, C5; H6 contract | End-to-end control plane: datastore genesis + `DatastoreId` derivation, stable principal/device/key model (HX-05), capability issuer/subject/scopes/epoch/expiry/revocation, root authority for schema/membership/keys/checkpoints, historical authorization, **peer-side author verification independent of relays** (CX-02), one deterministic unresolved-author outcome (HX-02), signed peer handshake contract. |
| **M0e — Delivery, durability, versions & limits** | C8, H4, H5, H7, H9, H11, O6 | Independently green sub-gates: **M0e.1** group/WAL reference model (executable, no SQLite required); **M0e.2** delivery/ack/resume state machine with append-sequence or frontier cursors (HX-04); **M0e.3** resource-safe CBOR decoding profile (HX-03), compatibility registry, symmetric receive limits (HX-12). |
| **M0f — Frontiers, checkpoints & snapshots** | C7, O7 | Executable retirement/reconnect/late-op/checkpoint/root-comparison models. **No physical GC.** |

**Composite gate:** all package model suites green in Rust **and** the independent TypeScript model runner; cross-package fixtures cover signed encrypted operations through authorization, grouping, Merkle sync, checkpoint identity, and replay. Production backends not required. Freezes name a **versioned profile**, not an unlimited promise.

```text
P0 → version registry → M0a → { M0b, M0c, M0d, M0e }
M0b + M0d → encrypted-operation integration fixtures
M0c + M0e.2 → M0f
M0a–M0f composite → M1 → M2 → M3a → M3b → M3c (v0.1.0)
                                      │       │
                                      └→ M4a  └→ M4b → M5 GA program
```

---

## 5. Post-M0 path

### M1 — Local durable alpha (**MVP**, `v0.1.0-local`)

Rust + SQLite + CLI subset. Atomic oplog/state/**HLC** transaction with crash injection at every named boundary (CX-07). Deterministic replay and delete semantics (H3 release-blocking). M1-profile schema/query only — reject `unique`, Richtext, ACLs, sync-only controls. Exit: green storage contract tests + Exemplar local scenarios (E1/E2).

### M2 — SDK alpha (`v0.1.0-sdk`)

Node/NAPI product API, lifecycle, subscription/backpressure/error contracts. The independent TS **model runner stays separate** from the Rust-backed SDK and evolves toward the M3c wire peer (HX-07).

### M3 — split into three gates (resolves CX-04, HX-08)

| Gate | Scope | Release |
|------|-------|---------|
| **M3a — durable convergence** | **L2 reference relay** (durable persistence, receipt vs durable ack, full-oplog catch-up, GC off), Merkle/delta wire, resume cursors, loss/reorder/partition/rejoin, three-peer offline catch-up, crash/restart. Pre-provisioned signed test identities only. | internal |
| **M3b — security** | Author keys, peer-verifiable datastore authorization, signed peer handshake (HX-01), M0-frozen encryption envelope + key lifecycle, clock quarantine (H1), resource limits, malicious relay/peer negatives. | internal |
| **M3c — interop & release** | Independent TypeScript **wire peer**, version/upgrade matrix, packaging, support profile, Exemplar sharing + private-data scenarios (E3/E4/E5). | **`v0.1.0`** |

If the L2 relay cannot ship, M3 is renamed an online-sync preview and is **not** the first offline-first release.

### M4 — two tracks

**M4a platform:** browser storage, WASM (O4 budget), WebRTC, React. **M4b evolution:** cross-peer schema migration, snapshots, adjacent-version rollback/upgrade, large-payload implementation under the M0-approved extension decision (O1).

### M5 — focused GA program

**M5a operability:** backup/restore drills, observability/SLOs, packaging. **M5b lifecycle safety:** safe GC + peer lifecycle (C7 impl), rolling upgrades. **M5c release assurance:** fuzz/soak/failure injection, severity-based security sign-off (not "all findings closed").

### Parallel tracks (not GA gates)

Unique-index semantics (or a durable won't-do decision), Richtext (after O1 + M4b), Lean proofs tied to the M0 semantic kernel, comparative benchmarks.

### M6 — portfolio

Mobile C-ABI bindings, plugins, entity-ACL successor design (C6), hosted relay, admin UI — independently approved epics after compatibility stability.

---

## 6. Decision queue

From Codex §9. Owner = TBD until the delivery ledger (P0-4) assigns DRIs. A decision closes only via the SPEC §10 approved-resolution checklist.

| ID | Decision | Blocks | Status |
|----|----------|--------|--------|
| DQ-1 | Identity: key, device, or stable principal containing devices? | M0d, HX-05 | open |
| DQ-2 | Datastore genesis + root authority for members/schemas/keys/checkpoints? | M0d | open |
| DQ-3 | Per-operation author-membership verification + historical authorization after revocation? | M0d | open |
| DQ-4 | Executable model that closes C8 in M0 without the M1 SQLite backend? | M0e.1, P0-6 | open |
| DQ-5 | Encryption: property-level or operation-level; which envelope/context bytes freeze in M0? | M0a/M0b, CX-05 | open |
| DQ-6 | Extension/blob strategy so O1 can't invalidate M0a? | M0a | open |
| DQ-7 | Durable HLC state across restart/restore/rollback/cloned keys? | M0a, M1 | open |
| DQ-8 | Equal HLC timestamps with unequal signed ops: reject as equivocation or canonical tie-break? | M0a | open |
| DQ-9 | Is L2 durable catch-up mandatory for v0.1? (Plan assumes **yes** — M3a) | M3a | plan default: yes |
| DQ-10 | Unique indexes: removed from v0.1 profile or later feature gate? (Plan assumes **removed**) | M0b | plan default: removed |
| DQ-11 | Who approves normative resolutions; where do resolved-issue records live? (Plan default: delivery ledger, P0-4) | P0-4 | plan default set |
| DQ-12 | Team/capacity assumption, DRI set, effort bands for the critical path? | P0-4 | open |

DQ-1…DQ-8 gate the M0 packages they shape. DQ-9…DQ-12 gate release execution, not individual M0 packages.

---

## 7. Ground rules (carried forward, normative in SPEC §10 / ISSUES.md)

- **No wire or persistent format freeze before composite M0 exit**; freezes name a versioned profile.
- **GC disabled** until C7 partition/rejoin, forgotten-peer, late-op, and restore tests pass (M5b).
- **Pre-M0 implementation policy** applies: `zerodb-core`/`zerodb-storage` are experimental; their types are not normative.
- **Approved-resolution checklist** (SPEC §10) is the only way an issue closes: normative prose + machine-readable artifact + golden positive/negative vectors + Decision Log entry.
- Lean proofs do not gate M0 or v0.1.

---

## 8. Immediate next actions (ordered)

1. **P0-1** — restore the green baseline (`cargo test`, `cargo fmt`); fix A-02 with a receive-overflow test.
2. **P0-7** — doc hygiene pass (D-01…D-16), including README link fix and GROK historical banner.
3. **P0-2 / P0-3** — populate INVARIANTS.md; rewrite EXEMPLAR as scenario IDs.
4. **P0-4 / P0-5** — delivery ledger; `conformance/` skeleton + CI.
5. **P0-6** — amend SPEC §10 gate language (model vs backend).
6. **DQ-1…DQ-8** — work the M0 decision queue; record outcomes via the resolution checklist.
7. **Version registry → M0a** — begin the semantic & operation kernel with red fixtures.

No MVP implementation work (M1) starts before the composite-M0 model gates it depends on are green.
