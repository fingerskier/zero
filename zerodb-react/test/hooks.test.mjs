// React hooks over zerodb-wasm + openDurable.
//
// Requires the wasm pkg:
//   bash zerodb-wasm/scripts/build.sh
//
// Run: node --test --test-concurrency=1 zerodb-react/test/hooks.test.mjs
// One wasm heap — keep this file a single test so cases cannot interleave.

import test from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { createElement, useEffect } from 'react'
import renderer, { act } from 'react-test-renderer'

import { ADAPTER_INDEXEDDB, ADAPTER_OPFS } from '../../zerodb-wasm/js/storage.mjs'
import { fakeIndexedDB, memoryOpfsRoot } from '../../zerodb-wasm/test/doubles.mjs'
import {
  ZeroDbProvider,
  useMutation,
  useNode,
  useQuery,
  useSyncStatus,
  useZeroDb,
} from '../index.mjs'

const here = path.dirname(fileURLToPath(import.meta.url))
const pkgDir = path.join(here, '..', '..', 'zerodb-wasm', 'pkg')

assert.ok(
  fs.existsSync(path.join(pkgDir, 'zerodb_wasm.js')),
  'wasm pkg missing — run: bash zerodb-wasm/scripts/build.sh',
)

const wasm = await import(
  new URL(`file://${path.join(pkgDir, 'zerodb_wasm.js').replaceAll('\\', '/')}`)
)
await wasm.default({
  module_or_path: fs.readFileSync(path.join(pkgDir, 'zerodb_wasm_bg.wasm')),
})
const { ZeroDb } = wasm

async function flush(times = 8) {
  for (let i = 0; i < times; i++) {
    await act(async () => {
      await new Promise(r => setImmediate(r))
    })
  }
}

async function waitFor(read, check, { tries = 40 } = {}) {
  let last
  for (let i = 0; i < tries; i++) {
    await flush(2)
    last = read()
    try {
      check(last)
      return last
    } catch (err) {
      last = err
    }
  }
  throw last instanceof Error ? last : new Error(`waitFor failed: ${JSON.stringify(last)}`)
}

function Harness({ onTick, query = 'MATCH (t:Todo) RETURN t.title', nodeId }) {
  const ctx = useZeroDb()
  const sync = useSyncStatus()
  const q = useQuery(query)
  const node = useNode(nodeId)
  const mutate = useMutation()
  useEffect(() => {
    onTick({ ctx, sync, q, node, mutate })
  })
  return null
}

function mount(props) {
  const snap = { current: null }
  const tree = renderer.create(
    createElement(
      ZeroDbProvider,
      { ZeroDb, persistOnChange: true, ...props },
      createElement(Harness, {
        nodeId: props.nodeId,
        query: props.query,
        onTick: v => { snap.current = v },
      }),
    ),
  )
  return { tree, snap }
}

test('hooks: durable open, mutate+reopen, onChange, occupied-IDB auto', async () => {
  {
    const indexedDB = fakeIndexedDB()
    const { tree, snap } = mount({
      name: 'hooks-open',
      adapter: 'indexeddb',
      indexedDB,
    })
    const ready = await waitFor(
      () => snap.current,
      v => {
        assert.ok(v)
        assert.equal(v.sync, 'ready')
        assert.equal(v.ctx.status, 'ready')
        assert.equal(v.ctx.restored, false)
        assert.equal(v.ctx.adapter, ADAPTER_INDEXEDDB)
      },
    )
    assert.equal(ready.q.ready, true)
    assert.equal(ready.q.rows.length, 0)
    tree.unmount()
  }

  {
    const indexedDB = fakeIndexedDB()
    const first = mount({
      name: 'hooks-reopen',
      adapter: 'indexeddb',
      indexedDB,
    })
    const opened = await waitFor(
      () => first.snap.current,
      v => {
        assert.ok(v)
        assert.equal(v.sync, 'ready')
      },
    )
    let nodeId
    await act(async () => {
      nodeId = await opened.mutate.createNode('Todo')
      await opened.mutate.setLww(nodeId, 'title', 'persisted')
      await opened.mutate.flagEnable(nodeId, 'done')
    })
    const afterWrite = await waitFor(
      () => first.snap.current,
      v => {
        assert.ok(v?.q.ready)
        assert.equal(v.q.rows.length, 1)
        assert.equal(v.q.rows[0]['t.title'], 'persisted')
      },
    )
    assert.equal(afterWrite.node, null)
    first.tree.unmount()

    const second = mount({
      name: 'hooks-reopen',
      adapter: 'indexeddb',
      indexedDB,
      nodeId,
    })
    const restored = await waitFor(
      () => second.snap.current,
      v => {
        assert.ok(v)
        assert.equal(v.sync, 'ready')
        assert.equal(v.ctx.restored, true)
        assert.equal(v.q.rows.length, 1)
        assert.equal(v.q.rows[0]['t.title'], 'persisted')
        assert.ok(v.node)
        assert.equal(v.node.id, nodeId)
        assert.equal(v.node.props.title, 'persisted')
        assert.equal(v.node.props.done, true)
      },
    )
    second.tree.unmount()
  }

  {
    const indexedDB = fakeIndexedDB()
    const { tree, snap } = mount({
      name: 'hooks-onchange',
      adapter: 'indexeddb',
      indexedDB,
    })
    const opened = await waitFor(
      () => snap.current,
      v => assert.equal(v?.sync, 'ready'),
    )
    assert.equal(opened.q.rows.length, 0)
    await act(async () => {
      const id = await opened.mutate.createNode('Todo')
      await opened.mutate.setLww(id, 'title', 'live')
    })
    await waitFor(
      () => snap.current,
      v => {
        assert.equal(v.q.rows.length, 1)
        assert.equal(v.q.rows[0]['t.title'], 'live')
      },
    )
    tree.unmount()
  }

  {
    const indexedDB = fakeIndexedDB()
    const first = mount({
      name: 'zerodb-todo',
      adapter: 'indexeddb',
      indexedDB,
    })
    const opened = await waitFor(
      () => first.snap.current,
      v => assert.equal(v?.sync, 'ready'),
    )
    let nodeId
    await act(async () => {
      nodeId = await opened.mutate.createNode('Todo')
      await opened.mutate.setLww(nodeId, 'title', 'kept-idb')
    })
    await waitFor(
      () => first.snap.current,
      v => assert.equal(v.q.rows[0]?.['t.title'], 'kept-idb'),
    )
    first.tree.unmount()

    const second = mount({
      name: 'zerodb-todo',
      adapter: 'auto',
      indexedDB,
      opfsRoot: memoryOpfsRoot(),
      nodeId,
    })
    const auto = await waitFor(
      () => second.snap.current,
      v => {
        assert.equal(v?.sync, 'ready')
        assert.equal(v.ctx.adapter, ADAPTER_INDEXEDDB)
        assert.equal(v.ctx.restored, true)
        assert.equal(v.q.rows[0]?.['t.title'], 'kept-idb')
        assert.equal(v.node?.props.title, 'kept-idb')
      },
    )
    second.tree.unmount()
  }

  {
    const { tree, snap } = mount({
      name: 'hooks-opfs',
      adapter: 'opfs',
      opfsRoot: memoryOpfsRoot(),
    })
    const opened = await waitFor(
      () => snap.current,
      v => {
        assert.equal(v?.sync, 'ready')
        assert.equal(v.ctx.adapter, ADAPTER_OPFS)
      },
    )
    await act(async () => {
      const id = await opened.mutate(db => {
        const node = db.createNode('Todo')
        db.setLww(node, 'title', 'via-fn')
        return node
      })
      assert.ok(id)
    })
    await waitFor(
      () => snap.current,
      v => assert.equal(v.q.rows[0]?.['t.title'], 'via-fn'),
    )
    tree.unmount()
  }

  {
    const indexedDB = fakeIndexedDB()
    const { tree, snap } = mount({
      name: 'hooks-offline',
      adapter: 'indexeddb',
      indexedDB,
    })
    assert.equal(snap.current?.sync ?? 'offline', 'offline')
    const opened = await waitFor(
      () => snap.current,
      v => assert.equal(v?.sync, 'ready'),
    )
    tree.unmount()
  }

  {
    const indexedDB = fakeIndexedDB()
    let renders = 0
    const snap = { current: null }
    function Probe() {
      const ctx = useZeroDb()
      const sync = useSyncStatus()
      const q = useQuery('MATCH (t:Todo) RETURN t.title', { unused: 1 })
      renders += 1
      useEffect(() => {
        snap.current = { ctx, sync, q, renders }
      })
      return null
    }
    const tree = renderer.create(
      createElement(
        ZeroDbProvider,
        { ZeroDb, name: 'hooks-params', adapter: 'indexeddb', indexedDB },
        createElement(Probe),
      ),
    )
    const ready = await waitFor(
      () => snap.current,
      v => assert.equal(v?.sync, 'ready'),
    )
    const atReady = ready.renders
    await flush(12)
    assert.ok(
      snap.current.renders - atReady < 8,
      `inline params re-rendered too often: ${atReady} -> ${snap.current.renders}`,
    )
    assert.equal(snap.current.q.ready, true)
    tree.unmount()
  }

  {
    const indexedDB = fakeIndexedDB()
    const first = mount({
      name: 'store-a',
      adapter: 'indexeddb',
      indexedDB,
    })
    const opened = await waitFor(
      () => first.snap.current,
      v => assert.equal(v?.sync, 'ready'),
    )
    const peerA = opened.ctx.db.peerId()
    await act(async () => {
      const id = await opened.mutate.createNode('Todo')
      await opened.mutate.setLww(id, 'title', 'only-a')
    })
    await waitFor(
      () => first.snap.current,
      v => assert.equal(v.q.rows[0]?.['t.title'], 'only-a'),
    )
    first.tree.update(
      createElement(
        ZeroDbProvider,
        { ZeroDb, persistOnChange: true, name: 'store-b', adapter: 'indexeddb', indexedDB },
        createElement(Harness, {
          onTick: v => { first.snap.current = v },
        }),
      ),
    )
    await waitFor(
      () => first.snap.current,
      v => {
        assert.equal(v?.sync, 'ready')
        assert.notEqual(v.ctx.db.peerId(), peerA)
        assert.equal(v.q.rows.length, 0)
      },
    )
    first.tree.unmount()
  }

  {
    const indexedDB = fakeIndexedDB()
    const { tree, snap } = mount({
      name: 'hooks-persist-err',
      adapter: 'indexeddb',
      indexedDB,
    })
    const opened = await waitFor(
      () => snap.current,
      v => assert.equal(v?.sync, 'ready'),
    )
    const boom = new Error('quota')
    opened.ctx.journal.persist = async () => { throw boom }
    await act(async () => {
      await assert.rejects(() => opened.mutate.createNode('Todo'), /quota/)
    })
    await waitFor(
      () => snap.current,
      v => assert.equal(v.ctx.persistError, boom),
    )
    tree.unmount()
  }

  {
    const indexedDB = fakeIndexedDB()
    const { tree, snap } = mount({
      name: 'hooks-edge',
      adapter: 'indexeddb',
      indexedDB,
    })
    const opened = await waitFor(
      () => snap.current,
      v => assert.equal(v?.sync, 'ready'),
    )
    let queries = 0
    const orig = opened.ctx.db.query.bind(opened.ctx.db)
    opened.ctx.db.query = (...args) => {
      queries += 1
      return orig(...args)
    }
    await act(async () => {
      const a = await opened.mutate.createNode('Todo')
      const b = await opened.mutate.createNode('Todo')
      await flush(6)
      const before = queries
      await opened.mutate(db => db.createEdge('rel', a, b))
      await flush(6)
      assert.ok(queries > before, `createEdge did not invalidate query (${before} -> ${queries})`)
    })
    tree.unmount()
  }
})
