# Browser peer (experimental M4a slice)

A web page with its own local zerodb store — `zerodb-wasm`
(`LocalStore<MemoryBackend>` compiled to WebAssembly) — that syncs two-way
with a Node peer over the existing WebSocket sync protocol v2.

## Pieces

- `zero-sync.mjs` — framework-free sync driver. Speaks protocol v2 as the
  client over a `WebSocket` (browser or Node >= 22): `Hello` / `HelloOk` /
  `OpsMsg` / `OpsAck`, u32-BE length-prefixed JSON frames inside WS binary
  messages, with buffering for fragmented/coalesced frames. Exposes
  `syncOnce(db, url)`, `connectPush(db, url)` (persistent v2 push session:
  the server streams new ops as they land; local `db.onChange` events push
  back immediately), and `autoSync(db, url, intervalMs)` (push session
  preferred, interval poll fallback against push-unaware servers).
- `index.html` — demo UI: create nodes, set LWW props, sync once or
  auto-sync against a peer URL. Persists to IndexedDB incrementally: an
  op-journal object store keyed by op id, appended from `db.onChange`
  events; on load the identity is restored via `ZeroDb.fromSeed(seed, ds)`
  and the journal is re-imported (with a compact/rewrite pass when the
  journal drifts from the op set).
- `test/sync-driver.test.mjs` — Node test proving two-way convergence
  between the wasm store and a NAPI `Database.serve` peer.

## Run it

1. Build the wasm package (once, or after Rust changes):

   ```sh
   cd zerodb-wasm
   npx wasm-pack build --target web --out-dir ../examples/browser-peer/pkg
   ```

   (`--out-dir` drops the pkg beside the page so the page can be served
   standalone; without it the page falls back to `../../zerodb-wasm/pkg/`,
   which only resolves when you serve the repo root.)

2. Start a Node peer with a WS sync listener (from `examples/webapp`):

   ```sh
   node examples/webapp/server.mjs   # WS sync on ws://127.0.0.1:9787
   ```

3. Serve the page statically — the wasm module must be served with the
   `application/wasm` MIME type, which both of these do:

   ```sh
   npx serve examples/browser-peer    # http://localhost:3000
   # or from the repo root: npx serve .  → /examples/browser-peer/
   # or: python -m http.server 8000
   ```

4. Create nodes in the browser, then "sync once" (or start auto-sync). A
   fresh browser store adopts the Node peer's datastore on the first sync
   (the Node peer must have at least one op); after that, edits made on
   either side converge.

## Tests

```sh
node --test examples/browser-peer/test/sync-driver.test.mjs
```

(Requires the wasm pkg built and the NAPI binding built: `cd zerodb-napi &&
npm run build:debug`.)

## Honest limitations

- **Key in IndexedDB**: the ed25519 identity seed is persisted client-side;
  any script running on the page's origin can sign as this peer.
- **Push needs a push-capable server**: `autoSync` upgrades to a persistent
  push session only when the server acks the v2 `push` capability (NAPI
  `serve` does by default); otherwise it falls back to interval polling.
- **Memory store + op journal**: state still lives in wasm memory; the
  IndexedDB op-journal is incremental but a true OPFS/sqlite-wasm backend
  remains future work.
- M4a formally depends on M3c; this slice rides ahead as an experiment, the
  same way M2 rides on M1.
