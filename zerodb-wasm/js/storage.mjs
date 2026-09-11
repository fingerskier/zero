// Durable browser storage adapters for zerodb-wasm (M4a-a).
//
// Live store is still `LocalStore<MemoryBackend>` inside the wasm module
// (StoreBackend is synchronous; IDB/OPFS are not). These adapters persist
// the identity seed + signed KERNEL wire ops so a browser peer can reopen
// after reload. Replay rematerializes. Signed ops remain the source of truth.
//
// Auto-select prefers OPFS when a directory handle is available *and* the
// named IndexedDB is empty. An occupied IDB name stays on IndexedDB so
// Chrome users who already have `zerodb-todo` / `zerodb-browser-peer` do
// not mint a fresh identity. React hooks / WebRTC are not this slice.

export const ADAPTER_INDEXEDDB = 'indexeddb'
export const ADAPTER_OPFS = 'opfs'

/** Capability hint. `auto` prefers OPFS when present; `openJournal` may keep occupied IDB. */
export function detectAdapter(opts = {}) {
  if (opts.adapter && opts.adapter !== 'auto') return opts.adapter
  if (opts.opfsRoot || opts.getDirectory || globalThis.navigator?.storage?.getDirectory) {
    return ADAPTER_OPFS
  }
  if (opts.indexedDB || globalThis.indexedDB) return ADAPTER_INDEXEDDB
  throw new Error('no durable browser storage adapter available')
}

function idbFactory(opts = {}) {
  return opts.indexedDB ?? globalThis.indexedDB
}

function opfsAvailable(opts = {}) {
  return !!(opts.opfsRoot || opts.getDirectory || globalThis.navigator?.storage?.getDirectory)
}

/**
 * Open a named journal on the requested (or auto-detected) adapter.
 * Inject `indexedDB` / `opfsRoot` for tests (fake-idb / OPFS doubles).
 * `auto`: keep IndexedDB when that name already has identity/ops.
 */
export async function openJournal(name, opts = {}) {
  const requested = opts.adapter && opts.adapter !== 'auto' ? opts.adapter : 'auto'
  if (requested === ADAPTER_INDEXEDDB) return IndexedDbJournal.open(name, opts)
  if (requested === ADAPTER_OPFS) return OpfsJournal.open(name, opts)
  if (requested !== 'auto') throw new Error(`unknown adapter ${requested}`)

  const factory = idbFactory(opts)
  if (factory) {
    const peek = await peekIndexedDb(factory, name)
    if (peek.occupied) {
      return new IndexedDbJournal(name, new IdbDriver(peek.db, factory, name))
    }
    peek.db.close()
  }
  if (opfsAvailable(opts)) return OpfsJournal.open(name, opts)
  if (factory) return IndexedDbJournal.open(name, opts)
  throw new Error('no durable browser storage adapter available')
}

/**
 * Product open path: restore a wasm `ZeroDb` from durable storage, or mint
 * a fresh identity. Returns `{ db, journal, restored, opCount, adapter }`.
 */
export async function openDurable(ZeroDb, opts = {}) {
  const journal = await openJournal(opts.name ?? 'zerodb', opts)
  const restored = await journal.restore(ZeroDb)
  return { ...restored, journal }
}

function req(r) {
  return new Promise((resolve, reject) => {
    r.onsuccess = () => resolve(r.result)
    r.onerror = () => reject(r.error)
  })
}

function isNotFound(err) {
  return !!err && (err.name === 'NotFoundError' || err.code === 8 || err.code === 'NOT_FOUND_ERR')
}

/**
 * Open an existing DB at its current version (no downgrade). A missing
 * name is created at version 1 with `state` + `journal` stores.
 */
function openIndexedDb(factory, name) {
  return new Promise((resolve, reject) => {
    const r = factory.open(name)
    r.onupgradeneeded = () => {
      const db = r.result
      if (!db.objectStoreNames.contains('state')) db.createObjectStore('state')
      if (!db.objectStoreNames.contains('journal')) db.createObjectStore('journal')
    }
    r.onsuccess = () => resolve(r.result)
    r.onerror = () => reject(r.error)
  })
}

async function peekIndexedDb(factory, name) {
  const db = await openIndexedDb(factory, name)
  const driver = new IdbDriver(db, factory, name)
  const ident = await driver.getIdentity()
  if (ident && (ident.seed || ident.ds || ident.bundle)) {
    return { db, occupied: true }
  }
  const ops = await driver.listOps()
  return { db, occupied: ops.length > 0 }
}

/**
 * Shared restore/persist: identity `{ seed, ds }` plus a journal of wire ops.
 * Compact/rewrite when the journal drifts from the live op set.
 * persist/replace/restore/reset share a per-instance queue so overlapping
 * `onChange` → persist cannot last-writer-wins an OPFS rewrite.
 */
export class DurableJournal {
  #driver
  #persisted = new Set()
  #kind
  #name
  #tail = Promise.resolve()

  constructor(kind, name, driver) {
    this.#kind = kind
    this.#name = name
    this.#driver = driver
  }

  get kind() {
    return this.#kind
  }

  get name() {
    return this.#name
  }

  #serialized(fn) {
    const next = this.#tail.then(fn, fn)
    this.#tail = next.then(() => {}, () => {})
    return next
  }

  /**
   * Restore a store from the journal, or mint a fresh one.
   * Returns `{ db, restored, opCount, adapter }`.
   */
  restore(ZeroDb) {
    return this.#serialized(() => this.#restoreInner(ZeroDb))
  }

  async #restoreInner(ZeroDb) {
    const saved = await this.#driver.getIdentity()
    if (!saved) {
      const db = new ZeroDb()
      await this.#driver.setIdentity({ seed: db.seedHex(), ds: db.datastoreId() })
      return { db, restored: false, opCount: 0, adapter: this.#kind }
    }
    const db = ZeroDb.fromSeed(saved.seed, saved.ds)
    let ops = await this.#driver.listOps()
    if (ops.length === 0 && saved.bundle) {
      ops = JSON.parse(saved.bundle).ops
    }
    if (ops.length > 0) {
      const res = db.importJson(JSON.stringify({
        format: 1,
        datastore_id: saved.ds,
        ops,
      }))
      if (res.accepted > 0) db.replay()
    }
    if (ops.length !== db.opCount()) {
      await this.#driver.replaceOps(JSON.parse(db.exportJson()).ops)
    }
    for (const id of db.opIds()) this.#persisted.add(id)
    await this.#driver.setIdentity({ seed: db.seedHex(), ds: db.datastoreId() })
    return { db, restored: true, opCount: db.opCount(), adapter: this.#kind }
  }

  /**
   * Append any not-yet-journaled ops; rewrite identity when the datastore
   * id changed (first sync may adopt the peer's datastore).
   */
  persist(db) {
    return this.#serialized(() => this.#persistInner(db))
  }

  async #persistInner(db) {
    const ids = db.opIds().filter(id => !this.#persisted.has(id))
    if (ids.length > 0) {
      const ops = JSON.parse(db.exportOpsByIds(ids))
      await this.#driver.putOps(ops)
      for (const op of ops) this.#persisted.add(op.id)
    }
    const meta = await this.#driver.getIdentity()
    if (!meta || meta.ds !== db.datastoreId()) {
      await this.#driver.setIdentity({ seed: db.seedHex(), ds: db.datastoreId() })
    }
  }

  /** Delete identity + journal for this name. */
  reset() {
    return this.#serialized(async () => {
      this.#persisted.clear()
      await this.#driver.reset()
    })
  }
}

class IdbDriver {
  constructor(db, factory, name) {
    this.db = db
    this.factory = factory
    this.name = name
  }

  #get(key) {
    return req(this.db.transaction('state').objectStore('state').get(key))
  }

  #set(key, value) {
    return req(this.db.transaction('state', 'readwrite').objectStore('state').put(value, key))
  }

  getIdentity() {
    return this.#get('peer')
  }

  setIdentity(peer) {
    return this.#set('peer', peer)
  }

  listOps() {
    return req(this.db.transaction('journal').objectStore('journal').getAll())
  }

  putOps(ops) {
    return new Promise((resolve, reject) => {
      const tx = this.db.transaction('journal', 'readwrite')
      const store = tx.objectStore('journal')
      for (const op of ops) store.put(op, op.id)
      tx.oncomplete = () => resolve()
      tx.onerror = () => reject(tx.error)
    })
  }

  async replaceOps(ops) {
    await req(this.db.transaction('journal', 'readwrite').objectStore('journal').clear())
    await this.putOps(ops)
  }

  async reset() {
    this.db.close()
    await new Promise((resolve, reject) => {
      const r = this.factory.deleteDatabase(this.name)
      r.onsuccess = () => resolve()
      r.onerror = () => reject(r.error)
      r.onblocked = () => resolve()
    })
  }
}

/** IndexedDB adapter: `state` holds `{ seed, ds }`; `journal` is one wire op per id. */
export class IndexedDbJournal extends DurableJournal {
  static async open(name, opts = {}) {
    const factory = idbFactory(opts)
    if (!factory) throw new Error('IndexedDB is not available')
    const db = await openIndexedDb(factory, name)
    return new IndexedDbJournal(name, new IdbDriver(db, factory, name))
  }

  constructor(name, driver) {
    super(ADAPTER_INDEXEDDB, name, driver)
  }
}

class OpfsDriver {
  constructor(dir) {
    this.dir = dir
  }

  async #readText(handle) {
    const file = await handle.getFile()
    return file.text()
  }

  async #writeText(dir, name, contents) {
    const handle = await dir.getFileHandle(name, { create: true })
    const w = await handle.createWritable({ keepExistingData: false })
    await w.write(contents)
    await w.close()
  }

  async getIdentity() {
    let handle
    try {
      handle = await this.dir.getFileHandle('identity')
    } catch (err) {
      if (isNotFound(err)) return null
      throw err
    }
    const text = await this.#readText(handle)
    return JSON.parse(text)
  }

  async setIdentity(peer) {
    await this.#writeText(this.dir, 'identity', JSON.stringify(peer))
  }

  async listOps() {
    let handle
    try {
      handle = await this.dir.getFileHandle('journal.jsonl')
    } catch (err) {
      if (isNotFound(err)) return []
      throw err
    }
    const text = await this.#readText(handle)
    if (!text.trim()) return []
    return text.split('\n').filter(Boolean).map(line => JSON.parse(line))
  }

  async putOps(ops) {
    const existing = await this.listOps()
    const seen = new Set(existing.map(o => o.id))
    const merged = existing.concat(ops.filter(o => !seen.has(o.id)))
    const body = merged.map(o => JSON.stringify(o)).join('\n')
    await this.#writeText(this.dir, 'journal.jsonl', body ? `${body}\n` : '')
  }

  async replaceOps(ops) {
    const body = ops.map(o => JSON.stringify(o)).join('\n')
    await this.#writeText(this.dir, 'journal.jsonl', body ? `${body}\n` : '')
  }

  async reset() {
    try { await this.dir.removeEntry('identity') } catch { /* gone */ }
    try { await this.dir.removeEntry('journal.jsonl') } catch { /* gone */ }
  }
}

/**
 * OPFS adapter: directory `<name>/` with `identity` JSON and a `journal.jsonl`
 * of signed wire ops. Not sqlite-wasm / wa-sqlite (parked).
 */
export class OpfsJournal extends DurableJournal {
  static async open(name, opts = {}) {
    let root = opts.opfsRoot
    if (!root) {
      const getDirectory = opts.getDirectory ?? globalThis.navigator?.storage?.getDirectory
      if (typeof getDirectory === 'function') root = await getDirectory.call(globalThis.navigator?.storage ?? globalThis)
    }
    if (!root) throw new Error('OPFS is not available')
    const dir = await root.getDirectoryHandle(name, { create: true })
    return new OpfsJournal(name, new OpfsDriver(dir))
  }

  constructor(name, driver) {
    super(ADAPTER_OPFS, name, driver)
  }
}
