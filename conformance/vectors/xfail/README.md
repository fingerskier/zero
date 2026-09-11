# xfail lane

New contract fixtures land here and must be demonstrated red before
promotion (`conformance/README.md`).

M3c-c H9 relay+peer fixtures were demonstrated red 2026-09-11 with
handlers unregistered (`0 passed, 8 failed`), then promoted to
`vectors/required/` when both the independent TS runner and the Rust
`cargo test` harnesses were green. Empty xfail is a valid state.
