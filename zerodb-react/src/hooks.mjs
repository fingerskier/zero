// Optional React surface over the live zerodb-wasm peer + durable adapters.
//
// Wraps ZeroDb + openDurable (IndexedDB / OPFS journal of signed KERNEL ops).
// SPEC §5.4 names (useQuery / useNode / useMutation / useSyncStatus) — the
// typed `db.query(Post).where(...)` DSL does not exist yet; query is the O3
// string. useSyncStatus is local durable-store readiness, not WebSocket or
// WebRTC sync. Not M4a complete. H6 stays pinned.

import {
  createContext,
  createElement,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react'
import { openDurable } from '../../zerodb-wasm/js/storage.mjs'

const ZeroDbContext = createContext(null)

function notReady() {
  throw new Error('useZeroDb must be used within ZeroDbProvider')
}

function safeOff(db, id) {
  try {
    db.offChange(id)
  } catch {
    /* instance already dropped */
  }
}

/**
 * Subscribe to `db.onChange` and bump a version after the wasm borrow drops.
 * Callbacks must not re-enter ZeroDb synchronously (queueMicrotask).
 */
function useGraphVersion(db) {
  const [version, setVersion] = useState(0)
  useEffect(() => {
    if (!db) return undefined
    let mounted = true
    const id = db.onChange(() => {
      queueMicrotask(() => {
        if (mounted) setVersion(v => v + 1)
      })
    })
    return () => {
      mounted = false
      safeOff(db, id)
    }
  }, [db])
  return version
}

/**
 * Open one durable ZeroDb for the tree. Passes `name` / `adapter` /
 * `indexedDB` / `opfsRoot` / `getDirectory` through to `openDurable`.
 * Occupied-IDB auto-select stays in the adapter (M4a-a P1).
 */
export function ZeroDbProvider({
  children,
  ZeroDb,
  name = 'zerodb',
  adapter = 'auto',
  indexedDB,
  opfsRoot,
  getDirectory,
  persistOnChange = true,
  fallback = null,
}) {
  const [state, setState] = useState({
    status: 'loading',
    db: null,
    journal: null,
    restored: false,
    adapter: null,
    opCount: 0,
    error: null,
  })
  const openedRef = useRef(null)
  const aliveRef = useRef(true)

  useEffect(() => {
    let cancelled = false
    aliveRef.current = true
    openedRef.current = null
    openDurable(ZeroDb, { name, adapter, indexedDB, opfsRoot, getDirectory })
      .then(opened => {
        if (cancelled) {
          if (typeof opened.db?.free === 'function') opened.db.free()
          return
        }
        openedRef.current = opened
        setState({
          status: 'ready',
          db: opened.db,
          journal: opened.journal,
          restored: opened.restored,
          adapter: opened.adapter,
          opCount: opened.opCount,
          error: null,
        })
      })
      .catch(error => {
        if (!cancelled) {
          setState(s => ({ ...s, status: 'error', error }))
        }
      })
    return () => {
      cancelled = true
      aliveRef.current = false
      openedRef.current = null
      // Do not `db.free()` here: `onChange` persist/read microtasks may
      // still run this turn, and wasm-bindgen treats a freed pointer as
      // a hard error. Abandoned instances last until page unload.
    }
  }, [ZeroDb, name, adapter, indexedDB, opfsRoot, getDirectory])

  useEffect(() => {
    const db = state.db
    const journal = state.journal
    if (!db || !journal || persistOnChange === false) return undefined
    const id = db.onChange(() => {
      queueMicrotask(() => {
        if (!aliveRef.current) return
        journal.persist(db).catch(() => {})
      })
    })
    return () => safeOff(db, id)
  }, [state.db, state.journal, persistOnChange])

  const persist = useCallback(async () => {
    if (!state.db || !state.journal) throw new Error('ZeroDb is not ready')
    await state.journal.persist(state.db)
  }, [state.db, state.journal])

  const value = useMemo(
    () => ({ ...state, persist }),
    [state, persist],
  )

  const tree = fallback != null && state.status !== 'ready' ? fallback : children
  return createElement(ZeroDbContext.Provider, { value }, tree)
}

export function useZeroDb() {
  const ctx = useContext(ZeroDbContext)
  if (!ctx) notReady()
  return ctx
}

/**
 * Re-run an O3 string query (`MATCH … RETURN …`) after each `onChange`.
 * `params` is optional and passed to `queryWith` as JSON.
 */
export function useQuery(query, params) {
  const { db, status } = useZeroDb()
  const version = useGraphVersion(db)
  const paramsKey = params === undefined ? '' : JSON.stringify(params)
  const [result, setResult] = useState({ rows: [], error: null, ready: false })

  useEffect(() => {
    if (!db || status !== 'ready' || !query) {
      setResult({ rows: [], error: null, ready: false })
      return
    }
    let mounted = true
    try {
      const raw = params === undefined
        ? db.query(query)
        : db.queryWith(query, paramsKey)
      const rows = Array.isArray(raw) ? raw : []
      if (mounted) setResult({ rows, error: null, ready: true })
    } catch (error) {
      if (mounted) setResult({ rows: [], error, ready: true })
    }
    return () => { mounted = false }
  }, [db, status, version, query, params, paramsKey])

  return result
}

/** One materialized node `{ id, label, deleted, props }` from `listNodes`. */
export function useNode(id) {
  const { db, status } = useZeroDb()
  const version = useGraphVersion(db)
  const [node, setNode] = useState(null)

  useEffect(() => {
    if (!db || status !== 'ready' || !id) {
      setNode(null)
      return
    }
    let mounted = true
    try {
      const listed = db.listNodes()
      const found = Array.isArray(listed) ? listed.find(n => n.id === id) : null
      if (mounted) setNode(found ?? null)
    } catch {
      if (mounted) setNode(null)
    }
    return () => { mounted = false }
  }, [db, status, version, id])

  return node
}

function bindMutators(mutate) {
  mutate.createNode = label => mutate(db => db.createNode(label))
  mutate.deleteNode = node => mutate(db => db.deleteNode(node))
  mutate.setLww = (node, key, value) => mutate(db => db.setLww(node, key, value))
  mutate.counterInc = (node, key, n = 1) => mutate(db => db.counterInc(node, key, n))
  mutate.counterDec = (node, key, n = 1) => mutate(db => db.counterDec(node, key, n))
  mutate.gcounterInc = (node, key, n = 1) => mutate(db => db.gcounterInc(node, key, n))
  mutate.setAdd = (node, key, value) => mutate(db => db.setAdd(node, key, value))
  mutate.setRemove = (node, key, value) => mutate(db => db.setRemove(node, key, value))
  mutate.flagEnable = (node, key) => mutate(db => db.flagEnable(node, key))
  mutate.flagDisable = (node, key) => mutate(db => db.flagDisable(node, key))
  return mutate
}

/**
 * Write through live ZeroDb methods, then `journal.persist`.
 * `mutate(db => { … })` or `mutate.setLww(id, 'title', 'milk')`.
 * Not the SPEC sketch `p => p.viewCount.increment(1)` DSL.
 */
export function useMutation() {
  const { db, journal, status, persist } = useZeroDb()

  const mutate = useCallback(async fn => {
    if (!db || !journal || status !== 'ready') {
      throw new Error('ZeroDb is not ready')
    }
    const result = fn(db)
    await persist()
    return result
  }, [db, journal, status, persist])

  return useMemo(() => {
    const wrapped = mutate
    wrapped.persist = persist
    return bindMutators(wrapped)
  }, [mutate, persist])
}

/**
 * Local durable-store readiness only.
 * `'offline'` while opening / on error; `'ready'` after `openDurable`.
 * Does not wrap `zero-sync.mjs` or WebRTC (H6 pinned).
 */
export function useSyncStatus() {
  const { status } = useZeroDb()
  if (status === 'ready') return 'ready'
  return 'offline'
}
