# ZeroDB peer webapp

One `@zerodb/node` LocalStore per server process. Each instance serves a single-pane UI over its own store, listens for sync on a loopback WebSocket (`db.serve`), and — when `ZERO_PEER` is set — stays converged with that peer via `db.autoConnect` (re-sync on local mutation + 1s poll for remote changes). GunDB-style: connect once, stay converged.

## Run two instances

```bash
cd zerodb-napi && npm run build   # once, builds the native addon
cd ../examples/webapp

# instance 1
PORT=8787 WS_PORT=9787 node server.mjs

# instance 2 (separate terminal) — auto-syncs with instance 1
PORT=8788 WS_PORT=9788 ZERO_PEER=ws://127.0.0.1:9787 node server.mjs
```

Open http://localhost:8787 and http://localhost:8788 in two tabs. Create/set/delete nodes in either tab and watch the other converge within ~1s (instance 2 pushes on mutation and polls instance 1; instance 1's changes arrive on instance 2's next poll).

Env vars: `PORT` (HTTP UI, default 8787), `WS_PORT` (sync listener, default PORT+1000), `ZERO_PEER` (optional `ws://host:port` to auto-sync with), `ZERO_DATA` (sqlite path, default `data/peer-<PORT>.sqlite`).

Data lives in `examples/webapp/data/*.sqlite` (disposable, experimental format — delete freely).

## What it demonstrates

- CRUD + LWW/set/flag props via the NAPI SDK
- `subscribe` live change events streamed to the browser (SSE), including sync sessions
- `serve` + `autoConnect` WebSocket sync (protocol v2 two-way, set-diff) with dirty-flag re-sync, interval poll, and reconnect backoff
