# ZeroDB Schema Specification — IR, Epochs, Migrations & Query Subset (M0b)

**Version:** 0.1.0-draft
**Status:** normative **draft** — the M0b contract under construction (ISSUES C2; decisions O2/O3 recorded 2026-07-16). Package exit requires every rule here backed by vectors green in both conformance runners; nothing freezes before composite M0.
**Authority:** this document owns the schema IR, schema epochs, the migration DSL, and the v0.1 query subset. Encoding primitives and the operation envelope come from [KERNEL.md](KERNEL.md) (the `ep` field and the `SchemaEpoch` variant body defined here fill KERNEL's reserved slots). [SPEC §3](SPEC.md) remains the informative overview; on conflict this document wins for schema semantics.

---

## 1. Two canonical layers (O2)

- **Authoring canonical:** TypeScript SDK definitions (SPEC §3.2 style). Developers never write IR by hand in the normal path. TS is code and is **never** evaluated by the core, replicated, or hashed.
- **Identity canonical:** the **schema IR** below. The deterministic TS→IR compiler (standalone npm tool, ships ≤ M1) emits it; the M1 CLI consumes IR files directly. Same logical schema ⇒ same IR bytes, independent of TS formatting.
- The `.zerodb` DSL is **dropped** as an input format.

## 2. Schema IR

The IR is one canonical-CBOR map (KERNEL §3 profile):

```
{
  "v":     schema_ir_format_version (uint, = 1)
  "name":  text (informational; NOT part of identity semantics beyond bytes)
  "nodes": { label → EntityDef }        // text keys, canonical order
  "edges": { label → EdgeDef }
}
EntityDef = { "props": { path → PropDef } }
EdgeDef   = { "props": { path → PropDef }, "src": [label…] | null, "dst": [label…] | null }
PropDef   = {
  "crdt":      uint      // registry crdt_tags: 0 lww, 1 gcounter, 2 pncounter,
                         //   3 orset, 4 flag; 5 mvregister, 6 lwwmap, 7 rga (reserved, M2)
  "type":      uint      // value-type tag: 0 any-scalar, 1 bool, 2 int, 3 float64,
                         //   4 text, 5 bytes
  "nullable":  bool
  "encrypted": bool      // DQ-5: legal only with crdt ∈ {lww, mvregister}
}
```

- **`SchemaId = BLAKE3(domain("schema_ir") ‖ canonical IR bytes)`** (32 B). An IR is immutable; any change is a new `SchemaId` introduced by a new epoch.
- **`unique` is not representable** (DQ-10): the TS compiler MUST reject `unique:` annotations for v0.1; there is no IR field to smuggle it through.
- `encrypted: true` on a non-LWW/MVRegister property is a compile **and** decode error (DQ-5 constraint, KERNEL §7).
- Unknown map keys anywhere in the IR: **reject** (same forward-compatibility stance as KERNEL §3; evolution goes through `schema_ir_format_version`). This is also what makes `unique` structurally unsmuggleable.
- Schemaless mode is the absence of an epoch (see §3): every property defaults to `lww / any-scalar / nullable / plaintext`.

**Structural validation** runs on every decoded IR before any use; outcomes are named and conformance-tested:

| Outcome | Raised when |
|---------|-------------|
| `IR_UNKNOWN_KEY` | any map carries a key outside its definition (incl. a smuggled `unique`) |
| `IR_VERSION_UNSUPPORTED` | `v` ≠ 1 |
| `IR_CRDT_UNSUPPORTED` | crdt tag is reserved (5–7, until M2) or unregistered |
| `IR_ENCRYPTED_INVALID` | `encrypted: true` with crdt ∉ {lww} (∪ {mvregister} once M2 activates it) |
| `IR_TYPE_MISMATCH` | counter crdt without `type: int`, or flag crdt without `type: bool` |
| `IR_INVALID` | any other shape violation (missing required key, wrong CBOR kind, non-map top level) |

## 3. Schema epochs (C2)

Epochs make CRDT-type resolution deterministic across replay and migration.

- A datastore's epochs are a **linear, causally ordered sequence** `0, 1, 2, …`. Epoch 0 is "schemaless defaults" and exists implicitly at genesis.
- Epoch `n > 0` is introduced by a **`SchemaEpoch` operation** (KERNEL kind 5). Body (canonical CBOR):

```
{ "epoch": uint (= n), "schema": SchemaId, "ir": bytes (the full IR),
  "prev": OpId of the epoch n−1 operation (or null for n = 1),
  "migration": [ MigrationStep… ] }
```

  The full IR rides in the operation (schemas are small; content-addressing dedupes); `prev` makes the chain explicit so epoch linearity is verifiable without search. Issuing authority is datastore control-plane (DQ-2; enforcement M0d/M3b). Two distinct `SchemaEpoch` ops claiming the same `epoch` with the same `prev` are a **fork**: deterministic resolution = the op lower in the KERNEL §4.5 total order wins; the loser and its causal descendants quarantine (application-visible), pending the M0d authority rules.
- Every data operation carries `ep` (KERNEL §4.1) = the highest epoch in its causal past. **CRDT type resolution for an op uses its own epoch's IR** — never the receiver's current schema.
- **Mixed-version rule:** an op with `ep = n` arriving at a peer that has not applied epoch `n` waits in the bounded causal buffer (its `deps` necessarily include the epoch chain); buffer overflow → reject with `EPOCH_UNKNOWN` (retryable). An op with `ep` *older* than current is applied under its own epoch's semantics and then transformed by the intervening migrations (§4) — late ops from long-offline peers remain meaningful.

## 4. Migration DSL (C2)

Migrations are **data, not code** — a list of steps from a closed, versioned registry; every step is total and deterministic.

| Step | Fields | Semantics |
|------|--------|-----------|
| `add_prop` | entity, path, PropDef, `default` (typed scalar or null) | New property; existing entities materialize `default`. |
| `remove_prop` | entity, path | Visibility removal only — operations and history are untouched (no data destruction; GC rules are C7's). Late ops for a removed property apply to shadow state (invisible, convergent). |
| `change_crdt` | entity, path, from-PropDef, to-PropDef, `transform` (registry tag) | Rebinds the property; `transform` maps old materialized state to the new type. |
| `add_entity` / `remove_entity` | label (+EntityDef) | Introduce / hide an entity type (visibility semantics as `remove_prop`). |

**Transform registry v1.** Each transform is **total** (defined for every input, including degenerate ones) and **deterministic**; each reads the source property's materialized scalar and produces the new lww value. `null` is a legal input and output.

| Transform | Source → target | Rule | Degenerate input |
|-----------|-----------------|------|------------------|
| `keep_text` | lww → lww | value unchanged | `null` → `null` |
| `parse_int_or(default)` | lww(text) → lww(int) | if the text matches `^[+-]?\d+$` and fits `i64`, that integer; else `default` | non-text / non-numeric / overflow / `null` → `default` |
| `int_to_text` | lww(int) → lww(text) | base-10 decimal string of the integer | `null` → `null` |
| `counter_value_to_lww_int` | gcounter\|pncounter → lww(int) | the counter's materialized total as an `int` | (counters are always defined; pncounter may be negative) |
| `reset_to(default)` | any → lww | `default`, ignoring prior state | any input → `default` |
| `lww_to_mvregister` | lww → mvregister | singleton — **M2-reserved**, not in the v0.1 profile | — |

Anything not expressible ⇒ the schema author picks `reset_to` — silent partial transforms do not exist. The seed produced by every non-mvregister transform is placed at the SchemaEpoch order key per the *Seed order key* rule above.

*Seed order key.* Transforms that materialize prior state into an order-keyed register (`counter_value_to_lww_int`, `lww_to_mvregister`, `reset_to`) place the seeded value at a definite point in the §4.5 total order: **the `SchemaEpoch` operation's own order key** `(ts.physical_ms, ts.logical, author, OpId)`. A data op authored under the new epoch outranks the seed iff its order key is greater — so a concurrent pre-migration write cannot silently beat a post-migration one, and the seed position is identical on every peer.

**Replay determinism rule (I-17):** materialization is a pure function of (operation set, epoch chain). Ops apply under their own epoch's types; at each epoch boundary the migration steps transform materialized state exactly once, in list order. Fresh full replay ≡ any incremental application order (the vector suite's falsification target).

### 4.1 Executable materialization model (segmented replay)

The reference model materializes one property from its operation set and the canonical epoch chain, reusing the KERNEL §6 CRDT apply rules unchanged:

1. **Canonical chain.** Resolve the `SchemaEpoch` records into one linear chain. Level 1's winner is the record with the **lowest §4.5 order key** among `prev = null` records; level `n`'s winner is the lowest-order-key record whose `prev` is level `n−1`'s winner. Every other record — a fork loser, or any record whose `prev` chain does not lead to a winner — is **quarantined** (application-visible), and so is any data op bound to a quarantined record (KERNEL §4.5 quarantine, not exclusion-from-set: convergent and reversible if the authority later ratifies it).
2. **Pipeline steps 4–5.** Dedup by `OpId`; exclude equivocation groups (§4.5). An op whose resolved epoch has no winner in the known chain yields `EPOCH_UNKNOWN` (the bounded-buffer/late-op outcome of §3, retryable) — the model raises it rather than buffering.
3. **Segments.** Split the chain into maximal runs of epochs that resolve the property to **one CRDT type** (a `change_crdt` starts a new segment; `add_prop`/`remove_prop`/entity steps do not). Within a segment all surviving ops — regardless of which epoch in the segment authored them — apply to **one** replica of that type, so their §4.5 total order is pooled (a late op in an earlier epoch still competes with a later epoch's op; per-epoch partitioning would wrongly reorder them).
4. **Boundaries.** At each `change_crdt`, read the completed segment's materialized value and emit one **synthetic seed op** of the next type carrying that value at the seed order key above; it is ingested into the next segment's replica alongside that segment's ops. The transform is applied exactly once, in list order.
5. **Read** the final segment's replica.

Because every segment is a set-based CRDT materialization and the chain/segment structure is a pure function of the record set, the result is independent of arrival order (I-1) and fresh replay equals incremental application (I-17).

*Named outcomes.* `EPOCH_UNKNOWN` — a surviving op resolves to an epoch with no winner in the known chain, whether past the chain's end or stranded behind a gap/broken link. `SCHEMA_TYPE_CHANGE_WITHOUT_MIGRATION` — two adjacent epochs in one segment (no `change_crdt` between them) disagree on the property's CRDT type; the schema is malformed, because a type change must carry a migration. These are model-visible, not silent.

## 5. Query subset (O3)

v0.1 grammar (case-insensitive keywords; parameterization via `$name` placeholders only — no string splicing):

```
query    := MATCH pattern (WHERE expr)? RETURN items (ORDER BY sort (ASC|DESC)?)? (LIMIT uint)?
pattern  := entity | entity edge entity                    // one hop max in v0.1
entity   := "(" var (":" label)? ")"
edge     := "-[" var? (":" label)? "]->" | "<-[" var? (":" label)? "]-"
expr     := expr AND expr | expr OR expr | NOT expr | "(" expr ")"
          | operand cmp operand | operand IS NULL | operand IS NOT NULL
cmp      := "=" | "<>" | "<" | "<=" | ">" | ">="
operand  := var "." path | literal | "$" name
items    := (var | var "." path) ("," …)*
sort     := var "." path
```

Deterministic semantics:

- **Null:** any `cmp` with `null` evaluates **false** (not unknown — two-valued logic); `IS NULL / IS NOT NULL` are the only null tests. Missing property ≡ null.
- **Cross-type comparison** is false except int/float64 numeric comparison (exact where representable). `ORDER BY` uses one total order over values: `null < bool < int/float (numeric) < text (bytewise UTF-8) < bytes (bytewise)`, with the owning entity id as final tiebreaker — result order is identical on every peer (I-1 extended to reads). **`ORDER BY` is ascending when neither `ASC` nor `DESC` is given.** With no `ORDER BY` clause, rows are ordered by the bound entity ids in pattern order — never left arrival-dependent.
- **Conflict surfacing:** LWW/counters/sets/flags read their materialized value (`orset` → sorted array). `MVRegister` (M2) reads as an array of concurrent values; **a singleton MVRegister compares as its one element, while an MVRegister with zero or more than one value makes the comparison false.** Selection always surfaces the array — conflicts are visible, never silently collapsed.
- Aggregation, multi-hop paths, and mutation-in-query are **post-v0.1**; the parser MUST reject them, not partially execute.

## 6. Conformance vectors

| `type` | Checks | Invariants |
|--------|--------|------------|
| `schema-ir` | canonical IR bytes ↔ tagged description, round trip, `SchemaId`; negatives (unknown key, `unique` smuggling, `encrypted` on a counter) | I-6, I-17 |
| `epoch-replay` | op sets spanning a `SchemaEpoch` (incl. a `change_crdt` type change) → identical state under permutations and fresh replay (§4.1 model); own-epoch type resolution, same-type cross-epoch pooling, fork lowest-order-key resolution + descendant quarantine, `EPOCH_UNKNOWN` for an op past the known chain | I-1, I-17 |
| `migration-transform` | each registry transform: input state → output state, incl. degenerate inputs | I-17 |
| `query-eval` | grammar accept/reject cases; evaluation over a fixture graph incl. null, cross-type, MVRegister, ORDER BY determinism | — (read-side determinism) |

Same lifecycle as every family: red in `xfail/`, promoted on both-runners-green; M0b's exit requires the full suite promoted plus the C2 checklist.
