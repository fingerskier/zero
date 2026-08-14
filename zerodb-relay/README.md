# zerodb-relay

Experimental **L2** relay process (RELAY-SPEC **0.2.2-draft**). Not a format freeze.

```
zerodb-relay --path ./relay.sqlite --bind 127.0.0.1:7700
```

WebSocket, binary frames, one CBOR envelope per message. Handshake, durable validated oplog, dual-root SYNC (relay publishes `validated_root` only), cursor catch-up, per-op `OP_ACK`.

**Not** in this slice: Merkle walk, AUTH membership (M3b), EXEMPLAR E3 at full 1000-op scale.

Tests: `cargo test -p zerodb-relay`.
