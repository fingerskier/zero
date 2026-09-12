# Changelog

All notable tree changes for the product label. Git tag `v0.1.0` follows this Decision Log act (steward tags after merge). Workspace crates stay unpublished. Formats stay draft-1 / unfrozen.

Version story:

| Label | Meaning | Status |
|-------|---------|--------|
| Workspace / npm `0.1.0-alpha` | Crate and package semver (`Cargo.toml` `[workspace.package]`, `@zerodb/node`, `@zerodb/ts-to-ir`) | current; `publish = false` / `"private": true` |
| Git `v0.1.0-local` | M1 experimental exit | tagged |
| Git `v0.1.0-sdk` | M2 experimental exit (not SPEC-complete M2) | tagged |
| Git `v0.1.0` | SPEC M3c exit + Decision Log act | tag follows this Decision Log act |

Do not bump workspace semver to `0.1.0` and do not `cargo publish` / `npm publish`. Formats stay draft-1 / unfrozen; freeze is a separate Decision Log act.

---

## Unreleased

### H5 slice — DTLS channel binding on the DataChannel (this PR)

`HELLO.channel_binding` (BLAKE3 `zerodb-dc-binding-v1` over both SHA-256 DTLS certificate fingerprints, order-independent) is in the existing `AuthTranscript` / `zerodb-relay-auth-v2` hello map when present and omitted when absent (relay WebSocket vectors unchanged). The DataChannel handshake server derives the value from its own SDP, requires the HELLO claim to match before CHALLENGE, and binds its own derivation; missing / foreign / malformed → `0x201`. A signaling MITM that terminates DTLS on both legs and forwards AUTH unchanged now fails before OPS (`webrtc.test.mjs` keeps the pre-fix leak as a control case). `conformance/ts/webrtc/binding.mjs` parses real `a=fingerprint` lines; the fake `RTCPeerConnection` emits one per connection. Fixture `H6-AUTH-004` in both runners; registry `domain_separation.dc_channel_binding`; `zerodb-relay` binds a sent `channel_binding` on the WS path too. **H5 not closed** (handshake-server identity, signed CHALLENGE/WELCOME, TLS remain). **Not** M4a complete. Crates/npm stay `0.1.0-alpha` unpublished.

### H6 closed — shared peer protocol over DataChannel (this PR)

H6 (Direct P2P sync has no protocol) is **closed**. Shared peer protocol over an ordered `zerodb-relay` DataChannel: HELLO / `zerodb-relay-auth-v2` / WELCOME / OPS. Evidence on main: #25 @ `671adba`, #26 @ `45a88b5`, #27 @ `ef2fca9`, #28 @ `bfe69b9`. Same `AuthTranscript` helper; no second preimage. TURN/NAT is infra (no hosted TURN / `wrtc`). **Not** M4a complete. O4 still open. No E10. Crates/npm stay `0.1.0-alpha` unpublished.

### M4a-h6-auth-ds — bind HELLO.datastore into AuthTranscript (landed main #28 @ `bfe69b9`)

Optional `HELLO.datastore` is in the existing `AuthTranscript` / `zerodb-relay-auth-v2` hello map when present and omitted when absent (RELAY-HELLO-001 / H6-AUTH-001 stay byte-identical). A signaling MITM that swaps the offered id fails AUTH before OPS. Present-but-invalid claims (non-32-byte / non-hex) fail closed — they are not treated as omitted. Same helper; no v3 domain. Fixture `H6-AUTH-003` plus live webrtc MITM and `joinDs: 'not-a-datastore'` tests. **H6 not closed** on that PR. **Not** M4a complete. O4 stays open. No TURN/NAT. Crates/npm stay `0.1.0-alpha` unpublished.

### M4a-h6-close — H6 protocol close candidate (landed main #27 @ `ef2fca9`)

Reconnect/resume after dropping the ordered DataChannel: re-SIGNAL, repeat HELLO / `zerodb-relay-auth-v2` / WELCOME (same `AuthTranscript`; no second preimage), then `resume-cursor` / DELIVERY frontier so already-acked ops are omitted or `DUPLICATE`. Session datastore admission: populated A vs offered B is `AUTH_WRONG_DATASTORE` before OPS; empty store may adopt. Named `h6-profile` fixtures in the required lane. TURN/NAT is infra — no coturn, public STUN, or `wrtc`. **H6 not closed** (steward confirm). **Not** M4a complete. O4 stays open. Crates/npm stay `0.1.0-alpha` unpublished.

### M4a-h6-fanout — live WS SIGNAL fanout + role negotiation (landed main #26 @ `45a88b5`)

Live `zerodb-relay` WebSocket SIGNAL (0x42) fanout: a forwarded `{sender, payload}` is written to the target socket (mailbox drain in `serve_connection`). Target missing → `0x307`. Handshake roles are PeerId order (`is_handshake_server` / `isHandshakeServer`); AUTH is still `zerodb-relay-auth-v2` (no second preimage). Tests: `zerodb-relay/tests/signal_ws.rs`, `conformance/ts/webrtc/signal-ws.test.mjs`, role cases in `webrtc.test.mjs`. **H6 not closed.** **Not** M4a complete. O4 stays open (262.6 KiB gzip vs ~250 KB). Crates/npm stay `0.1.0-alpha` unpublished.

### M4a-webrtc — H6 first-cut (landed main #25 @ `671adba`)

SIGNAL (0x42) + ordered `zerodb-relay` DataChannel carrying the shared RELAY 0.2 peer protocol (HELLO / `zerodb-relay-auth-v2` / WELCOME / OPS). Reuses `handshake.rs` `AuthTranscript` — no second AUTH preimage. JS lives at `conformance/ts/webrtc/` (not a wasm crate; O4 untouched). Tests: `conformance/ts/webrtc/webrtc.test.mjs` (in-process ordered channel, not `wrtc`) and `zerodb-relay/tests/signal.rs` (`0x307`). **H6 not closed.** **Not** M4a complete. O4 stays open (262.6 KiB gzip vs ~250 KB). Crates/npm stay `0.1.0-alpha` unpublished.

### M4a-hooks — optional `@zerodb/react` (landed main #24 @ `3f81d62`)

`ZeroDbProvider` + `useQuery` / `useNode` / `useMutation` / `useSyncStatus` wrapping live `zerodb-wasm` + `openDurable`. Persist-on-write via `journal.persist`. Tests: `zerodb-react/test/hooks.test.mjs`. `useSyncStatus` is local ready/offline (no WebSocket / WebRTC). **Not** M4a complete. O4 stays open (262.6 KiB gzip vs ~250 KB). Crates/npm stay `0.1.0-alpha` unpublished.

### M4a-a — WASM + IndexedDB/OPFS persist/reopen (landed main #23 @ `56a3bad`)

Durable browser adapters behind `zerodb-wasm` (`js/storage.mjs`): IndexedDB + OPFS journal identity + signed KERNEL ops; `openDurable` restores after reload. Persist/reopen tests in `zerodb-wasm/test/persist-reopen.test.mjs`. WASM size (size-oriented build): 721.0 KiB raw / **262.6 KiB gzip -9**. O4 ~250 KB gz target not met; O4 stays open. **Not** M4a complete (React hooks are the following slice; no WebRTC/H6). Crates stay `0.1.0-alpha` unpublished.

Review fixes (Codex P1/P2 on #23): `auto` keeps an occupied IndexedDB name instead of minting empty OPFS; IDB opens at the existing version (no v2→v1 `VersionError`); persist/replace are serialized per journal; malformed OPFS identity fails closed.

## v0.1.0

Decision Log act for product git tag `v0.1.0` (steward tags after this merge). Crates remain `0.1.0-alpha` unpublished. Evidence is H9 two-language fixtures (#19), existing Rust E3, and TS smoke. Live Rust↔TS partition/rejoin is follow-on, not this tag, not format freeze.

### Client WELCOME `protocol_version` reject — landed main #21 (`ca508d0`)

- Rust `relay_client` and the independent TS peer fail-closed on `WELCOME.protocol_version` other than `1` or missing (`0x102 VERSION_MISMATCH`). They do not proceed to OPS/sync. Not `FORMAT_UNSUPPORTED`. HELLO-side relay reject unchanged.

### M3c-d — packaging / support profile — landed main #20 (`0542490`)

- [SUPPORT.md](doc/SUPPORT.md) — draft-1 support profile (platforms, crates, binaries, TS peer/runner, L2 relay, limits, explicit non-support). Registry format `limits` are O6 **policy**, not store/relay ingress caps (WELCOME is).
- [UPGRADE.md](doc/UPGRADE.md) — v0.1 window-size-1 matrix from `conformance/registry.json` + [VERSIONS.md](doc/VERSIONS.md); `FORMAT_UNSUPPORTED` / `MERKLE_VERSION_MISMATCH`; M4 adjacent-version matrix is a forward pointer only. HELLO `protocol_version != 1` is relay-rejected; clients reject non-v1 / missing `WELCOME.protocol_version` (`0x102`).
- This changelog + workspace comment: crate `0.1.0-alpha` stays unpublished.

### M3c-c — two-language harness (H9) — landed main #19 (`bd752fa`)

Registry is the RELAY 0.2.2-draft protocol definition; `conformance/schemas/` generated from it. Required-lane relay+peer vectors green in `conformance/ts/runner.mjs` and `zerodb-core` `conformance_relay` / `conformance_peer`. H9 not removed. Not a format freeze.

### M3c-b — independent TypeScript wire peer — landed main #18

`conformance/ts/peer/` evolved from the runner, **not** NAPI-backed. Live RELAY 0.2 HELLO / `zerodb-relay-auth-v2` / WELCOME; signed KERNEL ops including SchemaEpoch n=1; merkle-walk catch-up; `EPOCH_UNKNOWN` fail-closed.

### M3c-a — signed `SchemaEpoch` — landed main #17

KERNEL kind 5 persist/ingest/import (n=1, empty migration). Unknown `ep` is `EPOCH_UNKNOWN`. Wrap-body unfrozen.

### Earlier (already on main)

M0 contracts, M1 `v0.1.0-local`, M2 `v0.1.0-sdk`, M3a L2 relay + E3, M3b remainder pinned (E5–E8 live; not M3b exit). See [LEDGER.md](plan/LEDGER.md) and the [ISSUES Decision Log](doc/ISSUES.md).
