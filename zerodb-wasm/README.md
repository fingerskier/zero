# `zerodb-wasm` — browser peer (M4a-a)

wasm-bindgen wrapper over `LocalStore<MemoryBackend>`, plus durable
**IndexedDB** and **OPFS** adapters that persist signed KERNEL ops so a
browser peer can reopen after reload.

This is the M4a-a slice (WASM + IDB/OPFS). **Not** M4a complete: React
hooks and WebRTC/H6 are follow-on / pinned. Formats stay draft-1 /
unfrozen. Crate version stays `0.1.0-alpha` unpublished.

## Why JS adapters, not a Rust `StoreBackend`

`StoreBackend` is synchronous. IndexedDB and OPFS are not (OPFS sync
access handles are worker-only). The live store stays in wasm memory;
adapters journal identity + signed wire ops and restore via
`ZeroDb.fromSeed` + `importJson` + `replay`. Signed ops remain the
source of truth (PLAN preserve).

## Build

```sh
cd zerodb-wasm
wasm-pack build --target web
```

## Persist / reopen

```js
import init, { ZeroDb } from './pkg/zerodb_wasm.js'
import { openDurable } from './js/storage.mjs'

await init()
const { db, journal, restored, adapter } = await openDurable(ZeroDb, {
  name: 'zerodb',
  adapter: 'auto', // OPFS when available, unless this name already has IDB data
})

const node = db.createNode('Todo')
db.setLww(node, 'title', 'milk')
await journal.persist(db)
```

Inject `indexedDB` or `opfsRoot` in tests (see `test/doubles.mjs`).

## Tests

```sh
bash scripts/build.sh
node --test --test-concurrency=1 test/persist-reopen.test.mjs
node scripts/measure-size.mjs
```

Size-oriented artifact (this slice): **721.0 KiB raw / 262.6 KiB gzip -9**.
O4 Automerge-comparable ~250 KB gz target is not met; O4 stays open.
CI fails only if gzip exceeds 400 KiB (regression vs the ~393 KiB
pre-optimization artifact).

Identity seed is stored client-side — any script on the origin can sign
as this peer. Acceptable for this experimental slice.
