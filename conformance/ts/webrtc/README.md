# H6 first-cut — WebRTC DataChannel over the shared peer protocol

Two peers negotiate a DataChannel (relay-facilitated `SIGNAL`) and then
run the **existing** RELAY 0.2 peer protocol on that channel: HELLO /
`zerodb-relay-auth-v2` transcript AUTH / WELCOME / OPS so a mutation on
A appears on B.

**This is not H6 closed and not M4a complete.** Role negotiation,
datastore-admission tokens, reconnect, TURN, and a conformance profile
with approved-resolution evidence remain open. O4 (WASM gzip) is
untouched — this slice is JS beside `conformance/ts/peer`, not inside
`zerodb-wasm`.

Formats stay draft-1 / unfrozen. No crate or npm publish. SPEC’s
`zerodb-webrtc` crate sketch is not how this repo ships; the TS peer
already owns the wire.

## How to run

```bash
node --test conformance/ts/webrtc/*.test.mjs
```

Rust in-process SIGNAL (0x307 + opaque forward):

```bash
cargo test -p zerodb-relay --test signal --locked
```

CI job: `WebRTC first-cut (H6)`.

## What is in

- `SIGNAL` (0x42): peer→relay `{target, payload}`; forwarded
  `{sender, payload}`. Relay does not inspect SDP vs ICE. Target not
  connected → `ERROR` `0x307` `TARGET_NOT_CONNECTED`.
- DataChannel label `zerodb-relay`, ordered, reliable; one protocol
  message per channel message (RELAY-SPEC §14.2).
- Handshake reuses `AuthTranscript` / `authTranscript` /
  `zerodb-relay-auth-v2` — the same intended WELCOME reconstruction as
  the relay (`handshake.rs` / `conformance/ts/models/relay.mjs`). No
  second AUTH preimage.
- Initiator plays the client role; answerer plays CHALLENGE/WELCOME
  (hardcoded for this cut — role negotiation is still open).
- Client `WELCOME.protocol_version` reject (`0x102`, PR #21) on the
  DataChannel path.
- Signed KERNEL wire remains source of truth. Wrong `wire.ds` is
  `AUTH_WRONG_DATASTORE` (fail closed, same as the WS peer). A
  populated answerer binds its existing datastore unless `expectedDs`
  is set; only an empty store infers/adopts the incoming OPS datastore.
- HELLO `protocol_version` other than `1` is fatal `0x102` before
  CHALLENGE (same as the relay).
- OPS is split with the existing `splitOpsBatches` helper against
  advertised WELCOME limits; the answerer consumes every batch.
- v1 nonce-only AUTH (`zerodb-relay-auth-v1`) is `AUTH_FAILED`.

## Honest limitations

- **Fake DataChannel / RTC.** Tests use an in-process ordered reliable
  channel and opaque fake SDP/ICE. `wrtc` is a CI liability (native
  addon, public STUN). This still exercises SIGNAL + the protocol state
  machine. It is **not** a browser RTCPeerConnection or a NAT traversal
  story. No TURN.
- **Signaling identity is relay-asserted** until the signed peer
  handshake (`HELLO`/`AUTH`) on the DataChannel. `SIGNAL.sender` is set
  by the relay.
- **Live `zerodb-relay` WS fanout** is not this slice. The process
  `handle()` implements SIGNAL + a per-session mailbox for in-process
  tests; the blocking WS accept loop does not yet push SIGNAL onto
  another socket. The TS `SignalRelay` double is what the WebRTC tests
  talk to.
- **Reconnect is open.** Drop the channel and you start over.
- **No H6 conformance profile.** No role-negotiation handshake, no
  SUBSCRIBE-token datastore admission on the DC path.
- **Not stuffed into wasm.** O4 stays open (262.6 KiB gzip vs ~250 KB;
  CI ceiling 400 KiB).

## Layout

| File | Role |
|------|------|
| `channel.mjs` | Fake ordered DataChannel + fake RTCPeerConnection |
| `signal.mjs` | In-process SIGNAL relay (0x307) |
| `negotiate.mjs` | Offer/answer/ICE over SIGNAL |
| `peer.mjs` | Shared protocol: transcript AUTH + OPS |
| `webrtc.test.mjs` | CI evidence |
