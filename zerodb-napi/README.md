# `@zerodb/node` — experimental M2 NAPI SDK

Node.js binding over the M1 `LocalStore` (SQLite). **Not** a format freeze and **not** M2 exit (`v0.1.0-sdk`).

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

## Not yet (SPEC M2 exit)

- O3 query + schema apply / TS→IR  
- MVRegister, RGA, LWWMap  
- Binding parity vector suite / E11 budgets  

See [plan/LEDGER.md](../plan/LEDGER.md) M2 subtasks.
