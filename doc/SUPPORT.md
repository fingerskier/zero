# ZeroDB support profile (draft-1 / unfrozen)

**Status:** packaging notes for the M3c product slice (`v0.1.0` Decision Log act, tagged @ `177e247`). Formats remain **draft-1 / unfrozen**. This is **not** a format freeze, **not** M3b exit, and **not** a crates.io/npm publish.

**Authority:** descriptive of what the tree actually builds and CI actually runs. Normative version policy is [VERSIONS.md](VERSIONS.md); current constants live in [`conformance/registry.json`](../conformance/registry.json). Upgrade/reject names: [UPGRADE.md](UPGRADE.md). Product tags vs crate/npm versions: [CHANGELOG.md](../CHANGELOG.md).

---

## 1. What this slice is

The first multi-peer secure product slice **with offline catch-up** (SPEC §10 M3c) is named by the `v0.1.0` Decision Log act (#22 @ `177e247`, tagged):

- Git tags: `v0.1.0-local` (M1 experimental), `v0.1.0-sdk` (M2 experimental), `v0.1.0` (M3c; on `177e247`).
- Workspace crates and npm packages stay `0.1.0-alpha` with `publish = false` / `"private": true`.
- All wire, bundle, SQLite, wrap-body, and RELAY shapes stay draft-1 / unfrozen.

## 2. Platforms (what CI covers)

| Surface | CI | Notes |
|---------|----|--------|
| Rust workspace (`cargo test --workspace --locked`, fmt, clippy `-D warnings`) | `ubuntu-latest`, `dtolnay/rust-toolchain@stable` | Workspace `edition = 2024`. No `rust-version` pin. |
| `@zerodb/node` NAPI addon | `ubuntu-latest` + `windows-latest`, Node 22 | Built from source for the host target. `package.json` `napi.targets` lists Windows only — that is **not** a published multi-platform npm matrix. |
| Conformance required lane + `generate-protocol.mjs --check` | `ubuntu-latest`, Node 20 | Independent TS runner; never NAPI. |
| TS wire peer smoke (`conformance/ts/peer/*.test.mjs`) | `ubuntu-latest`, Node 22 + built `zerodb-relay` | NAPI-free. |
| WebRTC / H6 (`conformance/ts/webrtc/*.test.mjs` + `zerodb-relay` `signal` / `signal_ws` + `conformance_h6`) | `ubuntu-latest`, Node 22 + built `zerodb-relay` | Live WS SIGNAL fanout + PeerId roles + fake ordered DataChannel + named `h6-profile` fixtures (incl. HELLO.datastore AUTH bind); not wrtc; no TURN. H6 closed (protocol); not M4a complete. |
| `tools/ts-to-ir` | `ubuntu-latest`, Node 20 | Authoring JSON → IR helper. |
| Browser / WASM (`zerodb-wasm`, Pages) | `wasm` job + `pages` workflow | M4a-a IDB/OPFS adapters + persist/reopen; **not** M4a complete; **not** this support profile's product platforms. |
| `@zerodb/react` optional hooks | `react-hooks` job | M4a slice over wasm + `openDurable`; **not** M4a complete; **not** a product platform. |

**Not in CI / not supported as product platforms:** macOS, iOS/Android, musl-only hosts, a hosted public relay. In-process TLS (`--tls-cert`/`--tls-key`, rustls/ring) is exercised in CI with a self-signed test certificate; certificate provisioning, rotation, and a public CA story are the operator's.

Node engines stated by `@zerodb/node`: `>=18`. Conformance and the TS peer are exercised at Node 20/22.

## 3. Crates, binaries, JS packages

| Artifact | Role | Publish |
|----------|------|---------|
| `zerodb-core` | KERNEL / AUTH / Merkle / relay codecs | `publish = false` |
| `zerodb-storage` | SQLite `LocalStore` (default); `MemoryBackend` when `sqlite` is off | `publish = false` |
| `zerodb-cli` (`zerodb`) | M1 local CLI | `publish = false` |
| `zerodb-relay` (`zerodb-relay`) | Experimental **L2** RELAY 0.2.2-draft process | `publish = false` |
| `zerodb-napi` / `@zerodb/node` | Experimental M2 Node binding | crate unpublished; npm `"private": true` |
| `zerodb-wasm` | Browser wasm peer; durable IDB/OPFS adapters (JS) | `publish = false` |
| `@zerodb/ts-to-ir` | Minimal authoring → IR JSON | `"private": true` |
| `@zerodb/react` | Optional React hooks over wasm + IDB/OPFS | `"private": true` |
| `conformance/ts/runner.mjs` | Independent two-language harness (H9) | not a package |
| `conformance/ts/peer/` | Independent RELAY 0.2 wire peer (M3c-b) | not a package; **not** the SDK |

Workspace version is `0.1.0-alpha` in the root `Cargo.toml`. Do not bump it to `0.1.0` and do not `cargo publish` / `npm publish` until a later Decision Log act says so. Git tag `v0.1.0` follows this Decision Log act; it does not publish crates.

## 4. Relay level

`zerodb-relay` is an experimental **Level 2** durable reference relay (registry `relay_wire.relay_level = 2`, RELAY-SPEC 0.2.2-draft):

- Binary WebSocket, one CBOR envelope per frame.
- HELLO / `zerodb-relay-auth-v2` transcript AUTH / WELCOME.
- Durable SQLite validated oplog; publishes `validated_root` only (not peer `accepted_root` equality).
- Frozen-snapshot `merkle-walk-v1` catch-up; cursor compatibility; per-op `OP_ACK`.
- Advertised WELCOME limits (registry `relay_wire.welcome_limits`) plus 3 connections per PeerId.
- Loopback plaintext is the default bind (`127.0.0.1:7700`). Non-loopback plaintext requires `--allow-insecure`. `--tls-cert <pem> --tls-key <pem>` terminates TLS in-process (`wss://`, rustls) and may bind anywhere. **This binary does not mint certificates.**
- Listener hardening: global `--max-connections` (1024), `--handshake-timeout-secs` (10), `--idle-timeout-secs` (300; `0` disables; PING/PONG and WebSocket pings keep a session alive), pre-decode WebSocket message ceiling, GOODBYE handled. Per-PeerId cap 3, subscription cap, and ops/bytes rate windows as before.

LocalStore / NAPI `connectRelay` and the TS peer speak the same envelopes.

## 5. How to build and run

Relay + independent TS peer (loopback):

```bash
cargo build -p zerodb-relay --locked
./target/debug/zerodb-relay --path ./relay.sqlite --bind 127.0.0.1:7700

node conformance/ts/peer/cli.mjs --url ws://127.0.0.1:7700 --schema --create Todo --set title=milk
node conformance/ts/peer/cli.mjs --url ws://127.0.0.1:7700 --join <datastore-hex>
```

Non-loopback plaintext (disposable LAN only):

```bash
./target/debug/zerodb-relay --path ./relay.sqlite --bind 0.0.0.0:7700 --allow-insecure
```

Rust LocalStore client: `zerodb_storage::relay_client` / NAPI `Database.connectRelay`. CLI M1 path (`zerodb serve` / `pull`) is the experimental plaintext TCP/WS v2 LAN path — see [M1-LOCAL.md](M1-LOCAL.md) — not the RELAY 0.2 product relay.

NAPI SDK (source build, not a registry install):

```bash
cd zerodb-napi
npm ci
npx napi build --platform --release
npm test
```

## 6. Known limits (draft-1)

Two tables. Do not treat the first as a runtime resource bound of this tree.

**Policy** (registry `limits`, VERSIONS §3, KERNEL/O6; provisional, ratified for draft-1). VERSIONS calls these pre-auth decode errors. This slice does **not** enforce `max_operation_bytes` / format `max_batch_*` on store `validate_wire_for_ds` or relay `on_ops`. Relay OPS uses the WELCOME table below (1 MiB / 64 ops / 16 MiB). What *is* enforced from this set: CBOR decode depth 16 (`zerodb-core`); `deps` ≤ 64 on store ingress.

| Cap | Value |
|-----|-------|
| `max_operation_bytes` | 65536 |
| `max_batch_bytes` | 262144 |
| `max_batch_ops` | 512 |
| `max_cbor_depth` | 16 |
| `max_deps_per_op` | 64 |

**Enforced on the RELAY 0.2 session** (registry `relay_wire.welcome_limits`; advertised experimental defaults; distinct from format `limits`):

| Cap | Value |
|-----|-------|
| `max_payload_bytes` | 1048576 |
| `max_batch_ops` | 64 |
| `max_batch_bytes` | 16777216 |
| `max_subscriptions` | 64 |
| `ops_per_second` | 100 |
| `bytes_per_second` | 10485760 |
| `max_connections_per_peer` | 3 |

HLC / peer ingest: `max_drift_ms` = 60000 (`CLOCK_DRIFT`). SchemaEpoch in this slice: **n=1 / empty migration**. Wrap-body remains unfrozen. GC is off until C7/M5b.

## 7. Not supported (do not claim)

- **M4a-a WASM size (O4 pinned 2026-09-12).** Size-oriented `scripts/build.sh` artifact `zerodb_wasm_bg.wasm`: **738317 bytes raw (721.0 KiB), 268908 bytes gzip -9 (262.6 KiB)**. ISSUES O4 target vs Automerge ~250 KB gz is **not** met; O4 is pinned (out of the M4a gate), **not** closed. CI records size and fails only if gzip exceeds 300 KiB. Not a format freeze.
- **M4a complete.** H6 protocol is **closed** (Decision Log 2026-09-12; [RELAY-SPEC](RELAY-SPEC.md) §14.2; #25–#28). Fake ordered DataChannel; no hosted TURN / public STUN / `wrtc` (TURN is infra). Signaling identity is relay-asserted until DC AUTH; DC AUTH is DTLS channel-bound (`HELLO.channel_binding`, H5 slice) but the handshake server is still unauthenticated (H5 open). Do not claim M4a complete (O4 pinned; M4a stays open on its own platform criteria (real-browser WebRTC DataChannel path — tests use a fake channel; direct/relay parity; browser restart/offline tests); E10 is M4b).
- **Full TLS production story** — in-process TLS exists (`--tls-cert`/`--tls-key`), but there is no CA, no minted or rotated certs, no OCSP/ALPN story, and no hosted relay; `--allow-insecure` is a LAN escape hatch only.
- **Format freeze** — no versioned frozen profile; wrap-body unfrozen.
- **C5 on-wire complete** — AUTH contract exists; do not claim C5 closed as a product/PKI story.
- **H9 closed** — two-language harness landed (PR #19); issue stays open until an approved-resolution removal.
- **H10 closed** — leftovers implemented; envelope/key lifecycle not closed.
- **M3b exit** — remainder pinned; E5–E8 live is the security bar carried into M3c, not a gate close.
- **Live Rust↔TS partition/rejoin** — follow-on, not this tag, not format freeze. Evidence for this act is H9 two-language fixtures (#19), existing Rust E3, and TS smoke.
- **M4 rolling-upgrade / adjacent-version rollback matrix** — [UPGRADE.md](UPGRADE.md) points forward; do not treat this profile as E10.
- **Format `limits` as a resource bound** — O6 policy numbers are listed above; they are not the relay/store ingress caps (WELCOME is).
- **crates.io / npm registry publish**, hosted relay, mobile bindings, entity-level ACLs (C6), MVRegister/RGA/LWWMap, production backup/SLO (M5a).

## 8. Publish readiness (no publish)

- Crates: workspace `publish = false`. Never `cargo publish`.
- npm: `@zerodb/node`, `@zerodb/ts-to-ir`, and `@zerodb/react` are `"private": true`. Never `npm publish`. NAPI consumers build the addon from this repo.
- Git tag `v0.1.0` follows this Decision Log act; it does not publish registries or freeze formats.

---

*Draft-1 / unfrozen. Git tag `v0.1.0` follows this Decision Log act. A format freeze is a later, separate act.*
