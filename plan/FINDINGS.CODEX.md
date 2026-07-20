# ZeroDB Current-State & Plan Review — FINDINGS.CODEX

**Baseline:** `3b269e8` (`main`, equal to `origin/main`)

**Review scope:** roadmap and delivery state; normative contracts; current Rust/SQLite/CLI implementation; Node/NAPI and TS→IR scaffolds; conformance harness; CI; current tests.

**Review posture:** findings are ordered by release/gate impact. This is a review, not an implementation or a format-freeze decision.

---

## 1. Executive verdict

ZeroDB has advanced substantially since the prior Codex review. The repository now has a real two-language conformance corpus, a working signed SQLite/CLI prototype, guarded LAN binding, stronger ingress validation, executable E1/E2 slices, local atomic batches, edges and queries, plus an initial NAPI package. The normal workspace, conformance, tool, SDK, and formatting checks are green on this machine.

The current plan nevertheless **overstates readiness**:

- Composite M0 should be treated as **provisionally closed, with targeted contracts reopened**, not as ready for a byte or persistent-format freeze. The encrypted-value AAD is circular; the frontier cannot decide its own late-op rule from its encoded data; the resume cursor cannot represent the receiver state used by its model; and the WAL model does not enforce several normative rules.
- M1 is **not one formal pass away from exit**. A confirmed arrival-order defect makes node delete/create visibility diverge for the same operation set; `init` can silently re-key an existing nonempty database; canonical schema epochs/dependencies are absent; and E1/E4 evidence is materially weaker than the normative acceptance scenarios.
- M2 is a useful spike, but it is not yet a reproducible SDK vertical: clean CI does not build/test the native package or TS→IR tool, the package targets Windows only, and the public surface omits the M2-defining query/mutate/subscribe/parity path.
- M3 should not begin implementation against the current relay draft. The relay document still carries pre-M0 placeholders and does not define how an L2 relay and an honest peer can compare Merkle roots when they retain different authorization sets.

**Recommendation:** pause formal M1 exit and any freeze. Run a short **R0 contract-and-state stabilization gate**, then complete M1 against an explicit, evidence-bearing checklist. M2 experimentation may continue in parallel, but M2 exit and API commitments should remain blocked on the stabilized M1 surface.

### Scorecard

| Area | Current assessment | Why |
|------|--------------------|-----|
| Architecture / sequencing | **Good direction** | M0 → M1 → M2 → M3a/b/c remains sensible |
| Composite M0 | **Targeted reopen required** | M0a envelope; M0e WAL/delivery; M0f frontier/snapshot have implementability gaps |
| M1 implementation | **Substantial prototype, not exit-ready** | Real CRUD/CRDT/storage progress, but confirmed SEC and destructive-init defects plus missing normative scope |
| M1 evidence | **Partial** | Green integration tests do not execute kill/crash-point/rollback scenarios as specified |
| M2 | **Spike/scaffold** | Three local native smoke tests; no clean-CI/package/platform/parity evidence |
| M3 readiness | **Not implementation-ready** | Relay wire draft and accepted-set semantics remain unresolved |
| Documentation / tracking | **Drifting** | PLAN/LEDGER, SPEC, M1-LOCAL, GROK, registry, and relay draft disagree |
| CI / quality | **Good baseline, incomplete gate** | Tests/fmt pass; Node/tool jobs absent; clippy fails under `-D warnings` |

---

## 2. Progress worth preserving

1. **A credible conformance base exists.** There are 103 required vectors, a JS runner, Rust package harnesses, a registry, and CI required-lane execution (`conformance/README.md:1-28`, `.github/workflows/ci.yml:21-39`).
2. **The local store is no longer a toy scaffold.** Signed operations, five M1 CRDTs, SQLite materialization, full-bundle prevalidation, atomic import, datastore adoption safeguards, HLC drift checking, replay, edges, query evaluation, and local schema pinning are present (`zerodb-storage/src/lib.rs:130-1043`).
3. **Several earlier high-risk bugs were addressed well.** OS CSPRNG seed generation, loopback-default serving, oplog-based HLC recovery, equal-timestamp LWW tests, and transactional prefix rollback are all represented by tests (`zerodb-storage/tests/m1_wave1.rs:77-213`, `zerodb-cli/tests/serve_bind.rs:1-116`, `zerodb-storage/tests/e1_e2_acceptance.rs:201-354`, `zerodb-storage/tests/m1_remainders.rs:87-121`).
4. **The roadmap names meaningful release slices.** `v0.1.0-local`, `v0.1.0-sdk`, and secure/offline-capable `v0.1.0` remain a much clearer sequence than a single big-bang release (`plan/PLAN.md:12-18`, `doc/SPEC.md:897-911`).
5. **Dangerous experimental transport is labeled.** Non-loopback serve requires an explicit flag and the docs warn that the path is plaintext, unauthenticated, and disposable (`README.md:75-79`, `doc/M1-LAN-TEST.md:1-7`).
6. **The plan correctly keeps GC disabled and separates contract models from production backends.** Those boundaries should remain.

These gains justify tightening the gates rather than replacing the overall architecture.

---

## 3. Critical findings

### CX-01 — M1 violates SEC for tombstone-before-create arrival

`apply_wire` handles a tombstone as an `UPDATE` against the current materialized row; if the node has not arrived, it updates zero rows. A later `CreateNode` inserts a live node (`zerodb-storage/src/lib.rs:1227-1235`, `zerodb-storage/src/lib.rs:1255-1259`). Replay similarly executes imperative create/update steps in oplog order (`zerodb-storage/src/lib.rs:818-845`).

A review reproduction imported the same two signed operations in opposite orders:

- `[CreateNode, Tombstone]` → node `deleted: true`
- `[Tombstone, CreateNode]` → node `deleted: false`

Both stores contained the same two OpIds. This directly falsifies I-1 and I-16 and means the ledger cannot mark E9 complete (`doc/INVARIANTS.md:10-14`, `doc/INVARIANTS.md:93-98`, `plan/LEDGER.md:59`).

**Required correction:** materialize entity existence/label/tombstone from the full set with a specified CRDT/state machine, including tombstone-before-create, duplicate/conflicting creates, same-author equivocation, late edges, and replay. Add permutation tests at the storage boundary before any M1 exit claim.

### CX-02 — `init` silently corrupts an existing nonempty datastore

`LocalStore::init` opens whatever path is supplied and unconditionally overwrites `seed`, `ds`, `salt`, and HLC metadata; it never checks for prior initialization or operations (`zerodb-storage/src/lib.rs:160-184`). The CLI and NAPI expose this directly as `init` (`zerodb-cli/src/main.rs:229-235`, `zerodb-napi/src/lib.rs:41-50`).

A review reproduction initialized a DB, wrote two operations, and ran `zerodb init` again. The database retained the old operations but received a new datastore ID and peer key. Export then produced a bundle whose top-level datastore ID differed from every operation's signed `ds`.

This is a destructive-command guard failure and can make an apparently healthy local database non-importable.

**Required correction:** fail closed when an initialized/nonempty target exists. If reset is needed, require a separate explicit destructive command/flag with backup guidance and tests proving no overwrite on the default path.

### CX-03 — The M0a encrypted-value envelope has a construction cycle

KERNEL makes `OpId` the hash of the operation body, which contains the encrypted envelope (`doc/KERNEL.md:104-109`). It also requires the envelope AAD to contain that same final `OpId` (`doc/KERNEL.md:154-162`). Therefore:

1. ciphertext/tag requires `OpId`;
2. `OpId` requires ciphertext/tag.

The vectors avoid the cycle by supplying an arbitrary external `op_id` to `ValueContext`; they never construct and sign a complete operation containing that envelope (`zerodb-core/tests/conformance_envelope.rs:23-33`, `zerodb-core/tests/conformance_envelope.rs:63-82`).

This blocks E6 and any format freeze despite the M0a-closed label.

**Required correction:** choose a non-circular binding, such as a pre-encryption slot/context hash that excludes ciphertext, or a two-level content structure with a separately hashed encrypted payload. Add an end-to-end golden vector: logical plaintext → envelope → complete operation bytes → OpId/signature → decrypt.

### CX-04 — M0f's frontier cannot implement its own late-op and snapshot claims

The encoded frontier stores only `PeerId → OpId` (`doc/FRONTIER.md:9-19`). Yet dominance and late-op rules compare the omitted HLC/total-order position (`doc/FRONTIER.md:23-36`). The implementation acknowledges this: `is_late_op` has only a tip ID, uses `op_id != tip_id` as a simplified proxy, and needs a separate full op list for the fixture-friendly correct comparison (`zerodb-core/src/frontier.rs:73-101`).

Additional gaps:

- KERNEL operations still carry only `deps: [OpId]` with a maximum of 64; no frontier/checkpoint reference integrates the claimed O7 compression (`doc/KERNEL.md:64-79`, `doc/FRONTIER.md:19`).
- `SnapshotId` is only a hash; no authenticated snapshot envelope, signer, signature, state encoding, or verification procedure is defined despite the claim that relays store “authenticated snapshots” (`doc/FRONTIER.md:39-50`).
- The tail boundary stores only an OpId even though ordering also requires timestamp and author (`doc/FRONTIER.md:41-48`, `zerodb-core/src/frontier.rs:33-39`).

C7/O7 should not remain “resolved” on these artifacts.

**Required correction:** redesign the frontier as a verifiable sequence/causal commitment (or define author sequence numbers/checkpoint links), integrate it into operation dependencies, and specify a signed snapshot artifact plus tail semantics. Add missing-gap, malicious-tip, restore, and checkpoint-translation vectors.

### CX-05 — The M0e resume cursor cannot represent the state used by the model

DELIVERY defines `Cursor = { last_acked_op_id, epoch }`, but OpIds are content hashes, not sequence positions, and receipt is explicitly reorderable (`doc/DELIVERY.md:31-37`). The rule then says the sender retransmits operations not covered by the receiver's anti-replay set, but that set is not transmitted by the cursor (`doc/DELIVERY.md:33-36`).

The executable model sidesteps the protocol: sender `held` and receiver `seen` sets coexist inside one in-memory object, and resume computes a direct set difference (`zerodb-core/src/delivery.rs:19-55`). The vectors therefore prove set difference, not cursor-based resumption across independent peers.

This does not establish I-12 and is not sufficient for H4 closure or M3a wire design.

**Required correction:** define an actual resumable commitment—e.g. author frontiers/checkpoint plus explicit gap negotiation or Merkle root/walk—and test independent sender/receiver state, stale cursor, late lower-order op, compaction, disconnect, and retry.

### CX-06 — The WAL/group model and M1 evidence do not satisfy the normative gate

WAL prose requires:

- `hlc_persist` only when the carrying op is already/co-atomically appended;
- unique group members and `n == len(members)`;
- all-or-nothing application visibility across recovery (`doc/WAL.md:38-47`, `doc/WAL.md:69-89`).

The model does not enforce these rules:

- `hlc_persist` accepts any timestamp with no related op (`zerodb-core/src/wal.rs:86-88`);
- `GroupManifest` has no `n`; `group_seal` checks neither uniqueness nor member-to-group binding (`zerodb-core/src/wal.rs:21-25`, `zerodb-core/src/wal.rs:90-103`);
- recovery puts every WAL record, including unsealed group members, into `material`; the model exposes no application-visible view that can prove those members remain hidden (`zerodb-core/src/wal.rs:124-139`).

The M1 tests cover closure-return rollback, a normal SQLite transaction, and a bad-signature prefix rollback—not process death at every named append/sync/apply/HLC/seal boundary (`zerodb-storage/tests/e4_groups.rs:23-127`, `zerodb-storage/tests/m1_remainders.rs:87-121`). SPEC explicitly requires crash atomicity at every commit boundary (`doc/SPEC.md:1021-1023`).

**Required correction:** repair the model first, add vectors that falsify each MUST, then map every crash point to real backend fault injection/subprocess-kill tests. Do not call fine-grained crash points optional.

### CX-07 — M1 schema/dependency completion is recorded as done, but the normative layer is absent

The ledger marks schema/IR/`ep` and TS→IR done while admitting full canonical CBOR/SchemaId work is later (`plan/LEDGER.md:60-62`). Current behavior is a raw JSON metadata pin:

- `apply_schema_json` stores input JSON bytes in `meta` (`zerodb-storage/src/lib.rs:315-342`);
- all local operations use `ep = 0` and empty dependencies (`zerodb-storage/src/lib.rs:1001-1031`, `zerodb-storage/src/lib.rs:1103-1136`);
- ingress decodes `deps` and `ep` into the preimage but does not enforce dependency availability, causal readiness, epoch existence, migration, or the 64-dependency limit (`zerodb-storage/src/lib.rs:1309-1385`);
- schema pin checks only local known-label writes; imported CRDT types are not checked against the pin (`zerodb-storage/src/lib.rs:638-668`).

SPEC's M1 gate still requires canonical IR, strict and schemaless modes, secondary indexes, and backend storage contracts (`doc/SPEC.md:1010-1023`). The descriptive M1 doc itself still lists mixed-type/dependency buffering and crash injection as open (`doc/M1-LOCAL.md:71`).

**Required correction:** reopen M1-schema/M1-tsir. Implement actual SCHEMA IR bytes/SchemaId, epoch ops, causal readiness, strict/soft behavior, remote validation, indexes, and migration-safe replay—or formally narrow M1 in SPEC before claiming exit.

### CX-08 — L2 relay and peer Merkle roots have no common accepted-set contract

MERKLE hashes the peer's **accepted** operation set after signature, authz, dedup, and equivocation exclusion (`doc/MERKLE.md:18-25`). AUTH says peer-side authorization is load-bearing and relay admission is only a filter (`doc/AUTH.md:151-181`, `doc/AUTH.md:207-219`). RELAY, however, persists its own “validated” set while lacking full peer authorization and builds its Merkle tree over that oplog (`doc/RELAY-SPEC.md:499-516`, `doc/RELAY-SPEC.md:553-571`).

A malicious/colluding relay can retain authentic but unauthorized operations that honest peers reject. The relay and peers then have permanently unequal roots even after every permissible op was delivered. M3a tests with pre-provisioned benign identities will not expose this; M3b security will.

**Required correction:** define the synchronized set explicitly. Options include full datastore-control authorization at L2, separate raw/authenticated and peer-authorized roots, or a protocol that acknowledges rejected OpIds without claiming root equality. Add malicious-member and revoked-member convergence cases before relay implementation.

---

## 4. High-priority findings

### HX-01 — PLAN/LEDGER materially overstate M1 completion

PLAN says the “remainders [are] largely implemented” and suggests a formal exit pass next (`plan/PLAN.md:57-61`). LEDGER marks E4, E9, schema, query, and TS→IR done and says the checklist is largely met (`plan/LEDGER.md:58-64`). That conflicts with CX-01, CX-06, CX-07 and with SPEC's still-unchecked M1 checklist (`doc/SPEC.md:1010-1023`).

Use `partial`, `prototype`, or `blocked(...)` until the exact normative evidence exists. A green happy-path test should not close an acceptance scenario whose fault schedule was not run.

### HX-02 — E1 evidence does not execute the E1 fault scenario

E1 requires kill-not-shutdown, a separate fresh replay, and a 1-hour clock rollback (`doc/EXEMPLAR.md:34-38`). The 50-todo test drops the in-process store cleanly and reopens/imports it (`zerodb-storage/tests/e1_e2_acceptance.rs:131-190`); the timestamp test also performs a normal drop/reopen and does not roll the clock back (`zerodb-storage/tests/e1_e2_acceptance.rs:201-223`).

Add subprocess kill, uncheckpointed WAL, deterministic clock injection, and a byte-level materialization oracle before M1 exit.

### HX-03 — E9 and graph coverage remain narrower than the published model

The implementation has node tombstones and derived incident-edge visibility, but no edge tombstone API, edge properties, conflicting entity-create semantics, or full equivocation handling for entity operations. `KIND_TOMBSTONE` is node-only (`zerodb-storage/src/lib.rs:1396-1451`), and query graph construction gives every edge an empty property map (`zerodb-storage/src/lib.rs:384-400`).

E9's recreate/no-resurrection clause is not tested, and the confirmed tombstone-before-create defect demonstrates why it matters (`doc/EXEMPLAR.md:78-82`).

### HX-04 — HLC authority and implementation disagree

KERNEL restart says derive `latest` from **own-device** operations and treats metadata as a cache (`doc/KERNEL.md:123-130`). The store queries the maximum timestamp across all authors and then takes `max(oplog, meta)`, allowing metadata ahead of the oplog to remain authoritative (`zerodb-storage/src/lib.rs:1945-1967`). WAL's model similarly says all authors because it lacks an author field (`zerodb-core/src/wal.rs:124-139`).

There is a real design choice here: preserving causality after receiving a remote timestamp may require durable receive-clock state, while own-op-only recovery only preserves local issuance monotonicity. Specify that state and test both stale-low and stale-high metadata, remote-receive/restart/local-write, restore, and clone divergence.

### HX-05 — AUTH contract gaps remain behind the “resolved” label

- Device certificate `expiry` is encoded but never checked by `verify_device_cert` (`doc/AUTH.md:29-49`, `zerodb-core/src/auth.rs:237-255`).
- Relay `KnownGrant` has no expiry, so admission verification cannot reject expired membership tokens (`zerodb-core/src/auth.rs:332-338`, `zerodb-core/src/auth.rs:406-442`).
- `delegable` is documented as permitting subset grants but is ignored by the authorization predicate, which requires admin for grant ops (`doc/AUTH.md:96-105`, `zerodb-core/src/auth.rs:453-465`, `zerodb-core/src/auth.rs:553-586`).
- Authz vectors receive `candidate.principal` as trusted input; they do not compose cert resolution, author signature, genesis, and authorization into one operation-ingress negative suite (`zerodb-core/tests/conformance_auth.rs:135-159`, `zerodb-core/tests/conformance_auth.rs:246-263`).

Resolve the intended expiry/delegation semantics and add composed end-to-end vectors before M3b.

### HX-06 — The relay specification is stale relative to the claimed M0 closure

The relay draft still says subscription is unauthenticated, traversal is a placeholder pending C3, durable ack is pending H11, canonical operation bytes are pending C1, and canonical Merkle construction is pending C3 (`doc/RELAY-SPEC.md:221-259`, `doc/RELAY-SPEC.md:328-339`, `doc/RELAY-SPEC.md:512`, `doc/RELAY-SPEC.md:553-565`). It also orders L2 receipt acknowledgement before persistence (`doc/RELAY-SPEC.md:467-475`).

Before M3a, publish a new relay draft wired to KERNEL/AUTH/MERKLE/DELIVERY/VERSIONS, with generated message schemas and two-language wire vectors.

### HX-07 — Freeze state is contradictory

Package docs and registry say byte freeze occurs “at” or “only at” composite M0 (`doc/KERNEL.md:4`, `doc/SCHEMA.md:4`, `doc/MERKLE.md:4`, `conformance/registry.json:6-23`). Composite M0 is now marked done (`plan/PLAN.md:27`). PLAN/ISSUES simultaneously say no format is frozen without a separate explicit Decision Log entry (`plan/PLAN.md:28-35`, `doc/ISSUES.md:48-49`). README says formal M1 exit **plus freeze** remains open while PLAN calls freeze optional (`README.md:5`, `plan/PLAN.md:59`).

Choose one state: `draft-1 / unfrozen` is the only defensible current answer. Update every package status and state whether freeze is an M1 prerequisite, an optional post-M1 action, or a later release gate.

### HX-08 — The TS→IR deliverable is mislabeled

The tool describes itself as “TypeScript-ish authoring JSON → simplified schema IR JSON”; full TS AST support is later, and output is a local pin map (`tools/ts-to-ir/ts-to-ir.mjs:1-19`). It does not emit SCHEMA's canonical CBOR IR, type/null/encryption definitions, edges, SchemaId, epochs, or migration data. Its tests assert only three CRDT strings and `unique` rejection (`tools/ts-to-ir/test.mjs:1-29`).

Call this `json-to-local-pin` or complete the actual O2 pipeline. It cannot satisfy M1-tsir as currently named.

### HX-09 — The xfail promotion lane is wired incorrectly on the Rust side

The policy requires new xfail vectors to be demonstrably red and non-blocking (`conformance/README.md:22-28`), and the JS xfail job is allowed to fail (`.github/workflows/ci.yml:31-39`). Most Rust conformance tests iterate over both `required` and `xfail` during the normal `cargo test --workspace` job—for example auth, CRDT, envelope, epoch, HLC, Merkle, op, query, schema, and WAL (`zerodb-core/tests/conformance_auth.rs:164-178`, `zerodb-core/tests/conformance_wal.rs:69-85`).

The first intentionally red Rust xfail vector will fail the required Rust job. Add lane selection or a dedicated continue-on-error Rust xfail job.

### HX-10 — CI does not reproduce current M1/M2 evidence

CI runs workspace tests, fmt, and the JS conformance runner only (`.github/workflows/ci.yml:9-39`). It does not run:

- `tools/ts-to-ir` tests;
- NAPI build + Node tests on a clean checkout;
- `scripts/test-mvp.ps1` or a portable equivalent;
- clippy;
- `cargo test --locked`;
- a platform matrix for SQLite/native packaging.

Local `npm --prefix zerodb-napi test` succeeded only because a locally built ignored Windows `.node` artifact existed. The package declares only `x86_64-pc-windows-msvc` (`zerodb-napi/package.json:7-13`). Add clean build/test/package jobs and Linux/macOS/Windows support policy before calling the scaffold done.

### HX-11 — M2 dependency/status fields are internally inconsistent

M2 is in progress while its declared dependency is M1, and M2-query/schema are labeled blocked on M1 rows that the same ledger marks done (`plan/LEDGER.md:70-83`). Parallel experimentation is reasonable, but the tracker should distinguish:

- `prototype-on-experimental-M1` (allowed now),
- `M2 exit dependency` (blocked on formal M1 exit/API contract), and
- actual technical blockers.

### HX-12 — Experimental ingress does not enforce the declared resource profile

The registry caps operations at 64 KiB, batches at 256 KiB/512 ops, deps at 64, and CBOR depth at 16 (`conformance/registry.json:76-84`). Local JSON/TCP import does not enforce these limits; transport accepts framed messages up to 64 MiB (`zerodb-cli/src/main.rs:561-573`), and `validate_wire_for_ds` decodes arbitrary dependency counts (`zerodb-storage/src/lib.rs:1339-1345`).

Even for a labeled LAN experiment, enforce cheap size/count limits before allocation/crypto to prevent accidental memory/CPU abuse and to make backend behavior converge toward M3.

---

## 5. Documentation and plan discrepancies

| ID | Discrepancy | Correction |
|----|-------------|------------|
| D-01 | `M1-LOCAL` says `grp` is always absent and edges are unimplemented, but both now exist (`doc/M1-LOCAL.md:83`, `zerodb-storage/src/lib.rs:255-312`, `zerodb-storage/src/lib.rs:944-992`). | Refresh the descriptive doc on every implementation milestone. |
| D-02 | `M1-LOCAL` calls named WAL crash points “optional,” while SPEC makes every commit boundary an exit requirement (`doc/M1-LOCAL.md:111`, `doc/SPEC.md:1023`). | SPEC wins; remove “optional.” |
| D-03 | `FINDINGS.GROK` is pinned to `f9cc660` and still calls now-landed work missing (`plan/FINDINGS.GROK.md:1-28`). | Archive it as historical or refresh it; do not call it the current open backlog. |
| D-04 | LEDGER says historical review files were removed, but `plan/FINDINGS.GROK.md` remains (`plan/LEDGER.md:18`). | Correct the index and define archive policy. |
| D-05 | SPEC is dated 2026-07-15 and still contains all M1 checkboxes unchecked while README/PLAN claim substantial completion (`doc/SPEC.md:3-6`, `doc/SPEC.md:1010-1023`, `README.md:5`). | Keep normative checkboxes synchronized or move live checklist authority entirely to LEDGER with trace links. |
| D-06 | RELAY uses string datastore IDs and old operation examples while KERNEL fixes byte identifiers and a tagged operation algebra (`doc/RELAY-SPEC.md:225-249`, `doc/RELAY-SPEC.md:791-812`, `doc/KERNEL.md:27-39`, `doc/KERNEL.md:60-98`). | Replace examples/types from one machine-readable registry/schema. |
| D-07 | RELAY recommended limits (1 MiB op, 16 MiB batch, 64 ops) conflict with KERNEL/registry maxima (64 KiB, 256 KiB, 512 ops) (`doc/RELAY-SPEC.md:526-533`, `conformance/registry.json:76-84`). | Separate hard format limits from negotiated transport limits and require the effective minimum. |
| D-08 | SPEC describes strict schema mode and secondary indexes, but neither appears in the current M1 backlog as open (`doc/SPEC.md:356-358`, `doc/SPEC.md:716-743`, `plan/LEDGER.md:60`). | Add explicit M1 rows or formally defer by amending SPEC. |
| D-09 | SPEC M1 requires `repl`; LEDGER marks query/repl done while admitting no REPL exists (`doc/SPEC.md:1020`, `plan/LEDGER.md:61`). | Keep row partial until REPL lands or remove REPL from M1 normatively. |
| D-10 | Query supports parameters normatively, but LocalStore always supplies an empty parameter map and exposes no parameter API (`doc/SCHEMA.md:112-130`, `zerodb-storage/src/lib.rs:346-353`). | Add CLI/NAPI parameter binding and negative tests. |
| D-11 | `storage_format_version` is automatically backfilled onto unversioned DBs without proving their layout (`zerodb-storage/src/lib.rs:1969-1977`). | Use an actual migration/probe; unknown legacy layout must fail closed after freeze. |
| D-12 | The NAPI API is synchronous and low-level while SPEC presents promise-based SDK semantics (`doc/SPEC.md:597-600`, `zerodb-napi/index.d.ts:3-36`). | Decide whether NAPI is an internal primitive under an async TS facade or the public SDK. |

---

## 6. Obvious implementation improvements

These are below the gate blockers but should be planned explicitly:

1. Add deterministic semantics for conflicting `CreateNode`/`CreateEdge` operations instead of arrival-order `UPSERT` label/endpoints (`zerodb-storage/src/lib.rs:1227-1253`).
2. Reject `delete-node` for an unknown node rather than committing a no-effect tombstone, unless tombstone-before-create is explicitly part of the set model (`zerodb-storage/src/lib.rs:236-251`).
3. Add edge property mutation, edge tombstone, inspect/get APIs, and query edge-property materialization before claiming property-graph completeness.
4. Avoid full-table scans in `infer_crdt_from_tx` and `load_prop_ops` for every property mutation; add `(kind/entity/path)` or normalized oplog indexes after semantics stabilize (`zerodb-storage/src/lib.rs:1602-1764`).
5. Return a typed error rather than panicking if OS randomness is unavailable (`zerodb-storage/src/lib.rs:1906-1908`).
6. Define key custody before non-disposable M1 use; the signing seed remains plaintext in SQLite, correctly disclosed in `M1-LOCAL` (`doc/M1-LOCAL.md:47-53`).
7. Add explicit cleanup or unique temporary directories in tests; repeated runs currently leave many SQLite artifacts under `target/`.
8. Add clippy to normal development CI after fixing the five current `-D warnings` failures.
9. Give every LEDGER `done(...)` row a commit plus exact test/vector path; “done” should be mechanically auditable.
10. Ratify DQ-12 enough to set E11 hardware, warm/cold definitions, percentile, dataset, and regression thresholds before M2 exit.

---

## 7. Recommended plan revision

### R0 — Contract and state stabilization (new, blocks formal M1 exit/freeze)

#### R0.1 — Protect existing data and restore SEC

1. Add red tests for re-init of initialized empty and nonempty DBs; make default `init` fail closed.
2. Add red permutation tests for create/tombstone, create/create label conflict, edge/create/delete, and entity equivocation.
3. Replace imperative entity rows with set-derived deterministic materialization.
4. Run fresh replay after every permutation and require byte-identical normalized state.

**Exit:** CX-01/CX-02 fixed; no default destructive command; I-1/I-16 storage tests green.

#### R0.2 — Reopen the deficient M0 contracts

1. **M0a:** remove encrypted-envelope/OpId circularity and add a complete encrypted-operation vector.
2. **M0e.1:** enforce HLC append relation, manifest cardinality/uniqueness/binding, and unsealed-group invisibility in model vectors.
3. **M0e.2:** replace the pseudo-cursor with an independently executable resume protocol.
4. **M0f:** define a verifiable frontier, operation/checkpoint integration, signed snapshot artifact, and valid tail boundary.
5. Re-run both runners and add an explicit Decision Log “targeted M0 amendment” entry; keep profile unfrozen.

**Exit:** CX-03 through CX-06 resolved by prose + positive/negative vectors, not direction notes.

#### R0.3 — Align authority and relay inputs

1. Declare one unambiguous freeze state (`draft-1, unfrozen`).
2. Refresh RELAY against resolved packages and define the relay/peer accepted set.
3. Generate identifiers, message bodies, limits, outcome codes, and transcripts from a machine-readable protocol artifact.
4. Correct PLAN/LEDGER/M1-LOCAL/GROK status drift.

**Exit:** no conflicting status/limits/type definitions; M3 wire work has an executable input contract.

### M1 — Complete the normative local release

#### M1a — Canonical data/control plane

- actual canonical Schema IR + SchemaId;
- SchemaEpoch operations and `ep` enforcement;
- causal `deps` readiness and limits;
- strict/schemaless modes and remote CRDT pin validation;
- required secondary indexes;
- deterministic nodes, edges, edge properties, and tombstones.

#### M1b — Durability acceptance

- fault hooks mapped to each WAL crash point;
- subprocess kill/reopen tests;
- E4 group manifest/all-or-none state tests;
- stale-low/stale-high HLC metadata, receive/restart, restore/clone tests;
- E1 exact 50-todo + clock rollback + kill + fresh-replay oracle.

#### M1c — CLI acceptance

- safe `init` / explicit reset;
- schema apply of canonical IR;
- parameterized query and interactive REPL (or approved SPEC deferral);
- edge CRUD/inspect;
- portable smoke script for CI.

#### M1 exit decision

Close only when every SPEC M1 checkbox has a LEDGER evidence link and E1/E2/E4/E9 exact scenarios pass. Treat format freeze as a **separate decision**; do not make an unsafe freeze the reward for finishing M1.

### M2 — SDK vertical after the M1 contract surface stabilizes

1. Keep the NAPI crate as an internal binding if the public SDK will be async.
2. Build/test native packages from clean checkout on supported OS/arch combinations.
3. Add query, schema, edges, mutate/batch, subscribe, close/lifecycle behavior.
4. Add binding parity against canonical core fixtures—not only CRUD smoke tests.
5. Implement MVRegister/resolve, RGA, and LWWMap with cross-language vectors.
6. Ratify and run E11 budgets; inspect `npm pack` contents and installation tests.

### M3a/b/c — Preserve the split, add two preconditions

Before M3a implementation:

- relay/peer accepted-set and Merkle-root semantics must be resolved;
- complete M0c/M0e wire messages and durable-ack behavior must have two-language transcripts.

Then keep the existing order: durable catch-up → security/admission/encryption → independent wire peer/release.

---

## 8. Immediate decision queue

1. **Freeze status:** confirm `draft-1, unfrozen`; decide whether any freeze is required for M1 release.
2. **Encrypted AAD:** select the non-circular context/preimage construction.
3. **Entity delete/create:** define tombstone CRDT, conflicting create semantics, and whether recreation with the same ID is legal.
4. **Frontier:** select sequence-number, causal-dot, checkpoint-chain, or another verifiable representation.
5. **Resume:** decide whether resume is frontier/gap based, Merkle-only, or hybrid; retire `last_acked_op_id` if it has no ordering meaning.
6. **Group visibility:** define where unsealed members live and how they remain invisible across recovery/import.
7. **HLC recovery:** decide the durable state needed for remote-receive causality vs own-operation monotonicity.
8. **Delegation/expiry:** decide device-cert expiry, grant expiry at relay admission, and whether `delegable` survives v0.1.
9. **M1 scope:** either implement canonical schema/strict/index/REPL requirements or amend SPEC before exit.
10. **M2 surface/platforms:** public async TS facade vs synchronous NAPI API; supported Node versions and target matrix.
11. **Relay accepted set:** full datastore auth at L2 vs dual roots vs explicit rejected-op reconciliation.
12. **Capacity:** ratify DQ-12 and assign realistic R0/M1/M2 effort ranges.

---

## 9. Verification performed

### Repository state

- `main` at `3b269e8`, equal to `origin/main` before this findings file was created.
- Worktree was clean before review-generated ignored `target/` artifacts.
- Reviewed all planning/status docs, normative package specs, relay spec, manifests/CI, current Rust implementation/tests, NAPI package/tests, and TS→IR tool/tests.

### Commands and outcomes

| Command | Outcome |
|---------|---------|
| `cargo test --workspace` | **pass**; all non-ignored workspace tests passed |
| `node conformance/ts/runner.mjs --lane required` | **pass: 103/103** |
| `npm --prefix tools/ts-to-ir test` | **pass: 2/2** |
| `npm --prefix zerodb-napi test` | **pass: 3/3**, using a pre-existing local Windows native addon |
| `cargo fmt --all -- --check` | **pass** |
| `cargo clippy --workspace --all-targets -- -D warnings` | **fail**: five lint errors (two collapsible-if, clone-on-copy, cloned-ref-to-slice, ptr-arg) |
| Manual reversed create/delete import | **confirmed defect**: same two ops materialize deleted vs live |
| Manual re-init of nonempty DB | **confirmed defect**: new top-level ds/key with retained old-ds ops |

### Review limitations

- No live Windows↔Pi run was repeated during this review.
- No clean VM/container NAPI build or package-install matrix was run.
- No fuzzing, sanitizer, model checking, benchmark, or external cryptographic review was performed.
- Passing current vectors demonstrates their encoded examples, not completeness of the contracts; CX-03 through CX-06 identify missing falsification cases.

---

## 10. Conclusion

The project is in a better state than the current verdict sounds: it has crossed from planning into a credible experimental database implementation with meaningful conformance infrastructure. The issue is not lack of progress; it is that status labels have advanced faster than the contracts and evidence.

The highest-leverage next move is **not** a ceremonial M1 exit or a format freeze. It is a short stabilization gate that fixes the confirmed data-state defects, repairs the four deficient M0 contracts, and makes PLAN/LEDGER mechanically traceable to exact acceptance evidence. After that, M1 can close honestly, M2 can build on a stable substrate, and M3 can start from a relay protocol that can actually converge under its security model.
