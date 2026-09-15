# Read/write revocation — research notes (2026-09-15)

**Status:** research, not a contract. Nothing here changes AUTH/KERNEL/RELAY-SPEC. Every gap below cites the code or vector that shows it; every recommendation is a proposal for a later Decision Log act. Draft-1 / unfrozen throughout.

**Question:** what does it mean, in ZeroDB, to take *write* or *read* access away from a principal, what do we already do, where does it leak, and what should the H10-close / M3b-exit design be?

---

## 0. Summary

- **Write revocation is real and deterministic today** (`CapabilityRevoke` kind 7, causal predicate AUTH §4). Its one structural hole is the **unbounded concurrency window**: a revoked member who never causally acknowledges the revoke can keep authoring ops that every honest peer accepts as "concurrent" (§4.2, vector `AUTH-AUTHZ-005`). Only the honest relay's wall-clock filter stops them, and that filter is not integrity (AUTH §5) and is bypassed by direct DataChannel sync or a colluding relay.
- **Read revocation is honest-client-only until a key rotation happens**, and rotation is manual, unlinked to the revoke, and not validated by peers. Two admins rotating concurrently can hand the revoked member a key that opens post-revoke notes written by the admin who had not yet seen the revoke (§3 G4). Membership-at-open (`subject_may_open_at`) is an SDK courtesy, not a cryptographic boundary.
- **Device revocation (`KeyRecord kr = 1`) is verified and then ignored.** `apply_device_principal` returns early for `kr != 0`, and `wire_to_authz` sets `principal = author`, so no device ever loses authority peer-side. This is the C5/PKI hole from the other direction.
- **Relay bug:** a `CapabilityGrant` re-delivered after its `CapabilityRevoke` **un-revokes** it at the relay (`upsert_grant` writes `revoked = excluded.revoked` on conflict in both the memory and SQLite stores).
- **Recommendation (§5):** (R1) small fixes now — monotone relay `revoked`, self-revoke without admin, document `kr = 1` as a no-op and "both removed" for mutual admin revokes; (R2) contract change — treat a revoke as an **expiry with a grace window** for ops not in its causal future, plus a peer-side `ts ≥ max(deps.ts)` check, so the concurrency window is bounded and still a pure function of stated timestamps (same precedent as §4.3 expiry); (R3) H10 close — **revoke implies rotate**: peers validate `kr = 2` recipient sets against membership at the KeyRecord's causal position and reject encrypted data ops sealed under a key whose recipient set includes a subject revoked in the op's causal past (`KEY_STALE`); (R4) post-v0.1 — cascade via an issuer pointer on grants, BeeKEM-style log(n) rotation, acknowledgment-based finality.

---

## 1. What "revocation" has to mean here

Four scopes exist (AUTH §3.1): `write`, `admin`, `read`, `sync`. Revoking each means something different in a replicated, offline-first store:

| Scope | Revocation goal | Enforcement point | Achievable? |
|---|---|---|---|
| `write` | later ops by the subject are not materialized by any honest peer | peer predicate (integrity); relay filter (bandwidth) | yes, up to the concurrency window |
| `admin` | later grants/revokes/epochs/KeyRecords by the subject are rejected | same | yes, same window; races with the subject's own concurrent grants |
| `read` (encrypted props) | subject cannot open values sealed after the revoke | key rotation + peer validation of key use | yes for *post-revoke* values; never for values already sealed under a key they hold |
| `read` (plaintext props, structure, metadata) | — | none | **no** by construction: replicated plaintext cannot be un-replicated (SPEC §6 threat model; ISSUES C6). H8 metadata privacy is separate |
| `sync` | relay stops serving the subject | relay admission (`datastore_allowed`) | availability only; a subject with any other peer or replica still has the bytes |

Adversaries to hold in mind:

1. **Honest-but-offline member** — revoked while disconnected; keeps writing in good faith; must converge on reconnect without a permanent fork.
2. **Hostile revoked member** — full replica, cached keys, can sign any timestamp, can pick any `deps`, may collude with a relay or sync directly over DataChannel.
3. **Colluding relay** — forwards everything (`Relay::memory_colluding` already models this).
4. **Compromised device of a live member** — motivates post-compromise security (PCS) via rotation; forward secrecy (FS) for history is not a goal (history must stay readable to members).
5. **Revoked admin** — combination of 2 with control-plane authority.

---

## 2. What exists today (evidence)

### 2.1 Write / admin revocation — peer side

- Body and semantics: AUTH §3.2 (kind 7 body `{grant, reason}`), §4.1 rule 3 ("no `CapabilityRevoke` targeting G in O's causal past"), §4.2 concurrent-with-revoke **accepted**.
- Predicate: `zerodb-core/src/auth.rs` `authorize()` — builds `past = causal_past(candidate)`, collects grants for the principal from `past`, collects revokes from `past`, accepts if any live grant carries the needed scope. Purely causal; the only wall-clock input is grant `expiry` vs `candidate.ts_physical_ms` (§4.3).
- Storage wrapper: `zerodb-storage/src/authz.rs` `authorize_wire` (solo-device: `principal = author`).
- Honest local ops dep every applied control op (`control_dep_hex`: genesis + last 63 control ops), so an honest client that has *applied* the revoke always puts it in its causal past. Missing deps are not satisfied on ingest (`lib.rs` ~3199), so a hostile peer cannot dep an op it does not carry.
- Vectors: `AUTH-AUTHZ-004-revoked` (revoke in causal past → `AUTH_REVOKED`), `AUTH-AUTHZ-005-concurrent-revoke` (revoke not in deps → `ok`). Tests: `zerodb-storage/tests/e5_membership.rs`.
- Required scope for kinds 6/7 is `admin` (`required_scope`); `delegable` is parsed and **never enforced** anywhere (grep: only appears in literals).

### 2.2 Write / read revocation — relay side

- `KnownGrant { revoked, expiry, scopes }` persisted (`store.rs` `membership_grants`). `apply_membership_from_op` upserts on kinds 0/6 and sets `revoked` on kind 7 (`session.rs` ~1540–1595).
- `author_write_allowed` (~1503): subject has a grant with the needed scope, `!revoked`, and `physical_ms < expiry`. **Wall-clock / flag only, not causal.** A well-formed op that honest peers would accept as concurrent is `REJECT/AUTHZ` at an honest relay.
- `datastore_allowed` (~1120): re-evaluated per request, so a revoke **does cut live sessions** on their next SYNC/OPS/walk (Decision Log 2026-08-17). SUBSCRIBE token check (AUTH §3.3) verifies `read + sync` and the revoked flag.
- Empty grant table ⇒ open datastore (`grants.is_empty() → Ok(true)`); the first grant fail-closes. Documented.

### 2.3 Read revocation — encryption

- Envelope: KERNEL §7, AAD = slot context. Group key: `KeyRecord kr = 2`, one X25519 wrap per recipient, wrap shape draft (32/32/24/48).
- `rotate_group_key(recipients)` (`lib.rs` ~612): mints a fresh key, keeps old keys in the ring, publishes wraps for the **caller-supplied** recipient list. Nothing derives the list from membership, nothing checks it.
- Current-key adoption on ingest: `adopt_as_current = wire_author_is_admin(...)` — the **latest applied admin KeyRecord** wins, whatever its causal position.
- `subject_may_open_at` (~3819): at materialize time, the *local* peer checks it held a `read` grant with `grant.ts.p ≤ note.ts.p`, not expired, and no revoke with `revoke.ts.p ≤ note.ts.p`. Wall-clock compare on stated timestamps; enforced only against the local identity; keys are still in the ring. Honest-client only.
- `decrypt_oracle` proves non-recipients cannot open post-rotation ciphertext (`e6_rotate_after_revoke_blinds_b_a_still_reads`, `e6_offline_revoke_note_without_rotate_stays_closed`, `e6_membership_at_open_blinds_same_key_after_revoke`).
- History for new joiners: `distribute_group_key` wraps only the current key. There is no policy for sharing older keys.

### 2.4 Device revocation

- `kr = 1` body validated (`validate_device_key_record`, `verify_device_cert` requires `revoke_of`; vector `AUTH-CERT-004-revoke`).
- `apply_device_principal` (~3781): `if cert.kr != KR_DEVICE_CERT { return Ok(()) }` — **no state change** for a revoke.
- `wire_to_authz`: `principal: author`. Principal resolution from certs is not wired into the predicate, so a device revoke cannot affect authorization. ISSUE C5 (record 4355) already notes the mirror problem: any write member can mint a cert binding a victim device to themselves.

### 2.5 Expiry (lease model)

Grants carry `expiry` and the predicate enforces it against the op's own `ts.p` (§4.3, `AUTH_EXPIRED`). This is the existing precedent for a **deterministic wall-clock bound** and is the template for R2.

---

## 3. Gaps

Severity: **P0** = integrity/confidentiality hole reachable by a hostile revoked member; **P1** = convergence/consistency defect or honest-path bug; **P2** = missing policy / documentation.

### G1 (P0) Unbounded concurrency window for a revoked writer
A hostile revoked member never includes the revoke in `deps`. Every op they sign is "concurrent" with the revoke forever and `authorize()` accepts it (§4.2, `AUTH-AUTHZ-005`). Over a colluding relay or a direct DataChannel session, honest peers materialize those ops. The honest relay's `author_write_allowed` is the only thing stopping this on the relay path, and AUTH §5 says the relay "MUST NOT [be relied] on for integrity". This is the same problem the literature calls the concurrent-revocation problem (p2panda, Keyhive, Policy-CRDT — §4).

There is also no peer-side check that an op's `ts` is ≥ the `ts` of its `deps` (I-5 is an issuing-device rule in KERNEL §5, not a receiver validation). So a hostile member can also **back-date** freely below any wall-clock bound we might add, unless that check is added.

### G2 (P0) Revoked-admin races and no cascade
- An admin being kicked can, concurrently, issue a fresh grant to themselves or to a sock-puppet principal. Both the grant and their subsequent ops are concurrent with the revoke and accepted. The new grant is not targeted by the revoke (revokes name a grant `OpId`, not a subject).
- Grants issued *under* a delegated/admin grant survive revocation of that grant (no issuer pointer in the grant body; UCAN has the same non-cascading semantics by choice; Keyhive cascades).
- `delegable` is dead: kinds 6/7 always require `admin`.

### G3 (P1, decide) Mutual / concurrent admin revocation
Two admins revoke each other concurrently. Each revoke is authorized in its own causal past, so **both are removed**. That is a defensible Byzantine-safe rule (p2panda's "remove both"), but it is currently an accident of the predicate, not a decision. Related: the founder holds a synthetic grant at the genesis `OpId` — it can be named by a `CapabilityRevoke` today and the predicate would honor it. Last-admin lockout (all admins revoked, founder key lost) has no recovery path (AUTH §1.3 says root loss is out-of-band).

### G4 (P0) Read revocation leaks through concurrent or stale rotation
- **Concurrent rotation:** admin A revokes B and rotates to key K_A (recipients: A, C). Admin C, not having seen the revoke, rotates to K_C (recipients: A, B, C). C's post-rotation notes are sealed under K_C and B opens them. At A, adoption is "latest applied admin KeyRecord", so A may even start sealing under K_C. Nothing validates that a KeyRecord's recipients are members at its causal position, and nothing ties a data op's `key_id` to the revokes in that op's causal past.
- **Stale key:** any writer who has seen the revoke but not a rotation keeps sealing under the old key (`META_KEY_CURRENT` unchanged). `e6_membership_at_open_blinds_same_key_after_revoke` shows the ciphertext *is* openable by B; only B's honest client refuses.
- **Rotation is optional:** `revoke_membership` does not rotate; the caller must remember.

### G5 (P1) Relay and peers accept different sets
The relay decides by flag + wall-clock (`author_write_allowed`), peers by causal past. An honest-but-offline member's ops written before they learned of the revoke are accepted by peers (§4.2) and rejected by the honest relay — so the relay's Merkle root and the peers' roots diverge, and the relay is not a catch-up source for those ops. This is the "synchronized set" problem flagged in `plan/archive/FINDINGS.CODEX.2026-07-20.md` and still open. The same ts-vs-causal mismatch exists in `subject_may_open_at`.

### G6 (P1) Relay un-revoke on grant re-delivery
`upsert_grant` on conflict sets `revoked = excluded.revoked` (memory: `HashMap::insert` replaces; SQLite: `ON CONFLICT ... revoked=excluded.revoked`). A kind-6 op that arrives (or is re-uploaded — clients upload all local ops on every connect, PERF P0-3) after its kind-7 revoke resets `revoked` to false. `revoked` must be monotone.

### G7 (P1) Device revoke is a no-op
See §2.4. Until the two-key principal/device path resolves `principal` from certs, `kr = 1` should be documented as advisory, and the SDK should not present it as revocation.

### G8 (P2) No self-revoke ("leave")
`reason = 1 (leave)` exists, but kind 7 needs `admin`, so a plain member cannot leave. A subject revoking their own grant should be allowed without admin.

### G9 (P2) Key-ring retention, PCS, FS, joiner history
Old keys live forever in every member's ring (needed to read history). A compromised device therefore leaks all history ever sealed — inherent to "history stays readable", worth stating. PCS is obtained only by rotation. New joiners get only the current key; whether they should get history keys is an unstated policy.

### G10 (P2) Concurrent-with-revoke ops are invisible to the app
Peers accept them silently. The revoker's client has no signal ("the revoker's client may surface concurrent late writes" — AUTH §4.2 — is not implemented). p2panda exposes exactly this set via an events API.

---

## 4. Prior art (what to borrow, what to avoid)

| System | Model | Revocation of write/admin | Revocation of read | Borrow |
|---|---|---|---|---|
| **DCGKA** — Weidner, Kleppmann, Hugenroth, Beresford, CCS 2021 | decentralized continuous group key agreement over a causal broadcast; no server | member removal is a group op; concurrent ops merge | removal triggers key update; FS + PCS proven; practical cost | the framing that removal *is* a key-agreement event, and the concurrency-tolerant merge |
| **Keyhive / BeeKEM** — Ink & Switch | convergent capabilities (delegation chains with CRDT state) + group CRDT + causal keys | signed delegation chain; cascade: "a principal can only decrypt if their authorization ancestors remain unrevoked" | BeeKEM: blank leaf + path, any member re-keys; log(n) per removal; concurrent updates kept as *conflict keys*; "remove paths are blanked after all other concurrent operations are merged" | cascade semantics; conflict-keys idea for concurrent rotations; log(n) rotation later |
| **p2panda access-control CRDT** (2025-08) | DAG of signed group ops, topological order | three candidate rules for concurrent removals: seniority (Sybil-prone), **remove both**, higher-hash; open by threat model | — | "remove both" default; **events API** exposing ops concurrent with a removal; acknowledgment-based finality as the eventual window bound |
| **Policy-CRDT** (remove-wins) | set of policies, remove-wins | any revocation in global history eventually suppresses the policy | — | confirms remove-wins is the safe default for admin races |
| **UCAN revocation** | certificate capabilities, block-list | issuer may revoke what it issued; **no cascade** ("two tickets"); revocations immutable, monotone set; accept revocations that arrive before the delegation | — | monotone revocation set (fixes G6); explicit non-cascade as a *choice* to contrast with |
| **Matrix Megolm** | sender-keys, per-room sessions | server-authoritative membership | rotate session on membership change, plus every 100 msgs / 1 week; history visibility is a room setting | "rotate on membership change" as a *mandatory* rule; a history-visibility policy for joiners |
| **MLS (RFC 9420) / TreeKEM** | server-ordered CGKA | Remove proposal + Commit | log(n) re-key; needs total order | not directly (needs a sequencer); BeeKEM is the decentralized adaptation |
| **Tahoe-LAFS** | capabilities *are* keys | none (re-encrypt to a new cap) | none | the honest statement that "read revocation = re-key + stop sharing" |

Sources: [DCGKA (ePrint 2020/1281)](https://eprint.iacr.org/2020/1281), [CCS'21 DOI](https://dl.acm.org/doi/10.1145/3460120.3484542), [Kleppmann summary](https://martin.kleppmann.com/2021/11/17/decentralized-key-agreement.html), [Keyhive notebook](https://www.inkandswitch.com/keyhive/notebook/), [BeeKEM entry](https://www.inkandswitch.com/keyhive/notebook/02/), [cross-fork security entry](https://www.inkandswitch.com/keyhive/notebook/06/), [BeeKEM explainer](https://meri.garden/posts/a-deep-dive-explainer-on-beekem-protocol/), [p2panda access-control notes](https://p2panda.org/2025/08/27/notes-convergent-access-control-crdt.html), [Policy-CRDT](https://www.researchgate.net/publication/398840744_Policy-CRDT_Conflict-Free_Replicated_Data_Type_with_Remove-Wins_Strategy_for_Convergent_Access_Control_in_Asynchronous_Environments), [UCAN revocation](https://ucan.xyz/revocation/), [Matrix Megolm](https://spec.matrix.org/v1.17/olm-megolm/megolm/), [Towards system-oriented formal verification of local-first access control (arXiv 2604.23560)](https://arxiv.org/pdf/2604.23560).

---

## 5. Design options and recommendation

### 5.1 Bounding the concurrency window (G1, G5)

| Option | Deterministic (I-1)? | Stops hostile? | Stops honest-offline fork? | Cost |
|---|---|---|---|---|
| (a) keep §4.2 as is | yes | no | n/a | 0 |
| **(b) revoke acts as expiry + grace** — for an op *not* in the revoke's causal future, accept only if `op.ts.p < revoke.ts.p + revoke_grace_ms`; plus peer-side rule `op.ts.p ≥ max(deps.ts.p)` | yes: pure function of stated timestamps, same as §4.3 | bounded to the grace window (see caveat) | yes: honest-offline ops inside grace converge; outside grace they are rejected everywhere the same way | one registry field (`revoke_grace_ms`, propose `= max_drift_ms` 60 000), predicate change, new vectors |
| (c) control-frontier binding — reject if the op's newest control dep is more than k control ops behind the receiver | **no** (depends on receiver state) | — | — | reject |
| (d) acknowledgment finality (p2panda) — quorum of members acks a frontier; ops behind it are Byzantine | yes once acks are ops | yes | yes | needs a quorum protocol; post-v0.1 |
| (e) surface `CONCURRENT_WITH_REVOKE` to the app, let the revoker tombstone | yes | partial (app-level) | n/a | small; complements any of the above |

**Caveat on (b):** a hostile member can still dep only *old* ops and pick `ts.p` just below `revoke.ts.p + grace`, so (b) bounds the window to *grace* seconds after the revoke's own stamp — it does not shrink it to zero. That is the same guarantee expiry gives today and is what every listed prior-art system without consensus settles for. Combined with the honest relay filter it is adequate for v0.1.

**Recommend (b) + (e).** (b) reuses the §4.3 pattern and keeps every peer computing the same answer from the op alone. (e) makes the residual window visible.

### 5.2 Admin races and cascade (G2, G3, G8)

1. **Revokes may target a subject, not only a grant.** Add `subject: PrincipalId | null` to the kind-7 body (draft-1, unfrozen, so this is a body change with vectors, not a freeze). A subject-revoke defeats every grant for that subject in the op's causal past *and* every grant to that subject that is concurrent with the revoke (remove-wins). This closes the "kicked admin grants themselves again" race without cascade machinery.
2. **Cascade via issuer pointer** (post-v0.1, needed anyway to make `delegable` real): grant body gains `via: OpId` of the issuing principal's authorizing grant; a grant is live iff its `via` chain is live (Keyhive rule). UCAN's "two tickets" exception falls out naturally: a subject with an independent live grant keeps access.
3. **Mutual concurrent admin revokes: both removed.** State it in AUTH §4.2 as the chosen rule; it is what the predicate already does and it is the Byzantine-safe choice. Founder exemption: a kind-7 naming the genesis `OpId` (synthetic founder grant) is `CAP_INVALID`. Ownership transfer stays post-v0.1.
4. **Self-revoke:** kind 7 with `reason = 1` whose target grant's `subject == author principal` requires no `admin`.

### 5.3 Read revocation: "revoke implies rotate" (G4, G9, G10)

Rules, all checkable by every peer from the op and its causal past:

1. **KeyRecord recipient validation.** A `kr = 2` KeyRecord is valid iff every wrap `recipient` holds a live `read` grant at the KeyRecord's causal position (same predicate as §4, evaluated per recipient). Otherwise `KEY_WRAP_NONMEMBER` (named reject; not materialized).
2. **Key freshness for data ops.** An encrypted `SetProperty` whose `key_id` resolves to a KeyRecord K is valid iff no subject in K's recipient set has a `CapabilityRevoke` (grant- or subject-targeted) in the data op's causal past that is not itself in K's causal past. Otherwise `KEY_STALE`. This turns "membership at open" from an honest-client check into an integrity rule: a note sealed under a key the revoked member holds, by an author who has seen the revoke, is never materialized anywhere.
3. **Auto-rotate.** `revoke_membership` performs `rotate_group_key` for the surviving `read` members in the same commit (one kind-7 then one kind-8, the kind-8 depping the kind-7). Non-admin writers who observe a revoke with no fresh key **block** encrypted writes (`KEY_STALE` locally) instead of sealing under the old key.
4. **Concurrent rotations are all current** (BeeKEM conflict keys). The ring keeps every key; `META_KEY_CURRENT` becomes a set; a writer seals under *any* key that passes rule 2, preferring the causally newest. The next admin rotation deps all of them and collapses the set.
5. **Recipient set derivation.** `rotate_group_key()` derives recipients from applied membership (`read` grants live at the local frontier), with an explicit override for tests. Recipients are `(PrincipalId, device pk)` pairs; the two-key path still ECDHs to the device key.
6. **History for joiners.** `CapabilityGrant` gains `history: bool` (default false). If true the granting admin also wraps all prior keys for the subject (one KeyRecord, many `key_id`s — body change) — Matrix's history-visibility, per grant.
7. **Documented limits.** No FS for history (every member's ring holds all keys); PCS only after a rotation; plaintext props, node/edge structure, labels, authors, timestamps are never revocable (SPEC §6, H8).

Cost: O(members) wraps per rotation in one op (≤ 64 KiB op limit ⇒ roughly 400 recipients at 160 B/wrap before batching is needed). Enough for v0.1; BeeKEM log(n) is the scale path.

### 5.4 Relay alignment (G5, G6)

- `revoked` monotone: `upsert_grant` must `revoked = revoked OR excluded.revoked`; memory store merges instead of replacing. Test: revoke, re-upload grant, SUBSCRIBE still denied.
- Relay applies the same **grace rule** as 5.1(b) when it has the revoke's timestamp, so honest-offline ops inside grace are stored and forwarded and the relay/peer accepted sets agree. Outside grace both reject. This closes the FINDINGS.CODEX "synchronized set" gap for the revocation case.
- Keep `datastore_allowed`'s per-request cut for `read`/`sync` (availability); no change.

### 5.5 Device revocation (G7)

Blocked on the two-key principal path (C5). Until then: AUTH §1.3 should say `kr = 1` is accepted, verified, and **not enforced**; SDK must not expose it as "revoke device". When principal resolution lands, a device revoke removes `(principal, device)` from the cert set causally, with the same grace rule as 5.1(b).

---

## 6. Proposed sequencing

| Step | Content | Touches | Gate |
|---|---|---|---|
| **R1** small fixes | monotone relay `revoked`; self-revoke; AUTH text for "both removed", founder un-revocable, `kr = 1` no-op; `CONCURRENT_WITH_REVOKE` event | `zerodb-relay/src/store.rs`, `zerodb-core/src/auth.rs`, `doc/AUTH.md` | e5 tests; no vector byte change |
| **R2** window bound | `revoke_grace_ms` in registry; predicate 5.1(b) + `ts ≥ max(deps.ts)`; subject-targeted revoke; vectors `AUTH-AUTHZ-006..008` red→green in both runners; relay grace | AUTH §3.2/§4.2/§4.3, registry, TS runner | M3b remainder row |
| **R3** H10 close path | 5.3 rules 1–5; `KEY_WRAP_NONMEMBER`, `KEY_STALE`; auto-rotate; e6 vectors for concurrent-rotation leak and stale-key reject | `zerodb-storage/src/lib.rs`, AUTH §8, KERNEL §7 | H10 close needs this plus wrap-body decision |
| **R4** post-v0.1 | `via` cascade, `history` on grants, BeeKEM rotation, ack finality | new contract text | M5/M6 window with C6 |

---

## 7. Decisions needed before R2/R3

1. Grace window value and whether it is one registry constant or per-datastore (genesis field).
2. Subject-targeted revoke (5.2.1) vs cascade-only (5.2.2) for v0.1.
3. "Both removed" for mutual admin revokes — confirm.
4. Auto-rotate on revoke as SDK default, or opt-in.
5. Joiner history policy default (`history: false`).
6. Whether relay adopts the grace rule (5.4) or stays a stricter filter with documented divergence.

---

**Not claimed:** no contract change, no vector change, no code change in this document; H10 not closed; M3b exit not claimed; C5/PKI not resolved; H8 direction not decided; format not frozen; crates.io/npm unpublished.
