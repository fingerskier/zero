# Changelog

All notable tree changes intended for a future product label. This file does **not** create git tag `v0.1.0` and does **not** claim M3c complete.

Version story:

| Label | Meaning | Status |
|-------|---------|--------|
| Workspace / npm `0.1.0-alpha` | Crate and package semver (`Cargo.toml` `[workspace.package]`, `@zerodb/node`, `@zerodb/ts-to-ir`) | current; `publish = false` / `"private": true` |
| Git `v0.1.0-local` | M1 experimental exit | tagged |
| Git `v0.1.0-sdk` | M2 experimental exit (not SPEC-complete M2) | tagged |
| Git `v0.1.0` | SPEC M3c exit + Decision Log act at tag time | **not tagged** |

Do not bump workspace semver to `0.1.0` and do not `cargo publish` / `npm publish` until that Decision Log act. Formats stay draft-1 / unfrozen unless that act (or a separate freeze act) says otherwise.

---

## Unreleased

Packaging and M3c-a..c on `main`. Preparable for a later `v0.1.0` tag; not that tag.

### M3c-d — packaging / support profile (this PR)

- [SUPPORT.md](doc/SUPPORT.md) — draft-1 support profile (platforms, crates, binaries, TS peer/runner, L2 relay, limits, explicit non-support).
- [UPGRADE.md](doc/UPGRADE.md) — v0.1 window-size-1 matrix from `conformance/registry.json` + [VERSIONS.md](doc/VERSIONS.md); `FORMAT_UNSUPPORTED` / `MERKLE_VERSION_MISMATCH`; M4 adjacent-version matrix is a forward pointer only.
- This changelog + workspace comment: crate `0.1.0-alpha` stays unpublished until a tag act.

### M3c-c — two-language harness (H9) — landed main #19 (`bd752fa`)

Registry is the RELAY 0.2.2-draft protocol definition; `conformance/schemas/` generated from it. Required-lane relay+peer vectors green in `conformance/ts/runner.mjs` and `zerodb-core` `conformance_relay` / `conformance_peer`. H9 not removed. Not a format freeze.

### M3c-b — independent TypeScript wire peer — landed main #18

`conformance/ts/peer/` evolved from the runner, **not** NAPI-backed. Live RELAY 0.2 HELLO / `zerodb-relay-auth-v2` / WELCOME; signed KERNEL ops including SchemaEpoch n=1; merkle-walk catch-up; `EPOCH_UNKNOWN` fail-closed.

### M3c-a — signed `SchemaEpoch` — landed main #17

KERNEL kind 5 persist/ingest/import (n=1, empty migration). Unknown `ep` is `EPOCH_UNKNOWN`. Wrap-body unfrozen.

### Earlier (already on main)

M0 contracts, M1 `v0.1.0-local`, M2 `v0.1.0-sdk`, M3a L2 relay + E3, M3b remainder pinned (E5–E8 live; not M3b exit). See [LEDGER.md](plan/LEDGER.md) and the [ISSUES Decision Log](doc/ISSUES.md).
