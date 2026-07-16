# Exemplar — Distributed ToDo App (Acceptance Scenarios)

The Exemplar is ZeroDB's end-to-end acceptance target: a distributed ToDo application whose scenarios below are the executable definition of "the milestone works." Every milestone exit gate in [SPEC §10](SPEC.md) cites scenario IDs from this file; every scenario cites the [INVARIANTS](INVARIANTS.md) it exercises.

**Status:** scenario definitions v1 (PLAN P0-3). Scenarios become executable tests in `conformance/` (model level) and the integration suites of their gating milestone. A scenario is *versioned*: changing its expected outcome requires a Decision Log entry.

## Application model

- **Datastore-per-list.** Each ToDo list (and each individually shared item) is its own datastore with its own membership and Merkle tree. "Share a list" = grant a datastore-membership capability; "share one item" = move/copy it into a single-item datastore and share that. There is **no entity-level ACL** in v0.1 ([ISSUES C6](ISSUES.md)).
- **Schema (informative sketch):** `Todo` node — `title: LWW<string>`, `done: Flag`, `tags: ORSet<string>`, `priority: LWW<number>`, `voteScore: PNCounter`; `Note` node — `body: LWW<string>` (encrypted in E6), edge `NOTATES: Note → Todo`; edge `CONTAINS: List → Todo`. Ordered lists (`RGA`) enter at M2.
- **"Administrative controls"** for v0.1 means the datastore owner's local operations (create datastore, grant/revoke membership, apply schema) via CLI/SDK — **not** distributed admin roles (post-v0.1, M6).
- **Actors:** `A` (owner), `B` (member), `C` (non-member/attacker). `R` is an L2 relay unless stated. Fault schedules are exact and reproducible.

## Scenario index

| ID | Scenario | Gates | Invariants |
|----|----------|-------|------------|
| E1 | Single-peer CRUD, restart, deterministic replay | M1 | I-1, I-3, I-4, I-14 |
| E2 | Conflict matrix across all M1 CRDT types | M1 (model) / M3a (live) | I-1, I-2, I-3, I-7 |
| E3 | Offline partition, relay catch-up, 3-peer convergence | M3a | I-1, I-11, I-12 |
| E4 | Crash mid-group: atomicity across recovery | M1 | I-13, I-14 |
| E5 | Share a list; non-member rejected | M3b | I-8, I-9 |
| E6 | Encrypted private notes; relay/non-recipient blind | M3b | I-10 |
| E7 | Forged & replayed operations rejected | M3b | I-3, I-8, I-9 |
| E8 | Far-future clock abuse quarantined | M3b | I-4, I-5 |
| E9 | Delete, dangling edge, late edge, no resurrection | M1 | I-16 |
| E10 | Schema migration replay across mixed versions | M4b | I-17 |
| E11 | Performance smoke: 10k-op list | M2 (provisional) / M5 (binding) | — |

## Scenarios

### E1 — Single-peer CRUD, restart, deterministic replay (M1)
**Given** peer A with an empty `groceries` datastore and applied schema.
**When** A creates 50 todos, edits titles, toggles `done`, adds/removes tags, adjusts `voteScore`; the process is killed (not shut down) and restarted; the oplog is separately replayed from scratch on a fresh store.
**Then** restarted state == pre-kill state == fresh-replay state, byte-identical materialization; every post-restart timestamp exceeds every pre-kill timestamp (I-4) even with the test clock rolled back 1 h at restart.

### E2 — Conflict matrix (M1 model / M3a live)
**Given** two replicas of one list, partitioned.
**When** both concurrently, per property type: edit the same `title` (LWW), add/remove overlapping `tags` (ORSet — concurrent add+remove ⇒ present), increment/decrement `voteScore` (PNCounter — both effects sum), toggle `done` (Flag — enable wins), edit the same `title` with **equal HLC timestamps** (I-7 tie-break/equivocation case); then merge in both directions.
**Then** both replicas materialize identical state matching the per-type oracle table checked into the fixture; merge order does not matter; re-merging is a no-op.

### E3 — Offline partition and relay catch-up (M3a)
**Given** A, B, C members of one datastore via L2 relay R; all converged.
**When** C disconnects for a simulated 1 h while A and B write 1 000 interleaved ops through R (C receives none live); B also crashes and restarts mid-window; C reconnects and syncs from R alone (A and B are offline at that moment).
**Then** C converges to the exact A/B state using only R's durable history (no live source); Merkle roots match across all three after reconnection; no op is delivered twice with effect (I-3) and none is missing (I-12). *This scenario is the CX-04 offline-first proof and fails by construction against an L1-only relay.*

### E4 — Crash mid-group (M1)
**Given** peer A writing a 5-op group (todo + 2 tags + note + edge).
**When** the process crashes at **each** named crash point of the M0e.1 WAL contract (one run per point) and recovers.
**Then** after every recovery, either all 5 ops are materialized or none are; the oplog, state store, and HLC agree; a completed-then-crashed group is not re-applied with double effect.

### E5 — Membership sharing and denial (M3b)
**Given** A owns list L; B holds a valid membership capability for L issued by A; C holds none (and later: a revoked one).
**When** B subscribes and writes; C attempts to subscribe, to submit a validly-signed op authored by C, and to replay one of B's ops into a different datastore; after revocation of B, B writes again.
**Then** B's pre-revocation ops materialize everywhere; every C attempt is rejected **by peers** (not merely by the relay — the test also runs with a colluding relay that forwards everything); post-revocation B ops are rejected per the historical-authorization rule (DQ-3); rejections are observable outcomes, not silent drops.

### E6 — Encrypted private notes (M3b)
**Given** A and B share list L through relay R; note bodies are declared encrypted for recipients {A, B}.
**When** A writes notes; the harness captures every artifact R observes (frames, stored ops, logs) plus a full datastore replica handed to non-recipient C.
**Then** B reads plaintext; neither R's captured artifacts nor C's replica permit plaintext recovery (harness includes a decrypt oracle attempt); after A removes B and rotates keys, B cannot decrypt notes written post-rotation.

### E7 — Forged and replayed operations (M3b)
**Given** the E5 topology with a malicious harness peer.
**When** the harness submits: an op with a flipped payload byte, an op signed by C claiming author B, a byte-exact duplicate of an old B op, and the same duplicate after the relay's dedup state is wiped.
**Then** zero of these change materialized state on any conforming peer; each yields the contract-specified rejection outcome.

### E8 — Clock abuse (M3b)
**Given** converged peers A, B and attacker C (a member) whose clock is set +30 days.
**When** C writes a far-future-timestamped op contending an LWW field with A's current write.
**Then** the H1 acceptance/quarantine rule applies deterministically on every peer — C's op does not silently win LWW for the next 30 days; when the quarantine window resolves, all peers converge on the same outcome (I-1).

### E9 — Delete semantics (M1)
**Given** peer A with todos, notes, and edges; a second replica for late-op delivery.
**When** A deletes a todo that has edges; the replica — which has not yet seen the delete — concurrently adds a new edge to that todo, then syncs; the todo's node is later re-created with the same logical content.
**Then** visibility follows the H3 derived-visibility state machine identically on both replicas: no divergent cascade ops are generated, the late edge resolves deterministically, and re-creation does not resurrect deleted edges.

### E10 — Schema migration replay (M4b)
**Given** history for a list whose `priority` field migrates LWW\<string\> → LWW\<number\> at epoch 2, with peers at mixed versions.
**When** an old-epoch peer receives post-migration ops (buffer/reject per C2 rule), upgrades, and replays; a fresh peer replays the full mixed-epoch history from scratch.
**Then** every peer materializes the identical post-migration state; replay across the epoch boundary is deterministic (I-17); the mixed-version buffering rule is observed exactly.

### E11 — Performance smoke (provisional M2 / binding M5)
**Given** one list with 10 000 operations across 1 000 todos.
**When** cold-open materialization, a 100-op incremental sync, and the O3 minimal query (`MATCH … WHERE done = false ORDER BY priority LIMIT 50`) run on the reference desktop profile.
**Then** provisional budgets (to be ratified in the delivery ledger, DQ-12): cold materialize < 1 s; incremental sync round < 250 ms; query < 50 ms; WASM bundle within the O4 budget once M4 lands. Budgets are **provisional** until M5 makes them binding.

## Explicitly out of scope (v0.1)

- Distributed administrative roles / entity-level ACLs → M6 (C6 successor design)
- Item-level sharing *within* a shared list other than datastore-per-item → M6
- Richtext note bodies → post-v0.1 (M5 feature track)
- GunDB data import → won't do (Decision Log 2026-07-16)
