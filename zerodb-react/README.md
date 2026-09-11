# `@zerodb/react` — optional hooks (M4a slice)

First-cut React surface over the **live** `zerodb-wasm` peer and the
IndexedDB / OPFS durable adapters (`openDurable`). SPEC §5.4 names
`useQuery`, `useNode`, `useMutation`, `useSyncStatus` — this package
matches those names and wraps the WASM API that exists today.

**Not M4a complete.** WebRTC / H6 stay pinned. The typed
`db.query(Post).where(p => p.published.eq(true))` DSL does not exist
yet; do not invent it. Formats stay draft-1 / unfrozen. Package version
is `0.1.0-alpha`, `"private": true` — do not `npm publish`.

## Layout

A sibling optional package (`@zerodb/node`, `@zerodb/ts-to-ir`), not
inside the wasm crate and not a rewrite of `examples/browser-peer`.
Hooks are JS sitting beside wasm so they cannot bloat the gzip artifact
(O4 still open: 262.6 KiB vs ~250 KB; CI ceiling 400 KiB).

## Open a durable store

```js
import init, { ZeroDb } from 'zerodb-wasm' // or the built pkg URL
import { ZeroDbProvider, useQuery, useNode, useMutation, useSyncStatus } from '@zerodb/react'

await init()

function App() {
  const sync = useSyncStatus() // 'offline' | 'ready' — local store only
  const { rows } = useQuery('MATCH (t:Todo) RETURN t.title')
  const mutate = useMutation()

  return (
    <div>
      <span>Store: {sync}</span>
      <button onClick={async () => {
        const id = await mutate.createNode('Todo')
        await mutate.setLww(id, 'title', 'milk')
      }}>add</button>
      {rows.map((row, i) => <div key={i}>{row['t.title']}</div>)}
    </div>
  )
}

root.render(
  <ZeroDbProvider ZeroDb={ZeroDb} name="zerodb" adapter="auto">
    <App />
  </ZeroDbProvider>
)
```

`ZeroDbProvider` calls `openDurable` once. Occupied IndexedDB names stay
on IndexedDB when `adapter: 'auto'` (M4a-a). The live store is still
`MemoryBackend`; the journal is signed KERNEL ops.

### Mutations

`useMutation()` is persist-on-write:

```js
const mutate = useMutation()
await mutate(db => {
  const id = db.createNode('Todo')
  db.setLww(id, 'title', 'milk')
  return id
})
// or: mutate.createNode / setLww / flagEnable / counterInc / …
```

Writes go through `ZeroDb`, then `journal.persist`. There is no second
persistence path.

### Subscribe

`useQuery(q, params?)` re-runs `db.query` / `db.queryWith` after
`onChange`. `useNode(id)` re-reads `listNodes()`. Callbacks are deferred
with `queueMicrotask` so they do not re-enter the wasm borrow.

## Honest limitations

- **Seed in origin storage.** The ed25519 identity seed is in IDB/OPFS;
  any script on the origin can sign as this peer.
- **MemoryBackend + journal.** State lives in wasm memory; adapters
  journal signed ops and restore via `fromSeed` + `importJson` + `replay`.
- **No typed query DSL.** O3 string queries only (`MATCH (t:Todo) …`).
- **`useSyncStatus` is not live sync.** It reports local open readiness
  (`offline` / `ready`). Wiring `examples/browser-peer/zero-sync.mjs`
  would sprawl; WebRTC / H6 are not started.
- **Not M4a complete.** Hooks + persist/reopen ≠ browser/WebRTC product.

## Tests

```sh
bash zerodb-wasm/scripts/build.sh
cd zerodb-react && npm test
```

Uses the M4a-a fake-idb / OPFS doubles in `zerodb-wasm/test/doubles.mjs`.

## Thin example

`examples/react-hooks/` — small static page, not a rewrite of the vanilla
browser-peer demo.
