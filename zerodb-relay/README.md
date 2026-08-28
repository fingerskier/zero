# zerodb-relay

Experimental **L2** relay process (RELAY-SPEC **0.2.2-draft**). Not a format freeze.

```
zerodb-relay --path ./relay.sqlite --bind 127.0.0.1:7700
zerodb-relay --path ./relay.sqlite --bind 0.0.0.0:7700 --allow-insecure
```

WebSocket, binary frames, one CBOR envelope per message. Handshake AUTH is a draft transcript (`zerodb-relay-auth-v2`). Durable validated oplog, dual-root SYNC (relay publishes `validated_root` only), frozen-snapshot `merkle-walk-v1` subtree/leaf traversal, OpId delta batches, cursor compatibility, and per-op `OP_ACK`. Session `max_subscriptions` / rate / 3 connections per PeerId are enforced. Non-loopback plaintext listen requires `--allow-insecure` (this binary does not terminate TLS and does not mint certificates).

A LocalStore / NAPI client speaks the same envelopes (`zerodb_storage::relay_client`, `Database.connectRelay`).

Signature / OpId / datastore admission is on (`m3b_admission`). AUTH membership + E5, E7 forged/replay, E8 clock quarantine, and E6 ciphertext persist are on. Full 1,000-write E3 is exercised in `zerodb-storage/tests/relay_client.rs`. **Not claimed:** M3b exit, H10 closed, format freeze.

Tests: `cargo test -p zerodb-relay`; `cargo test -p zerodb-storage --test relay_client`; NAPI `test/m3a-relay.test.mjs`.
