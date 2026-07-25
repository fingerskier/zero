// IndexedDB op-journal persistence for a zerodb wasm store.
//
// Same pattern as examples/browser-peer/index.html, factored into a module:
// 'state' holds { seed, ds } (the identity seed + datastore id); 'journal'
// holds one wire op per key (op id). Restores rebuild the store from seed +
// journal replay; persists append only the ops not yet journaled, and the
// journal is compacted/rewritten if it drifts from the store's op set.

function req(r) {
  return new Promise((resolve, reject) => {
    r.onsuccess = () => resolve(r.result)
    r.onerror = () => reject(r.error)
  })
}

export class ZeroJournal {
  #idb
  #name
  #persisted = new Set()

  static async open(name) {
    const j = new ZeroJournal()
    j.#name = name
    j.#idb = await new Promise((resolve, reject) => {
      const r = indexedDB.open(name, 1)
      r.onupgradeneeded = () => {
        if (!r.result.objectStoreNames.contains('state')) r.result.createObjectStore('state')
        if (!r.result.objectStoreNames.contains('journal')) r.result.createObjectStore('journal')
      }
      r.onsuccess = () => resolve(r.result)
      r.onerror = () => reject(r.error)
    })
    return j
  }

  #get(key) {
    return req(this.#idb.transaction('state').objectStore('state').get(key))
  }

  #set(key, value) {
    return req(this.#idb.transaction('state', 'readwrite').objectStore('state').put(value, key))
  }

  #journalAll() {
    return req(this.#idb.transaction('journal').objectStore('journal').getAll())
  }

  #journalPut(ops) {
    return new Promise((resolve, reject) => {
      const tx = this.#idb.transaction('journal', 'readwrite')
      const store = tx.objectStore('journal')
      for (const op of ops) store.put(op, op.id)
      tx.oncomplete = () => resolve()
      tx.onerror = () => reject(tx.error)
    })
  }

  #journalClear() {
    return req(this.#idb.transaction('journal', 'readwrite').objectStore('journal').clear())
  }

  /**
   * Restore a store from the journal, or mint a fresh one.
   * Returns `{ db, restored, opCount }`.
   */
  async restore(ZeroDb) {
    const saved = await this.#get('peer')
    if (!saved) {
      const db = new ZeroDb()
      await this.#set('peer', { seed: db.seedHex(), ds: db.datastoreId() })
      return { db, restored: false, opCount: 0 }
    }
    const db = ZeroDb.fromSeed(saved.seed, saved.ds)
    const ops = await this.#journalAll()
    if (ops.length > 0) {
      const res = db.importJson(JSON.stringify({ format: 1, datastore_id: saved.ds, ops }))
      if (res.accepted > 0) db.replay()
    }
    // Compact/rewrite if the journal drifted from the op set (skipped dupes,
    // interrupted writes).
    if (ops.length !== db.opCount()) {
      await this.#journalClear()
      await this.#journalPut(JSON.parse(db.exportJson()).ops)
    }
    for (const id of db.opIds()) this.#persisted.add(id)
    await this.#set('peer', { seed: db.seedHex(), ds: db.datastoreId() })
    return { db, restored: true, opCount: db.opCount() }
  }

  /**
   * Append any not-yet-journaled ops; re-write identity meta when the
   * datastore id changed (first sync may adopt the peer's datastore).
   */
  async persist(db) {
    const ids = db.opIds().filter(id => !this.#persisted.has(id))
    if (ids.length > 0) {
      const ops = JSON.parse(db.exportOpsByIds(ids))
      await this.#journalPut(ops)
      for (const op of ops) this.#persisted.add(op.id)
    }
    const meta = await this.#get('peer')
    if (!meta || meta.ds !== db.datastoreId()) {
      await this.#set('peer', { seed: db.seedHex(), ds: db.datastoreId() })
    }
  }

  /** Delete the whole database (identity + journal). */
  async reset() {
    await this.#set('peer', undefined)
    this.#idb.close()
    await new Promise((resolve, reject) => {
      const r = indexedDB.deleteDatabase(this.#name)
      r.onsuccess = () => resolve()
      r.onerror = () => reject(r.error)
      r.onblocked = () => resolve()
    })
  }
}
