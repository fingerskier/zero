# ZeroDB Authorization Specification — Identity, Genesis & Membership (M0d)

**Version:** 0.1.0-draft
**Status:** normative (**draft-1 profile**). The M0d exit checklist closed 2026-07-18 ([ISSUES Decision Log](ISSUES.md)): every rule below is backed by golden vectors green in **both** conformance runners and CI-blocking ([conformance/](../conformance/README.md)); the corpus grows as later packages land. All formats are **draft-1, unfrozen** until an explicit Decision Log freeze names a versioned profile — until then, a byte-affecting change re-runs the resolution checklist. **On-wire enforcement** of admission and author resolution ships M3b; this document is the contract-model layer (SPEC §10 two-layer rule).
**Authority:** this document owns principal/device identity, device certificates, genesis authority, membership capability grant/revoke, and the per-operation authorization predicate (ISSUES C4 admission half, C5; DQ-1/DQ-2/DQ-3). Operation envelope, preimages, and `DatastoreId` derivation shell come from [KERNEL.md](KERNEL.md) (kinds 0, 6, 7, 8 bodies filled here). [SPEC §6](SPEC.md) remains the informative overview; on conflict this document wins for authorization semantics.

Keywords MUST/SHOULD/MAY per RFC 2119. Invariant references (I-*) per [INVARIANTS.md](INVARIANTS.md).

---

## 1. Two-level identity (DQ-1)

### 1.1 Principals and devices

- A **Principal** is a self-certifying Ed25519 **root** keypair. `PrincipalId = BLAKE3(root public key)` (32 B; registry `PrincipalId`). The root key is **cold**: it signs only control artifacts (device certificates, ownership transfer), never data operations.
- A **Device** is an Ed25519 keypair authorized by a signed **device certificate** from the principal root. `PeerId = BLAKE3(device public key)` (32 B) — unchanged from KERNEL §2 / SPEC §6.1.
- **Every data and control operation is authored by a device key** (`author` = `PeerId` of the signing device). Verification resolves the device certificate chain to a principal.
- **Membership subjects are principals**, not devices. Rotating or adding a device never churns datastore membership (HX-05).

Solo-device users generate both keys transparently; the SDK hides the split.

### 1.2 Device certificate (`KeyRecord` kind 8, subtype `device_cert`)

`KeyRecord` body (canonical CBOR map; unknown keys reject):

```
{
  "kr":  uint (= 0 for device_cert; 1 = device_revoke; 2 = group_key — M3b)
  "device":  bytes(32)     // Ed25519 device public key
  "principal": bytes(32)  // PrincipalId (= BLAKE3(root pk); also carried for lookup)
  "root_pk": bytes(32)    // principal root public key (so PrincipalId is re-derivable)
  "issued":  uint         // HLC physical_ms at issue (informational; not load-bearing for validity)
  "expiry":  uint | null  // physical_ms exclusive upper bound, or null = no expiry
  "revoke_of": bytes(32) | null  // for kr=1: PeerId (or device pk hash) being revoked
  "cert_sig": bytes(64)   // Ed25519 signature by root_pk over cert-preimage
}
```

- **cert-preimage** = `domain("device_cert") ‖ canonical CBOR of the map without "cert_sig"`.
- **Verification of a device cert:**
  1. `PrincipalId` MUST equal `BLAKE3(root_pk)`.
  2. `PeerId` of the device MUST equal `BLAKE3(device)`.
  3. `cert_sig` MUST verify under `root_pk` over the cert-preimage.
  4. For `kr = 0` (`device_cert`): `revoke_of` MUST be null.
  5. For `kr = 1` (`device_revoke`): `revoke_of` MUST be the 32 B `PeerId` (or device pk — same hash) being revoked; the revoke is signed by the same root.
- **Historical lookup:** an operation with `author = P` verifies if **any** non-revoked `device_cert` for device public key `D` with `BLAKE3(D) = P` is present in the applied control set (or is carried inline — see §1.4). Revocation is causal (same rule as membership, §4): a revoke defeats only ops causally after the revoke record.

### 1.3 Device rotation and root loss

- **Rotation:** issue a new `device_cert` from the root; optionally emit `device_revoke` for the old device. Old-device ops remain valid under historical authorization if they were causally before the revoke.
- **Root compromise/loss (v0.1):** no in-protocol recovery. Re-admission of a new principal is an out-of-band founder/admin action (documented product limitation). Social recovery / multi-sig roots are post-v0.1.

### 1.4 Carrying keys with operations (C5)

Forwarded history (bridging, relay-to-relay) cannot recover a public key from `PeerId` alone. Implementations MUST satisfy **one** of:

1. **Inline:** each batch of operations from an author includes a resolvable `device_cert` `KeyRecord` for that author (same `OPS` message or causal dep), or
2. **Prior control plane:** the receiver already has an applied non-revoked cert for that author in the datastore control set.

**Unresolved-author rule:** if step (2) of the application pipeline (KERNEL §6) cannot resolve a device public key for `author`, the operation enters the **bounded quarantine buffer** with named outcome `AUTHOR_UNRESOLVED` (retryable while buffer holds). On buffer overflow or timeout: reject with `AUTHOR_UNKNOWN` — never silently materialize, never forward-as-valid (closes C5 / HX-02). Relay checks remain a bandwidth filter only (§5).

---

## 2. Genesis and root authority (DQ-2)

### 2.1 Genesis body (kind 0)

```
{
  "founder": bytes(32)    // PrincipalId of the founding principal
  "salt":    bytes(16)    // cryptographically random
  "init_ep": uint         // initial schema epoch reference (0 = schemaless)
  "fmt_v":   uint         // operation_format_version at creation (must equal envelope "v")
}
```

Envelope rules (KERNEL §4.6):

- At signing time, `ds` is **all-zero** (32 zero bytes).
- `author` is a device `PeerId` belonging to `founder` (device cert MUST be verifiable; the genesis op itself MAY carry the founding device cert as a causal dependency or co-batch `KeyRecord`).
- `DatastoreId = BLAKE3(domain("genesis") ‖ id-preimage-of-genesis)` where id-preimage is KERNEL §4.4 (map without `id`/`sig`, including the zero `ds`).
- Every subsequent operation in this datastore MUST carry that `DatastoreId` in `ds`. A second genesis claiming the same id is impossible by construction (preimage includes the salt and founder). Peers MUST reject any operation whose `ds` does not match the sole accepted genesis for that store.

### 2.2 Root authority

- The **founder** is root authority for the datastore.
- Every **control-plane** operation — `CapabilityGrant`, `CapabilityRevoke`, `SchemaEpoch`, `KeyRecord` (group keys), ownership transfer (post-v0.1 body), `Checkpoint` (M0f) — MUST chain by signature to the genesis authority:
  - authored by a device of the founder principal, **or**
  - authored by a device of a principal holding an **admin** capability (§3) whose grant is valid in the op's causal past (§4).
- Data operations (`CreateNode`, `CreateEdge`, `SetProperty`, `Tombstone`) require **write** membership (§3), not admin.
- Ownership transfer = a signed transfer record; **v0.1 has exactly one root at a time** (no k-of-n multi-owner).

---

## 3. Membership capabilities (C4 admission)

### 3.1 Grant body (kind 6 `CapabilityGrant`)

```
{
  "subject":   bytes(32)   // PrincipalId
  "scopes":    [uint, ...] // non-empty; registry below
  "expiry":    uint | null // physical_ms exclusive, or null
  "delegable": bool        // if true, subject may grant a subset of these scopes
  "ds_bind":   bytes(32)   // MUST equal envelope "ds" (redundancy for relay-side checks)
}
```

**Scope registry (v1):**

| Tag | Name | Meaning |
|-----|------|---------|
| 0 | `write` | author data operations |
| 1 | `admin` | issue grants/revokes, schema epochs, control plane |
| 2 | `read` | subscribe / pull history (admission credential for relays) |
| 3 | `sync` | participate in sync (default with `read` for v0.1 peers) |

Unknown scope tags: **reject** the grant (`CAP_SCOPE_UNKNOWN`).

**Implicit founder grant:** the founder principal holds `{write, admin, read, sync}` from genesis with no expiry and `delegable: true`, without a separate grant op. Models treat this as a synthetic grant at genesis `OpId`.

### 3.2 Revoke body (kind 7 `CapabilityRevoke`)

```
{
  "grant":  bytes(32)   // OpId of the CapabilityGrant being revoked
  "reason": uint        // 0 = unspecified, 1 = leave, 2 = kick, 3 = expiry-enforcement
}
```

### 3.3 Relay-verifiable admission credential

A **membership capability token** for SUBSCRIBE (RELAY-SPEC) is a compact proof a relay can check without schema knowledge:

```
{
  "ds":      bytes(32)
  "subject": bytes(32)   // PrincipalId
  "grant":   bytes(32)   // OpId of CapabilityGrant (or genesis OpId for founder)
  "scopes":  [uint, ...] // claimed scopes (subset of grant)
  "device":   bytes(32)   // device public key of the connecting peer
  "cert":    <device_cert KeyRecord body or OpId of applied cert>
  "sig":     bytes(64)   // device signature over token-preimage
}
```

- **token-preimage** = `domain("relay_auth") ‖ canonical CBOR of the token map without "sig"`.
- Relay verification (filter only — not load-bearing for peer integrity, DQ-3):
  1. Resolve device cert; `BLAKE3(device) = PeerId` of the AUTH'd connection.
  2. Grant exists for `(ds, subject)` with claimed scopes ⊆ grant.scopes (founder synthetic OK).
  3. Grant not known-revoked *to the relay* (relays MAY cache; false negatives only affect availability).
  4. Token `sig` verifies under `device`.

Peers MUST still run the full §4 predicate on every applied op. A malicious relay that admits a non-member cannot force honest peers to materialize that member's ops.

---

## 4. Per-operation authorization predicate (DQ-3)

Applied at pipeline step **(3)** (KERNEL §6), after structural decode and signature verification, before dedup. Inputs: the candidate op, the set of already-applied ops (control + data), and resolved author principal.

### 4.1 Validity rule

An operation **O** is authorized iff **all** of:

1. **Signature / author chain:** `O.sig` verifies under device public key `D` with `BLAKE3(D) = O.author`, and a non-revoked device cert binds `D` to principal `Π` (or `AUTHOR_UNRESOLVED` / `AUTHOR_UNKNOWN` per §1.4).
2. **Datastore bind:** `O.ds` equals the datastore's genesis-derived id (except genesis itself).
3. **Membership at grant-time (causal):**
   - If `O` is genesis: author device must belong to `body.founder`.
   - Else: there exists a grant **G** (real or synthetic founder grant) such that:
     - `G.subject = Π`,
     - `G` is in `O`'s causal past (transitively via `deps`),
     - required scope for `O.kind` is in `G.scopes` (data → `write`; control → `admin` except `KeyRecord` device_cert/device_revoke which the principal root signs off-envelope — device certs are verified by root sig, not by admin grant of the authoring device alone; see note),
     - no `CapabilityRevoke` targeting `G` is in `O`'s causal past,
     - if `G.expiry` is non-null, `O.ts.physical_ms < G.expiry` (wall-clock advisory only for expiry; see §4.3).
4. **Control-plane chain:** for control ops other than genesis, the authoring principal MUST hold `admin` under (3) **or** be the founder.

**Note — device cert ops:** a `KeyRecord` with `kr ∈ {0,1}` is valid if its `cert_sig` verifies under the named `root_pk` and the principal matches; the *envelope* author should be a device of that principal (self-attestation of publication). Group-key `KeyRecord`s (`kr = 2`) require `admin` or key-distribution policy (M3b).

### 4.2 Concurrent-with-revocation

Ops concurrent with a revoke (neither is in the other's causal past) are **accepted** if they satisfy (3) without that revoke. Deterministic: every peer computes the same answer from `deps` (I-1). The revoker's client may surface concurrent late writes; no op flips valid→invalid when the revoke later arrives (avoids C6-style reevaluation).

### 4.3 Expiry

Capability `expiry` is compared against the op's own `ts.physical_ms`. Clock attacks on expiry are bounded by H1 peer-side skew policy (M3b); M0d models treat expiry as a pure function of stated timestamps in the fixture.

### 4.4 Named outcomes

| Outcome | When |
|---------|------|
| `AUTHOR_UNRESOLVED` | author key not yet available; buffered |
| `AUTHOR_UNKNOWN` | buffer overflow/timeout; reject |
| `AUTH_SIG_INVALID` | envelope signature fails under resolved device key |
| `AUTH_NO_MEMBERSHIP` | no valid grant in causal past for required scope |
| `AUTH_REVOKED` | grant revoked in causal past |
| `AUTH_EXPIRED` | grant expiry ≤ op timestamp |
| `AUTH_WRONG_DATASTORE` | `ds` mismatch / non-zero ds on genesis / wrong bind |
| `AUTH_NOT_ADMIN` | control op without admin/founder |
| `CAP_SCOPE_UNKNOWN` | grant carries unknown scope tag |
| `CAP_INVALID` | grant/revoke/cert structural failure |

Rejects are **not** materialized and MUST NOT be forwarded as valid (I-8, I-9).

---

## 5. Relay vs peer roles

| Check | Peer | Relay |
|-------|------|-------|
| Envelope decode + limits | MUST | MUST |
| OpId recompute | MUST | MUST |
| Author signature (resolved key) | MUST | SHOULD when key known; MUST NOT reject solely because author ≠ transport sender (until full C5 cache) |
| Full §4 membership predicate | MUST | MUST NOT rely on for integrity |
| SUBSCRIBE capability token (§3.3) | n/a | MUST (admission filter) |
| Forward unverified as valid | MUST NOT | MUST NOT |

---

## 6. Shared quarantine buffer

One bounded, application-visible quarantine is shared by:

- `AUTHOR_UNRESOLVED` (this document),
- `EQUIVOCATION` device quarantine signal (KERNEL §4.5; exclusion is separate and permanent for the op set),
- restore/clone divergence (DQ-7),
- future-clock H1 (M3b).

**v0.1 model parameters** (registry may pin later): max entries = 1024 ops or 1 MiB, whichever first; overflow drops oldest unresolved with `AUTHOR_UNKNOWN` (or the package-specific overflow tag). Exact bounds are not byte-frozen until composite M0; vectors name the bound they assume.

---

## 7. Conformance vectors

| `type` | Checks | Invariants |
|--------|--------|------------|
| `device-cert` | cert-preimage, root sig verify/fail, PrincipalId/PeerId derivation, revoke | I-8 |
| `genesis-id` | genesis body → `DatastoreId`; zero-`ds` rule; salt sensitivity | I-9 |
| `authz-predicate` | op sets + grants/revokes → accept / named reject; concurrent revoke; founder synthetic grant | I-8, I-9 |
| `admission-token` | token preimage + relay-side checks (positive + forged/wrong-ds/revoked) | C4 admission |

Lifecycle: red in `xfail/`, promote on both-runners-green. All four families promoted; M0d exit closed 2026-07-18 (Decision Log).

---

## 8. Out of scope (deliberate)

- Entity-level ACLs (C6) — post-v0.1.
- Group encryption key distribution behavior beyond `KeyRecord` slot (`kr = 2`) — M3b.
- Multi-owner / k-of-n root — post-v0.1.
- Social recovery of lost principal roots — product/docs only in v0.1.

---

*Draft change policy: until composite M0, edits require ordinary review; after freeze, byte-affecting changes re-run the resolution checklist.*
