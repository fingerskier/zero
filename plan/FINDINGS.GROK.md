> **Historical document (disposition recorded).**  The headline items C-P1–C-P3 were substantially resolved by commit `3aba801` (M0a–M0f split, pre-M0 implementation policy, release labels); remaining items are superseded by the newer review.  Per-finding disposition: [FINDINGS.CODEX.md §7](FINDINGS.CODEX.md).  Live work status is in [LEDGER.md](LEDGER.md).  Do not treat recommendations below as open work items.

# ZeroDB Project Plan Review — FINDINGS.GROK

**Date:** 2026-07-15  
**Reviewer:** Grok (xAI)  
**Scope:** Critical review of the **project plan** (SPEC §10 M0–M6 roadmap, ISSUES.md gating, README v0.1 scope, EXEMPLAR acceptance target, supporting docs) against current repository state.  
**Not in scope:** Full re-audit of every CRDT algorithm detail (already tracked as C/H issues); line-level code quality of the ~1k LOC scaffold.

**Documents reviewed:**

| Document | Role in the plan |
|----------|------------------|
| `README.md` | Public status, v0.1 scope, milestone summary |
| `doc/SPEC.md` §10 (+ cross-refs) | Authoritative milestone plan and product architecture |
| `doc/ISSUES.md` | C/H/O gates, decision log |
| `doc/RELAY-SPEC.md` | Wire-protocol slice deferred behind M0/M3 |
| `doc/EXEMPLAR.md` | Claimed end-to-end acceptance target from M1 |
| `doc/INVARIANTS.md`, `doc/BIBLIO.md` | Supporting rigor (currently stubs/thin) |
| `zerodb-core/`, `zerodb-storage/`, root `Cargo.toml` | Evidence of pre-M0 implementation |

**Prior art:** External review `FINDINGS.CODEX.md` (2026-07-13, retired) produced C1–C8 / H1–H11 and the M0–M6 replacement roadmap; those findings were folded into ISSUES and SPEC. This review evaluates the **adopted plan**, not the original Phase 1–5 plan.

---

## 1. Executive verdict

**The M0–M6 plan is directionally correct and a real improvement** over an implementation-first Phase plan: it puts executable contracts before format freezes, local durability before multi-peer, datastore-level trust before entity ACLs, and ecosystem work last.

**It is not yet an execution-ready project plan.** As written it is a high-quality **milestone sketch** with:

- Severely overloaded gates (especially **M0**, **M3**, **M5**)
- An undefined product cut called “v0.1” relative to milestones
- Acceptance criteria that do not exist in executable form (EXEMPLAR is a feature wish-list)
- Residual SPEC narrative that still markets post-v0.1 features as if they were near-term
- Early Rust scaffolding that already risks de-facto format choices before M0 exit
- No owners, effort, critical path, or process for “approved normative resolution”

**Recommendation:** Treat the current roadmap as **approved architecture of sequencing**, not as a ship schedule. Before treating M0 as “in progress,” split M0 into ordered contract packages, define which milestone ships “v0.1,” and replace EXEMPLAR with scenario-level acceptance tests. Do **not** expand the scaffold into oplog/Merkle/wire types until C1–C5 contracts and golden vectors exist.

**Bottom line:** Plan grade **B− as strategy / D as project management**. Strong sequencing thesis; weak scoping, acceptance, and contradiction hygiene.

---

## 2. What the plan gets right

These should be preserved; they are the durable value of the 2026-07-13 replan.

1. **M0 before freezes.** Explicit rule that no wire or persistent format freezes until Critical issues have normative resolutions + red conformance tests. Correct for a multi-language, content-addressed system.
2. **Local-first product slice (M1).** Rust + SQLite + CLI before Node/WASM/mobile avoids binding-layer thrash while core semantics are still fluid.
3. **Trust model honesty (C6).** Deferring entity-level distributed ACLs and admitting read-ACL is not confidentiality is the right security posture for v0.1.
4. **Issue IDs as gates.** C/H/O mapping into milestones is better than a prose “open questions” graveyard.
5. **Relay pruning (0.2).** CBOR-only, no session resumption/PoW/mutual-auth theater, schema-blind relays — reduces surface while contracts are unfinished.
6. **GC disabled until C7.** Explicitly refusing compaction until causal frontiers and peer lifecycle exist avoids silent data loss.
7. **Red/green milestone culture.** Each milestone “begins with failing contract/acceptance tests” is the right engineering discipline *if* those tests are actually written first.
8. **Crypto suite fixed early.** Ed25519 / BLAKE3 / X25519+XChaCha20-Poly1305 as interop constants is a good early decision (custody pluggable, algorithms not).

---

## 3. Critical plan findings

Critical = will cause rework, false “done,” security/product failure, or multi-month stall if not fixed **before treating the plan as executable**.

### C-P1 — M0 is a multi-quarter research program labeled as a gate

**M0 exit requires all of:**

| Work package | Source |
|--------------|--------|
| Full operation algebra + deterministic CBOR + preimages | C1 |
| Schema epochs + migration DSL | C2 |
| Canonical Merkle tree + complete sync state machine | C3 |
| Datastore membership capabilities + signed context | C4 |
| Author-key distribution / rotation / historical lookup | C5 |
| Snapshot/compaction/causal-frontier **contracts** | C7 |
| Group manifests + crash atomicity contracts | C8 |
| Delivery/dedup/anti-replay contracts | H4 (partial) |
| Version/upgrade policy | H7 |
| O2 schema SoT + O3 query subset decisions | O2, O3 |
| Two independent toy implementations + golden/negative fixtures | SPEC §10 M0 outcome |
| Lean 4 models + proof **statements** drafted | SPEC §10 M0 checklist |

That is not one milestone; it is the entire foundation of the product. There is **no internal ordering**, no “contract package” definition, no estimate of whether “two toy implementations” means two crates in-repo or two languages, and no rule for partial freezes (e.g. freeze op encoding after C1 while Merkle is still open).

**Risk:** M0 never exits → either the project stalls in design, or implementers quietly freeze incomplete formats from the existing scaffold.

**Fix:** Split M0 into sequenced packages with independent red fixtures, for example:

1. **M0a** — identifiers + op encoding + hash/sign preimages (C1, C4 context fields)  
2. **M0b** — schema epoch + migration DSL (C2, O2)  
3. **M0c** — Merkle canonical form + sync state machine transcripts (C3)  
4. **M0d** — authz keys/capabilities (C5, C4 admission)  
5. **M0e** — groups, delivery, version policy (C8, H4, H7)  
6. **M0f** — snapshot/frontier contracts only (C7, O7) — not GC impl  

Lean 4 **statements** may stay in M0; Lean **proofs** must not gate M0 (see C-P5).

---

### C-P2 — “M0 precedes all implementation” is already violated in spirit

Decision log: *“M0 spec-stabilization precedes all implementation.”*  
Repository reality:

- Workspace crates `zerodb-core` / `zerodb-storage` exist (~1k LOC)
- `PeerId` is a **16-byte random** value, not `BLAKE3(Ed25519PublicKey)` (SPEC §6.1)
- Storage is a single minimal `StorageAdapter` with `put_op`/`get_op` — not the SPEC dual `OplogStore`/`StateStore`, and not C8-transactional
- Comment in storage: *“Intentionally minimal for Phase 1”* — obsolete plan language

Exploratory code is fine; **unnamed exploratory code with production type names** is how premature freezes happen. HLC/LWW experiments are low risk; `OpId`/`PeerId`/`GroupId` shapes and storage traits are not.

**Fix:**

- Label current crates `experimental` / `// unstable until M0` in README and crate docs  
- Forbid committing Merkle/op wire codecs until M0a golden vectors exist  
- Add a short “implementation policy” to SPEC §10: which code may land pre-M0 (pure CRDT math, HLC) vs which must not (wire, persistence layout, PeerId derivation)

---

### C-P3 — “v0.1” is not mapped to any milestone

README and SPEC list v0.1 commitments:

- Runtime: Rust core + SQLite + CLI  
- Trust: signatures + datastore membership  
- Non-goals: entity ACLs, mobile, Richtext, hosted relay, GunDB migration  

But:

| v0.1 claim | Earliest milestone that can deliver it |
|------------|----------------------------------------|
| Offline CRUD + CLI | M1 |
| Membership capabilities enforced | M3 |
| Multi-user sharing (EXEMPLAR) | M3 |
| Private/encrypted data (EXEMPLAR) | M3 (H10) |
| TypeScript SDK | M2 |
| Browser | M4 |

So “v0.1” is either:

- **M1-only** → not a product anyone can share or encrypt, or  
- **M1–M3** → a large multi-milestone train with no intermediate release names.

**Fix:** Define release trains explicitly, e.g.:

- **v0.1.0-local** = M1 exit  
- **v0.1.0-sdk** = M2 exit  
- **v0.1.0** (first multi-peer) = M3 exit  

Or pick one and rewrite README/EXEMPLAR non-goals accordingly.

---

### C-P4 — EXEMPLAR cannot gate milestones as written

SPEC §10: *“The Exemplar ToDo app supplies end-to-end acceptance scenarios from M1 onward.”*

`doc/EXEMPLAR.md` is ~28 lines of bullets: CRUD, share/link, notations, admin controls, private/public data, pressure-test themes. It has:

- No schema  
- No peer topology or failure scenarios  
- No offline/conflict cases with expected merge outcomes  
- No security scenarios (membership denial, forged op, encryption key rotation)  
- No mapping of feature → milestone (admin controls contradict C6 deferral; private data needs H10)  
- No success metrics or performance budgets  

**Risk:** Every “exit gate” that cites the exemplar is unfalsifiable → green milestones with no product proof.

**Fix:** Rewrite EXEMPLAR as a test plan with named scenarios, e.g.:

- `E1` single-peer CRUD + restart/replay (M1)  
- `E2` concurrent LWW/ORSet/counter conflicts (M1 local / M3 multi-peer)  
- `E3` share list via membership capability; non-member rejected (M3)  
- `E4` encrypted title/body; non-recipient cannot decrypt (M3)  
- `E5` partition 1h + rejoin converges (M3)  
- Explicitly mark “administrative entity ACLs” as **post-v0.1 / M6**, or redefine “admin” as local CLI ops on a single-owner datastore  

---

### C-P5 — M3 and M5 are “big bang” milestones that will slip or ship incomplete

**M3** packs into one exit:

- Authz (C4/C5), full Merkle wire (C3), delivery/ack (H4/H11), handshake (H5), E2E crypto lifecycle (H10), clock policy (H1), shared P2P+relay handshake (H6), reference relay, **two-language** conformance, partition/malicious-peer suite, exemplar sharing+private data  

That is “the entire networked secure product.” A slip in H10 alone blocks the exemplar’s private-data claim; a slip in C3 blocks all sync value.

**M5** packs into **GA**:

- Compaction/GC (C7 impl), unique indexes (H2), **Peritext Richtext**, backup/restore, observability/SLOs, fuzzing/soak, **Lean 4 proofs for many CRDTs + Rust conformance**, external security audit, competitive benchmarks  

Formal proofs and external audit are **research/assurance tracks**, not reasonable co-requisites of a first GA of an unfinished graph DB. Making them GA-blocking either delays forever or forces fake “proofs.”

**Fix:**

- Split M3 into **M3a** (unsigned/dev sync over fixed op format — if policy allows) or better: **M3-sync** (Merkle+delivery+relay L1) then **M3-secure** (signatures/membership/crypto/H1)  
- Or keep one M3 but define **internal exit ramps** with shippable demos  
- Move Lean proofs and external audit to **M5+ / v0.2 assurance**, keep M5 GA focused on: GC safe, backup/restore, fuzzing of codecs/sync, operational packaging  
- Richtext should not share a GA gate with first safe GC  

---

### C-P6 — Acceptance of Critical issues is process-undefined

ISSUES says Critical items need “approved normative resolutions” for M0 exit. There is no definition of:

- Who approves  
- What artifact counts (SPEC patch? separate RFC? golden vectors only?)  
- Whether resolution text in ISSUES is enough without SPEC normative rewrite  
- How “resolved” items leave ISSUES into Decision Log without losing the normative prose  

Several C items still say **“Resolution: direction…”** (C4, C6) rather than a finished contract. Direction ≠ implementable norm.

**Fix:** For each C-item, require a checklist: (1) SPEC/RELAY normative text, (2) machine-readable schema or fixtures, (3) negative cases, (4) Decision Log entry, (5) issue closed/removed per policy.

---

### C-P7 — Product narrative still oversells post-plan features

The plan correctly defers ACLs, Richtext, mobile, hosted relay — but SPEC still presents:

- Full entity ACL section (§9.2) with quarantine semantics known broken under C6  
- “Web of trust” in §6.2 with **no milestone**  
- Richtext marked “*(Phase 3)*” (obsolete numbering; actually M5 non-goal for v0.1)  
- Competitive table listing **Lean 4** as if it were a ZeroDB property (“planned” footnote is easy to miss)  
- Architecture diagram / §4.3 still advertising **TCP** transport while RELAY-SPEC 0.2 removed TCP bindings  
- Node/Edge `_meta.signature?` still **optional** while operation signatures are mandatory (§2.3 vs §6.1)  

**Risk:** Contributors and future-you implement the aspirational narrative instead of the plan.

**Fix:** One pass of “plan hygiene”: mark every deferred feature `Status: post-v0.1` or move to appendix; kill Phase-N language; align diagrams with RELAY-SPEC 0.2; competitive table only claims shipped/proven properties.

---

## 4. High plan findings

### H-P1 — Schema and query decisions straddle M0/M1/M2 awkwardly

| Decision | Decide | Implement | Tension |
|----------|--------|-----------|---------|
| O2 schema SoT | M0 | M2 | M1 already requires `schema apply` + strict/schemaless |
| O3 query subset | M0 | M1 CLI query | M1 cannot ship a CLI query without a frozen grammar |

**Fix:** M0 must freeze a **minimal** query grammar and a **canonical schema IR** (even if TS sugar lands in M2). M1 implements IR + minimal query only; M2 is binding + sugar generation.

---

### H-P2 — H3 (delete/resurrection) is under-ranked for M1

H3 is “High” and scheduled M1, but incorrect cascade-as-generated-ops semantics **breaks SEC** (divergent op sets per peer view). That is Critical-class behavior for a graph database whose M1 exit includes “deterministic delete/referential-integrity state machine.”

**Fix:** Promote H3 to Critical for M1 exit (or fold into C-series as C9), and prefer **derived visibility** over multi-peer cascade generation unless a single deterministic emitter is specified.

---

### H-P3 — Cross-peer features split across M0/M4/M5 without a dependency graph

Examples:

- C2: epochs M0, cross-peer migration M4  
- C7: contracts M0, snapshots M4, GC M5  
- O7: deps scale M0/M5 interacting with C7  
- H6: peer protocol shared with relay M3, WebRTC M4  

The prose is mostly right, but there is **no explicit dependency DAG**. A reader can implement M4 snapshots without realizing C7 tail-boundary contracts were incomplete.

**Fix:** Add a one-page dependency diagram to SPEC §10 (or ISSUES) showing contract → implement milestones.

---

### H-P4 — No resource, timeline, or critical-path model

Zero dates, person-weeks, or “solo founder vs small team” assumptions. Without that, M0–M6 reads like a multi-year research OS project (Automerge/Loro-scale) marketed with GunDB-style accessibility.

**Fix:** Even rough ranges help: e.g. M0a–M0e = N months solo; M1 = …; identify the longest pole (likely Merkle+auth contracts or E2E key lifecycle).

---

### H-P5 — Two-language conformance is scheduled before language strategy is real

M0 wants two toy implementations; M3 wants golden/negative fixtures in two languages; first SDK language is TypeScript at M2; core is Rust.

**Gap:** Who writes the second language in M0 — a minimal TS encoder only? Python? The plan never says. If the answer is “Rust + TypeScript,” M0 implicitly requires a TS package before M2.

**Fix:** State: M0 second implementation = TypeScript pure encoder/decoder (no storage), checked in under `conformance/`, not the full SDK.

---

### H-P6 — INVARIANTS.md and BIBLIO.md do not support the rigor the plan claims

- `INVARIANTS.md` is three bullets + `<TODO>`  
- BIBLIO is a reading list without ties to SPEC claims or Lean goals  
- Plan promises Lean models in M0 and proofs in M5 without a written invariant list to prove  

**Fix:** Make a populated `INVARIANTS.md` an **M0 deliverable** (even informal): SEC, HLC monotonicity, signature meaning, membership meaning, no GC without frontier, etc.

---

### H-P7 — Relay levels vs milestone mapping is incomplete

RELAY-SPEC L0/L1/L2 exist, but SPEC M3 says “minimal relay level” without saying L1 vs L2. H11 durable ack is L2-relevant; snapshot bootstrap is M4/C7. Operators cannot know what “reference relay” means at M3 exit.

**Fix:** M3 reference relay = **L1** (forward + validate + ephemeral dedup) with explicit non-goals; L2 persistence + durable ack = M3-secure or M5.

---

### H-P8 — Early code already encodes wrong constants

| Item | SPEC / plan | Code today |
|------|-------------|------------|
| `PeerId` | BLAKE3(pubkey), 32 bytes stored | 16-byte random |
| Storage API | Oplog + State + atomic boundary (C8) | single `StorageAdapter` |
| Plan language | M0–M6 | “Phase 1” comment in storage |

Low severity if experimental; high if these types start appearing in fixtures.

---

## 5. Medium plan findings

1. **CLI surface in SPEC §5.1 is a product fantasy relative to M1.** `peers trust`, `keys *`, `sync connect`, Cypher mutual-followers are M2–M6 features listed as if they ship with the first CLI. M1 CLI should be a short explicit subset.
2. **O1 large payloads deferred to M4 while RGA ships M2.** Ordered sequences invite large lists; size limits (O6) should be provisional in M0/M1 even if blob strategy waits.
3. **O5 GunDB migration “won't do” is fine**, but marketing still says “succeed GunDB” without a migration story — set expectations in README.
4. **Plugins / custom CRDTs (M6)** with no sandbox policy until then — OK, but forbid extension points in M1–M2 APIs that would freeze a bad plugin ABI.
5. **Performance budgets** appear only as M2/M5 checkboxes with no numbers. Even provisional targets (ops/s local materialize, sync of 10k ops, WASM size vs O4) make exit gates real.
6. **No CI story** for golden vectors (where they live, how multi-language hashes are checked).
7. **Threat model table** still stronger than mechanisms for several rows pending H1/H8/H10 — plan should mark those rows “aspirational until M3.”
8. **Flutter/C ABI in M6** is decided (good) but C ABI stability before M6 is unplanned — if Node uses NAPI not C ABI, mobile may re-invent; decide whether a thin C ABI appears at M2 or only M6.
9. **“Carrier pigeon” Transport joke** is fine; TCP as first-class in architecture is not, post relay prune.
10. **No public contribution workflow** beyond “start with ISSUES.md” — for a project “seeking co-architects,” M0 needs an RFC template.

---

## 6. Plan / roadmap structural assessment

```
Current sequencing (good thesis):

  M0 contracts ──► M1 local ──► M2 SDK ──► M3 secure sync ──► M4 browser/P2P/evolution
                                                              │
                                                              └─► M5 GA/assurance ──► M6 ecosystem
```

| Milestone | Sequencing judgment | Scope judgment |
|-----------|---------------------|----------------|
| M0 | Correct position | **Overloaded** — must package-split |
| M1 | Correct | Slightly heavy (query+schema+all CRDTs+delete machine); still salvageable |
| M2 | Correct | Reasonable if M0 IR exists |
| M3 | Correct position | **Overloaded** — split sync vs secure or define ramps |
| M4 | Correct | Depends on M0 C7 snapshot contracts; OK if enforced |
| M5 | Too late for first user value, too early for Lean+audit | **Overloaded / wrong GA contents** |
| M6 | Correct parking lot | ACL redesign here is honest |

**Missing intermediate outcomes the plan should advertise:**

- “Single-node embedded graph CRDT DB” (M1) as a useful artifact  
- “Node library with parity fixtures” (M2)  
- “Trusted multi-writer todo sync” (M3) as true v0.1 product  

Without those, the plan only has meaning at M5 — a classic death march shape.

---

## 7. Consistency matrix (plan vs docs vs code)

| Claim | Where | Status |
|-------|-------|--------|
| M0 before format freezes | SPEC header, ISSUES, README | Policy clear; **process unclear** (C-P6) |
| M0 before all implementation | Decision log | **Contradicted** by crates + Phase-1 comments (C-P2) |
| Exemplar = acceptance from M1 | SPEC §10 | **False** given EXEMPLAR content (C-P4) |
| Entity ACLs deferred | ISSUES C6, README non-goals | **Narrative still present** in SPEC §9.2 (C-P7) |
| Signatures mandatory for sync | SPEC §6.1, trust model | **Node/Edge meta still optional** |
| Relay not implementation-ready | SPEC §4.4, README | Consistent and correct |
| TCP transport | SPEC §2.1, §4.3 | **Stale** vs RELAY-SPEC 0.2 |
| Richtext Phase 3 | SPEC §3.1 | **Stale** numbering; M5 / non-goal v0.1 |
| PeerId = BLAKE3(pubkey) | SPEC §6.1 | **Code uses random 16-byte** |
| Dual storage traits + atomicity | SPEC §7, C8 | **Code has minimal single trait** |
| Lean proofs differentiator | SPEC §1 bets, §11 table | **Not in plan until M5**; overclaimed |
| GC disabled until C7 tests | ISSUES C7 | Consistent and correct |
| v0.1 = Rust+SQLite+CLI+membership | README | **Membership is M3** — scope mismatch (C-P3) |

---

## 8. Recommended plan revisions (priority order)

### Immediate (before more code)

1. **Package-split M0** (C-P1) with ordered exits and golden-vector ownership.  
2. **Define v0.1 = M3 exit** (or rename releases) (C-P3).  
3. **Rewrite EXEMPLAR** as scenario IDs mapped to milestones (C-P4).  
4. **Implementation policy** for pre-M0 code; scrub Phase language (C-P2).  
5. **Define “approved resolution”** checklist (C-P6).  
6. **SPEC hygiene pass** for deferred features, signatures, TCP, Phase-N, competitive claims (C-P7).  

### Short-term plan edits

7. Promote or Critical-gate **H3** for M1 (H-P2).  
8. Freeze **minimal query grammar + schema IR** in M0 for M1 (H-P1).  
9. Split **M3** and thin **M5 GA** (C-P5).  
10. Populate **INVARIANTS.md** as M0a companion (H-P6).  
11. Map relay **L1 vs L2** to milestones (H-P7).  
12. Add rough **effort/critical path** note even if one person (H-P4).  

### Explicitly do *not* do yet

- Freeze CBOR op codecs from current types  
- Implement Merkle roots against unspecified bucket rules  
- Build entity ACL evaluation  
- Promise Lean-proved GA  
- Expand CLI to full §5.1 surface in M1  

---

## 9. Suggested replacement milestone sketch (informative)

Not a mandate — a concrete alternative that preserves the plan’s thesis:

| ID | Outcome | Exit evidence |
|----|---------|---------------|
| **M0a** | Op algebra + CBOR + preimages + DatastoreId in signed context | Golden bytes, two encoders |
| **M0b** | Schema IR + epochs + migration DSL (no JS closures) | Replay vectors across type change |
| **M0c** | Merkle canonical + sync state machine transcripts | Mismatch-recovery transcript fixtures |
| **M0d** | Author keys + membership capability format | Negative auth vectors |
| **M0e** | Groups, delivery, version policy | Crash/group/dedup contract tests (red) |
| **M1** | Local Rust+SQLite+CLI exemplar E1/E2 | Restart/replay/crash/delete SEC tests |
| **M2** | TS/NAPI parity | Byte-identical fixtures vs core |
| **M3s** | Sync over WS + L1 relay | 3-peer partition/reorder |
| **M3e** | Signatures, membership, E2E envelope, H1 | Malicious-peer + private-data E3/E4 |
| **M4** | Browser + WebRTC + migrations + snapshots | Upgrade matrix |
| **M5** | GC, backup, fuzz, ops packaging | Restore + forgotten-peer GC |
| **M5+** | Lean proofs, external audit, Richtext | Assurance release |
| **M6** | Mobile ABI, ACLs redesign, hosted relay, tools | Ecosystem |

---

## 10. Decision queue (plan owners should answer)

1. Is **v0.1** the first multi-peer secure release (M3) or the local CLI (M1)?  
2. Will M0’s second implementation be **TypeScript conformance only**?  
3. Is **derived delete visibility** acceptable to kill cascade-op SEC risk (H3)?  
4. Is **Lean** a marketing differentiator (keep in M5+) or a hard GA gate (reconsider)?  
5. Reference relay at first multi-peer: **L1 only** or L2 with durable ack?  
6. Pre-M0 code: **delete, quarantine, or formal experimental track**?  
7. What is the **approval body** for C-issue resolutions (solo maintainer RFC? external review?)  
8. Does exemplar “admin controls” mean **CLI on owner datastore** or **entity ACLs** (if the latter, it is M6)?  

---

## 11. Summary scorecard

| Dimension | Score | Note |
|-----------|-------|------|
| Strategic sequencing | **Strong** | Contracts → local → SDK → secure sync → evolve → assure → ecosystem |
| Scope control | **Weak** | M0/M3/M5 kitchen-sink gates |
| Acceptance falsifiability | **Weak** | Exemplar not a test plan |
| Spec/plan consistency | **Mixed** | Issues tracking good; narrative lag and Phase leftovers |
| Security realism | **Strong** | C6 deferral, GC off, untrusted relay |
| Execution readiness | **Poor** | No owners, estimates, package splits, process |
| Alignment with repo code | **Poor** | Scaffold contradicts “spec first” and some constants |
| Learn-from-prior-review | **Strong** | Codex findings absorbed into ISSUES + M0–M6 |

---

## 12. Conclusion

The project does **not** need another wholesale roadmap rewrite. It needs the M0–M6 skeleton to be turned into a **managed plan**:

- package the foundation,  
- name the first product,  
- make acceptance executable,  
- stop the aspirational SPEC from outrunning the milestone contracts,  
- and keep code exploratory until golden vectors exist.

Until C-P1 through C-P7 are addressed, treat claims of “Milestone N complete” as non-auditable.

---

*End of FINDINGS.GROK.md*
