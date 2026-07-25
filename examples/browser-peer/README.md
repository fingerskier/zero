# Browser peer (experimental M4a slice)

A web page with its own local zerodb store — `zerodb-wasm`
(`LocalStore<MemoryBackend>` compiled to WebAssembly) — that syncs two-way
with a Node peer over the existing WebSocket sync protocol v2.

## Pieces

- `zero-sync.mjs` — framework-free sync driver. Speaks protocol v2 as the
  client over a `WebSocket` (browser or Node >= 22): `Hello` / `HelloOk` /
  `OpsMsg` / `OpsAck`, u32-BE length-prefixed JSON frames inside WS binary
  messages, with buffering for fragmented/coalesced frames. Exposes
  `syncOnce(db, url)` and `autoSync(db, url, intervalMs)` (poll-only; the
  wasm store has no change events yet).
- `index.html` — demo UI: create nodes, set LWW props, sync once or
  auto-sync against a peer URL. Persists to IndexedDB: identity seed +
  datastore id + full export bundle, restored on load via
  `ZeroDb.fromSeed(seed, ds)` + `importJson(bundle)`.
- `test/sync-driver.test.mjs` — Node test proving two-way convergence
  between the wasm store and a NAPI `Database.serve` peer.

## Run it

1. Build the wasm package (once, or after Rust changes):

   ```sh
   cd zerodb-wasm
   npx wasm-pack build --target web
   ```

2. Start a Node peer with a WS sync listener (from `examples/webapp`):

   ```sh
   node examples/webapp/server.mjs   # WS sync on ws://127.0.0.1:9787
   ```

3. Serve the repo root statically and open the page — the wasm module must be
   served with the `application/wasm` MIME type, which both of these do:

   ```sh
   npx serve .          # then open http://localhost:3000/examples/browser-peer/
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
- **Poll-only auto-sync**: no dirty-flag or server push; `autoSync` re-syncs
  on an interval.
- **Memory store**: persistence is whole-bundle export to IndexedDB on each
  change — fine at MVP scale, not incremental.
- M4a formally depends on M3c; this slice rides ahead as an experiment, the
  same way M2 rides on M1.
