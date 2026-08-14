# `@zerodb/node` — experimental M2 NAPI SDK

Node.js binding over the M1 `LocalStore` (SQLite). Experimental M2 exit (`v0.1.0-sdk`, 2026-08-14). **Not** a format freeze and **not** SPEC-complete M2.

## Build

Requires Rust (MSVC on Windows) and Node ≥ 18.

```bash
cd zerodb-napi
npm install
npm run build    # release native addon
npm test
```

## Minimal usage

```js
const { Database } = require('@zerodb/node') // or require('./index.js') from this package

const db = Database.init('./todo.sqlite')
const node = db.createNode('Todo')
db.setLww(node, 'title', 'milk')
console.log(db.getLww(node, 'title'))
db.close() // required before deleting the file on Windows
```

## API surface (current)

| Method | Notes |
|--------|--------|
| `Database.init` / `open` | path to SQLite file |
| `close` | drop connection |
| `createNode` / `deleteNode` | graph entities (nodes only) |
| `setLww` / `getLww` / `getProp` | property read/write |
| `gcounterInc`, `counterInc`/`Dec`, `setAdd`/`Remove`, `flagEnable`/`Disable` | M1 CRDT helpers |
| `listNodes`, `inspect`, `replay` | introspection |
| `exportJson` / `importJson` | format-1 op bundles |
| `subscribe` / `unsubscribe` | live change callbacks; async delivery (next tick) |
| `query` | O3 minimal `MATCH/WHERE/RETURN/ORDER BY/LIMIT`; rows keyed `"t.title"` |
| `applySchema` | pin or SCHEMA IR JSON; returns `{ schemaId, epoch }` |
| `applyCrdtVector` | replay one KERNEL `crdt-apply` fixture through the core kernel |
| `serve(port, allowInsecureLan?)` / `stopServe` | WebSocket sync listener (see Sync) |
| `connectPeer(url)` | one two-way sync session against `ws://host:port` |
| `autoConnect(url, intervalMs)` / `disconnect` | background live sync with retry/backoff |

### Sync

```js
const port = dbA.serve(0)               // loopback only; 0 = OS-assigned port
const s = dbB.connectPeer(`ws://127.0.0.1:${port}`)
// { accepted, skipped, sent, remoteAccepted, remoteSkipped }
dbA.stopServe()
```

- `serve(port)` binds `127.0.0.1` only. `serve(port, true)`
  (`allowInsecureLan`) binds `0.0.0.0` — the wire is **plaintext and
  unauthenticated**: anyone on the network can read and write the store.
  Only enable on trusted LANs (mirrors the CLI `--allow-insecure-lan`).
- All sync sockets carry 30s read/write timeouts; a stalled peer surfaces
  as a sync error (`{kind:'sync', error}` serve event or a thrown/
  `sync-error` on the connect side) instead of a hang. Sessions are atomic
  (`import_bundle`), so a timed-out session never leaves partial state.
- Each incoming connection is served on its own thread; the store lock is
  taken only after the WebSocket handshake, so a stalled pre-handshake
  peer never blocks the accept loop or the store.

### Subscribe events

```js
const sub = db.subscribe((e) => console.log(e))
// {kind:'op', method:'setLww', node, key, opId}   local mutations
// {kind:'import', accepted, skipped}              importJson
// {kind:'replay'}                                 replay
db.unsubscribe(sub)
```

**Versioned-experimental:** the JSON `WireOp` bundle format, the SQLite layout,
and these event shapes are pre-freeze and may change without notice until a
Decision Log format freeze. Do not persist bundles across SDK versions.

## Not in this tag (still deferred)

- MVRegister, RGA, LWWMap (until an app needs them)
- E11 artifact-size / performance budgets
- query-scoped subscribe; interactive `repl`

See [plan/LEDGER.md](../plan/LEDGER.md). Binding parity fixtures are in `test/m2-parity.test.mjs`.
