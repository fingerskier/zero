# zerodb-relay

Experimental **L2** relay process (RELAY-SPEC **0.2.2-draft**). Not a format freeze.

```
zerodb-relay --path ./relay.sqlite --bind 127.0.0.1:7700
```

WebSocket, binary frames, one CBOR envelope per message. Handshake, durable validated oplog, dual-root SYNC (relay publishes `validated_root` only), frozen-snapshot `merkle-walk-v1` subtree/leaf traversal, OpId delta batches, cursor compatibility, and per-op `OP_ACK`.

A LocalStore / NAPI client speaks the same envelopes (`zerodb_storage::relay_client`, `Database.connectRelay`).

**Not** in this slice: AUTH membership/signature admission and encrypted payload enforcement (M3b). Full 1,000-write E3 is exercised in `zerodb-storage/tests/relay_client.rs`.

Tests: `cargo test -p zerodb-relay`; `cargo test -p zerodb-storage --test relay_client`; NAPI `test/m3a-relay.test.mjs`.
