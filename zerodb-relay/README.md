# zerodb-relay

Experimental **L2** relay process (RELAY-SPEC **0.2.2-draft**). Not a format freeze.

```
zerodb-relay --path ./relay.sqlite --bind 127.0.0.1:7700
zerodb-relay --path ./relay.sqlite --bind 0.0.0.0:7700 --tls-cert cert.pem --tls-key key.pem   # wss://
zerodb-relay --path ./relay.sqlite --bind 0.0.0.0:7700 --allow-insecure                       # LAN tests only
```

Hardening flags: `--max-connections` (1024), `--handshake-timeout-secs` (10), `--idle-timeout-secs` (300; `0` disables). `--stats-interval-secs N` prints one `zerodb-relay stats {json}` line to stderr every N seconds (process-lifetime counters: sessions, ops accepted/duplicate/rejected, `sync_requests`, `merkle_builds`, node/leaf/delta requests; `0` = off) — benchmark/ops evidence, never on the wire (`Relay::stats()`). Used by `bench/relay-chat/`. Oversized WebSocket messages are refused before decode; RELAY §4.6 `PING`/`PONG` and `GOODBYE` are handled. Evidence: `zerodb-relay/tests/hardening.rs`.

WebSocket, binary frames, one CBOR envelope per message. Handshake AUTH is a draft transcript (`zerodb-relay-auth-v2`). Durable validated oplog, dual-root SYNC (relay publishes `validated_root` only), frozen-snapshot `merkle-walk-v1` subtree/leaf traversal, OpId delta batches, cursor compatibility, and per-op `OP_ACK`. Session `max_subscriptions` / rate / 3 connections per PeerId are enforced. Authenticated `SIGNAL` (0x42) is forwarded onto the target live socket (`0x307` if missing). Non-loopback plaintext listen requires `--allow-insecure`; with `--tls-cert`/`--tls-key` the binary terminates TLS in-process (rustls) and serves `wss://` on any bind (it does not mint certificates).

A LocalStore / NAPI client speaks the same envelopes (`zerodb_storage::relay_client`, `Database.connectRelay`).

Signature / OpId / datastore admission is on (`m3b_admission`). AUTH membership + E5, E7 forged/replay, E8 clock quarantine, and E6 ciphertext persist are on. Full 1,000-write E3 is exercised in `zerodb-storage/tests/relay_client.rs`. **Not claimed:** M3b exit, H10 closed, format freeze.

Tests: `cargo test -p zerodb-relay`; `cargo test -p zerodb-relay --test signal --test signal_ws`; `cargo test -p zerodb-storage --test relay_client`; NAPI `test/m3a-relay.test.mjs`.

Support profile (platforms, TLS-not-in-process, unpublished crates): [SUPPORT.md](../doc/SUPPORT.md). Version window: [UPGRADE.md](../doc/UPGRADE.md).
