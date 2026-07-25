# zerodb todo — dogfood app

A distributed todo list built on the zerodb browser peer (`zerodb-wasm`),
flavored after the acceptance target in [doc/EXEMPLAR.md](../../doc/EXEMPLAR.md):
todos are graph nodes labeled `Todo` with `title: LWW<string>`,
`done: Flag` (enable wins), `tags: ORSet<string>`; delete is a node tombstone.
Buildless static ES modules — no bundler, no framework.

- **Persistence**: IndexedDB op-journal (identity seed + datastore id +
  incremental ops, compacted on drift) via `zero-idb.mjs`.
- **Sync**: WebSocket sync protocol v2 via `zero-sync.mjs`
  (canonical copy in `examples/browser-peer/`). Auto-sync prefers a
  persistent push session and falls back to 2s polling; status shown as
  live-push / polling / disconnected / error.
- **Model**: `todo-model.mjs` is the DOM-free data layer, exercised by
  `test/todo-model.test.mjs` under Node against both a wasm store and a
  NAPI `Database.serve` peer.

## Run locally

```sh
cd zerodb-wasm && npx wasm-pack build --target web --out-dir ../apps/todo/pkg
# serve the repo root (the page falls back to ../../zerodb-wasm/pkg too)
npx serve .   # then open /apps/todo/
```

Start a peer to sync with, e.g. from `zerodb-napi`:

```js
const { Database } = require('@zerodb/node')
const db = Database.init('todo-peer.sqlite')
db.serve(9787)
```

## Tests

```sh
node --test apps/todo/test/todo-model.test.mjs
```

## Mixed content (https vs ws://)

The deployed page (GitHub Pages) is served over **https**. Browsers exempt
`ws://localhost` and `ws://127.0.0.1` from mixed-content blocking, so syncing
with a peer on the same machine works from the hosted page. Plain `ws://` to
a **LAN IP is blocked** by the browser. To sync with another machine on your
LAN, serve this page locally over http, or wait for `wss://` support (the
peer has no TLS listener yet).

## Deploy

`.github/workflows/pages.yml` builds the wasm pkg into `apps/todo/pkg`,
copies in the canonical `zero-sync.mjs`, and deploys to GitHub Pages at
`https://fingerskier.github.io/zero/`. One-time manual step: repo
**Settings → Pages → Source: GitHub Actions**.
