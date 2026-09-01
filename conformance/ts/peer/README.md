# Independent TypeScript wire peer (M3c-b)

Pure encoder/decoder + in-memory KERNEL store + RELAY 0.2 WebSocket client.
Reuses `conformance/ts/models/`. **NOT** the SDK. **Never** NAPI-backed — do
not import `zerodb-napi`, do not dlopen the native addon.

Speaks the live `zerodb-relay` wire as currently implemented (draft-1 /
unfrozen): HELLO / `zerodb-relay-auth-v2` transcript AUTH / WELCOME; signed
KERNEL ops (CreateNode, SetProperty LWW, SchemaEpoch kind 5 n=1 / prev=null /
empty migration); merkle-walk catch-up; `EPOCH_UNKNOWN` fail-closed; advertised
WELCOME `max_payload_bytes` / `max_batch_*` honored the same way as
`zerodb_storage::relay_client`.

```bash
cargo build -p zerodb-relay
./target/debug/zerodb-relay --path /tmp/relay.sqlite --bind 127.0.0.1:0

node conformance/ts/peer/cli.mjs --url ws://127.0.0.1:PORT --schema --create Todo --set title=milk
node conformance/ts/peer/cli.mjs --url ws://127.0.0.1:PORT --join <datastore-hex>
```

Tests: `node --test conformance/ts/peer/*.test.mjs`

This slice is **not** M3c complete (no H9 two-language harness, no packaging,
no `v0.1.0`).
