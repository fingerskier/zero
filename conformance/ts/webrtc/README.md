# H6 — WebRTC DataChannel + live WS SIGNAL fanout

Two peers negotiate a DataChannel (relay-facilitated `SIGNAL`) and then
run the **existing** RELAY 0.2 peer protocol on that channel: HELLO /
`zerodb-relay-auth-v2` transcript AUTH / WELCOME / OPS so a mutation on
the handshake client appears on the handshake server.

First-cut landed main #25 @ `671adba`. Live WS SIGNAL fanout + PeerId
roles landed main #26 @ `45a88b5`. Close candidate landed main #27 @
`ef2fca9`. This remainder binds optional `HELLO.datastore` into
`AuthTranscript` / `zerodb-relay-auth-v2`.

**This is not H6 closed and not M4a complete.** Steward/Matt confirm
before the issue is removed. O4 (WASM gzip) is untouched — this slice
is JS beside `conformance/ts/peer`, not inside `zerodb-wasm`.

Formats stay draft-1 / unfrozen. No crate or npm publish. SPEC’s
`zerodb-webrtc` crate sketch is not how this repo ships; the TS peer
already owns the wire.

## How to run

```bash
node --test conformance/ts/webrtc/*.test.mjs
node conformance/ts/runner.mjs --lane required
```

Rust in-process SIGNAL + live WS fanout + named H6 fixtures:

```bash
cargo test -p zerodb-relay --test signal --test signal_ws --locked
cargo test -p zerodb-core --test conformance_h6 --locked
```

CI job: `WebRTC first-cut (H6)` (extended, not replaced).

## What is in

- `SIGNAL` (0x42): peer→relay `{target, payload}`; forwarded
  `{sender, payload}`. Relay does not inspect SDP vs ICE. Target not
  connected → `ERROR` `0x307` `TARGET_NOT_CONNECTED`.
- **Live WS fanout:** `serve_connection` drains the per-session mailbox
  onto the target WebSocket. Two authenticated clients against a real
  `zerodb-relay` exchange opaque SIGNAL bytes.
- DataChannel label `zerodb-relay`, ordered, reliable; one protocol
  message per channel message (RELAY-SPEC §14.2).
- Handshake reuses `AuthTranscript` / `authTranscript` /
  `zerodb-relay-auth-v2` — the same intended-WELCOME reconstruction as
  the relay (`handshake.rs` / `conformance/ts/models/relay.mjs`). No
  second AUTH preimage.
- **Roles:** `is_handshake_server` / `isHandshakeServer` — lexicographically
  smaller PeerId issues CHALLENGE/WELCOME; the other sends HELLO/AUTH.
  RTC offerer is not the handshake server. Either peer can serve.
- Client `WELCOME.protocol_version` reject (`0x102`, PR #21) on the
  DataChannel path.
- Signed KERNEL wire remains source of truth. Wrong `wire.ds` is
  `AUTH_WRONG_DATASTORE` (fail closed, same as the WS peer).
- **Admission:** optional `HELLO.datastore` is in the AUTH transcript
  when present (omit when absent). A signaling MITM that swaps the
  offer fails AUTH. A populated (or `expectedDs`-bound) store of A vs
  offered B is one named `AUTH_WRONG_DATASTORE` **before** OPS. An
  empty store may adopt A.
- **Reconnect / resume:** drop the channel, re-SIGNAL, repeat the same
  handshake, then filter `exportOps` with the receiver `resume-cursor`
  / DELIVERY frontier. Pre-drop ops are omitted or `DUPLICATE`, not
  double-applied.
- HELLO `protocol_version` other than `1` is fatal `0x102` before
  CHALLENGE (same as the relay).
- OPS is split with the existing `splitOpsBatches` helper against
  advertised WELCOME limits; the handshake server consumes every batch.
- v1 nonce-only AUTH (`zerodb-relay-auth-v1`) is `AUTH_FAILED`.
- **Named fixtures:** `conformance/registry.json` `h6_profile` +
  `conformance/vectors/required/h6/` (`H6-SIGNAL-*`, `H6-ROLE-001`,
  `H6-AUTH-001/002/003`, `H6-WELCOME-001`, `H6-ADMIT-*`, `H6-RESUME-001`).

## Honest limitations

- **Fake DataChannel / RTC.** Protocol-half tests use an in-process
  ordered reliable channel and opaque fake SDP/ICE. `wrtc` is a CI
  liability (native addon, public STUN). This still exercises SIGNAL +
  the protocol state machine. It is **not** a browser RTCPeerConnection
  or a NAT traversal story.
- **No TURN.** SIGNAL already carries opaque ICE. Hosted TURN / public
  STUN is a deployment choice, not an H6 protocol hole (Decision Log).
  This tree does not add coturn.
- **Signaling identity is relay-asserted** until the signed peer
  handshake (`HELLO`/`AUTH`) on the DataChannel. `SIGNAL.sender` is set
  by the relay.
- **Not stuffed into wasm.** O4 stays open (262.6 KiB gzip vs ~250 KB;
  CI ceiling 400 KiB).

## Layout

| File | Role |
|------|------|
| `channel.mjs` | Fake ordered DataChannel + fake RTCPeerConnection |
| `signal.mjs` | In-process SIGNAL relay (0x307) |
| `negotiate.mjs` | Offer/answer/ICE over SIGNAL (RTC roles ≠ handshake roles) |
| `peer.mjs` | Shared protocol: transcript AUTH + admission + resume + OPS |
| `webrtc.test.mjs` | Fake-DC AUTH/OPS + roles + reconnect + admission |
| `signal-ws.test.mjs` | Live `zerodb-relay` WS fanout |
