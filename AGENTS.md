# Working in this repo

Zero (ZeroDB) is a decentralized, offline-first, CRDT-powered property graph database that syncs peer-to-peer and through relays. It is a pre-alpha product: formats are draft-1 / unfrozen and nothing is published to crates.io or npm.

## Where things are decided

Authority order on conflict: **[doc/SPEC.md](doc/SPEC.md) §10** (normative roadmap and milestone gates) → package contracts (**KERNEL, AUTH, SCHEMA, MERKLE, WAL, DELIVERY, FRONTIER, VERSIONS, RELAY-SPEC** in `doc/`) → **[doc/ISSUES.md](doc/ISSUES.md)** (open C/H/O items and the Decision Log) → **[plan/LEDGER.md](plan/LEDGER.md)** (live work rows and closed index) → **[plan/PLAN.md](plan/PLAN.md)** (the one ordered action list). `conformance/registry.json` holds machine-readable constants; `conformance/schemas/` is generated from it (`node conformance/generate-protocol.mjs --check`).

## Conventions that every change must respect

- **Draft-1 / unfrozen.** No wire, bundle, SQLite, wrap-body, or RELAY shape is frozen until an explicit Decision Log act names a versioned profile. Do not write "frozen".
- **Gates close only with evidence.** A milestone or C/H issue closes through the SPEC §10 approved-resolution checklist and a dated ISSUES Decision Log row citing commits, fixtures, or CI runs. "Direction decided" is not a close.
- **Say what is not claimed.** Every CHANGELOG entry, Decision Log row, and PR description ends with a **Not claimed** list (e.g. not M3b exit, not M4a complete, not format freeze, not H5 closed, crates/npm unpublished). Never let a doc imply more than the evidence.
- **Not-yet-merged text uses landed refs after merge.** Replace "(this PR)" / "this act" with `#NN @ sha` when the PR lands; the docs sweep of 2026-09-12 is the reference for the format.
- **Signed wire is the source of truth.** Indexes, derived columns, snapshots, and projections are acceleration only; `replay_all` is the oracle.
- **One AUTH preimage.** `zerodb-relay-auth-v2` over HELLO + nonce + intended WELCOME on both the relay WebSocket and the DataChannel; optional hello fields are omitted when absent so existing goldens stay byte-identical.
- **Conformance is two-language.** Golden and negative vectors must be green in both the Rust runner and the independent TS runner (`conformance/ts/runner.mjs --lane required`) before promotion from `xfail/`.

## How work flows

- Branch off `main`, push, open a PR with a test plan; CI runs the Rust workspace (tests, fmt, clippy `-D warnings`), NAPI, conformance lanes, TS peer, WebRTC, WASM size, and React hooks jobs.
- **Codex reviews every PR within minutes** and has been substantive; read and answer its threads before merging. Squash-merge.
- Local toolchain note: `cargo` may need `MISE_RUST_VERSION=1.98.1` (or `mise use -g rust@…`) in fresh shells; the TS suites that spawn `zerodb-relay` run `cargo build` themselves.
- Docs that must move together: CHANGELOG "Unreleased", ISSUES Decision Log, LEDGER rows, PLAN §5, SUPPORT "not supported" list. Concurrent PRs will conflict on those adjacent lines; keep both entries.

## Current not-claimed baseline (2026-09-12)

Not M4a complete (real-browser DataChannel path, direct/relay parity, browser restart/offline tests open). Not M3b exit (C5/PKI, H10 close, H8 direction). H5 open (handshake-server identity). Not a format freeze. `v0.1.0` is tagged as an experimental product slice, not a frozen format.
