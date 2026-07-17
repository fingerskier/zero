# conformance/

Cross-implementation conformance harness (PLAN P0-5). Golden vectors are the normative artifacts that M0 package exits require (SPEC §10 approved-resolution checklist); both runners must agree on every vector.

## Layout

```
conformance/
├── vectors/
│   ├── required/    # promoted vectors — CI-blocking, must be green in BOTH runners
│   └── xfail/       # newly activated contract fixtures — expected-failure lane,
│                    #   non-blocking until promoted at their package gate
└── ts/
    └── runner.mjs   # independent TypeScript/JS model runner (pure encoder/decoder +
                     #   semantic models; NOT the SDK, never NAPI-backed)
```

The Rust side runs the same vectors via `cargo test` harnesses in the workspace crates (added per package as contracts land).

## Vector format

One JSON file per vector: `{ "id": "...", "type": "<contract>", "invariants": ["I-6"], ... }`. Vector `type`s are registered in the runner dispatch tables as each M0 package lands; an unregistered type **fails** the required lane by design — a vector without a runner is not evidence.

## Promotion policy

1. New contract fixtures land in `vectors/xfail/` and must be **demonstrated failing** there (red) when activated.
2. A vector moves to `vectors/required/` as soon as it is green in **both** runners — CI then blocks on it, locking the progress in. The owning package's exit gate requires the package's **entire** suite promoted.
3. Moving a vector back out of `required/` requires a Decision Log entry.
