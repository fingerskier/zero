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

**Transform registry v1** (each total — defined for every input): `keep_text`, `parse_int_or(default)`, `int_to_text`, `counter_value_to_lww_int` (counter reads out as an LWW int seeded at the migration epoch), `lww_to_mvregister` (singleton), `reset_to(default)`. Anything not expressible ⇒ the schema author picks `reset_to` — silent partial transforms do not exist.

**Replay determinism rule (I-17):** materialization is a pure function of (operation set, epoch chain). Ops apply under their own epoch's types; at each epoch boundary the migration steps transform materialized state exactly once, in list order. Fresh full replay ≡ any incremental application order (the vector suite's falsification target).

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
- **Cross-type comparison** is false except int/float64 numeric comparison (exact where representable). `ORDER BY` uses one total order over values: `null < bool < int/float (numeric) < text (bytewise UTF-8) < bytes (bytewise)`, with the owning entity id as final tiebreaker — result order is identical on every peer (I-1 extended to reads).
- **Conflict surfacing:** LWW/counters/sets/flags read their materialized value (`orset` → sorted array). `MVRegister` (M2) reads as an array of concurrent values; a comparison against an MVRegister with > 1 value is false and selection surfaces the array — conflicts are visible, never silently collapsed.
- Aggregation, multi-hop paths, and mutation-in-query are **post-v0.1**; the parser MUST reject them, not partially execute.

## 6. Conformance vectors

| `type` | Checks | Invariants |
|--------|--------|------------|
| `schema-ir` | canonical IR bytes ↔ tagged description, round trip, `SchemaId`; negatives (unknown key, `unique` smuggling, `encrypted` on a counter) | I-6, I-17 |
| `epoch-replay` | op sets spanning a `SchemaEpoch` (incl. a `change_crdt` type change) → identical state under permutations and fresh replay | I-1, I-17 |
| `migration-transform` | each registry transform: input state → output state, incl. degenerate inputs | I-17 |
| `query-eval` | grammar accept/reject cases; evaluation over a fixture graph incl. null, cross-type, MVRegister, ORDER BY determinism | — (read-side determinism) |

Same lifecycle as every family: red in `xfail/`, promoted on both-runners-green; M0b's exit requires the full suite promoted plus the C2 checklist.
