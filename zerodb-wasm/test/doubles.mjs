// Test doubles for durable adapters: in-memory IndexedDB subset and OPFS
// directory handles. Enough for persist/reopen without a browser.

class IdRequest {
  result = undefined
  error = undefined
  onsuccess = null
  onerror = null
  onupgradeneeded = null
  onblocked = null
}

class IdStore {
  constructor() {
    this.map = new Map()
  }

  get(key) {
    const r = new IdRequest()
    queueMicrotask(() => {
      r.result = this.map.get(key)
      if (r.onsuccess) r.onsuccess({ target: r })
    })
    return r
  }

  put(value, key) {
    const r = new IdRequest()
    if (value === undefined) this.map.delete(key)
    else this.map.set(key, structuredClone(value))
    queueMicrotask(() => {
      r.result = key
      if (r.onsuccess) r.onsuccess({ target: r })
    })
    return r
  }

  getAll() {
    const r = new IdRequest()
    const values = [...this.map.values()].map(v => structuredClone(v))
    queueMicrotask(() => {
      r.result = values
      if (r.onsuccess) r.onsuccess({ target: r })
    })
    return r
  }

  clear() {
    const r = new IdRequest()
    this.map.clear()
    queueMicrotask(() => {
      r.result = undefined
      if (r.onsuccess) r.onsuccess({ target: r })
    })
    return r
  }
}

class IdTx {
  constructor(db) {
    this.db = db
    this.oncomplete = null
    this.onerror = null
    this.error = undefined
    // After the constructor's synchronous puts (same turn).
    queueMicrotask(() => {
      queueMicrotask(() => {
        if (this.oncomplete) this.oncomplete()
      })
    })
  }

  objectStore(name) {
    return this.db.stores.get(name)
  }
}

class IdDb {
  constructor(name) {
    this.name = name
    this.version = 0
    this.stores = new Map()
    this.objectStoreNames = {
      contains: n => this.stores.has(n),
    }
  }

  createObjectStore(name) {
    const s = new IdStore()
    this.stores.set(name, s)
    return s
  }

  transaction(_name, _mode = 'readonly') {
    return new IdTx(this)
  }

  close() {}
}

/** In-memory `indexedDB` factory (open / deleteDatabase). */
export function fakeIndexedDB() {
  const dbs = new Map()
  return {
    open(name, version = 1) {
      const r = new IdRequest()
      queueMicrotask(() => {
        let db = dbs.get(name)
        const isNew = !db
        if (!db) {
          db = new IdDb(name)
          dbs.set(name, db)
        }
        r.result = db
        if (isNew || db.version < version) {
          if (r.onupgradeneeded) r.onupgradeneeded({ target: r })
          db.version = version
        }
        if (r.onsuccess) r.onsuccess({ target: r })
      })
      return r
    },
    deleteDatabase(name) {
      const r = new IdRequest()
      dbs.delete(name)
      queueMicrotask(() => {
        r.result = undefined
        if (r.onsuccess) r.onsuccess({ target: r })
      })
      return r
    },
  }
}

class MemoryFile {
  kind = 'file'
  bytes = new Uint8Array()

  async getFile() {
    const bytes = this.bytes
    return {
      text: async () => new TextDecoder().decode(bytes),
      arrayBuffer: async () => bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength),
    }
  }

  async createWritable({ keepExistingData } = {}) {
    const file = this
    let next = keepExistingData === false ? new Uint8Array() : file.bytes
    return {
      async write(data) {
        if (typeof data === 'string') next = new TextEncoder().encode(data)
        else if (data instanceof Uint8Array) next = data
        else if (data instanceof ArrayBuffer) next = new Uint8Array(data)
        else throw new Error('unsupported write payload')
      },
      async close() {
        file.bytes = next
      },
    }
  }
}

class MemoryDirectory {
  kind = 'directory'
  #files = new Map()
  #dirs = new Map()

  async getDirectoryHandle(name, { create } = {}) {
    if (this.#dirs.has(name)) return this.#dirs.get(name)
    if (!create) {
      const err = new Error(`directory not found: ${name}`)
      err.name = 'NotFoundError'
      throw err
    }
    const d = new MemoryDirectory()
    this.#dirs.set(name, d)
    return d
  }

  async getFileHandle(name, { create } = {}) {
    if (this.#files.has(name)) return this.#files.get(name)
    if (!create) {
      const err = new Error(`file not found: ${name}`)
      err.name = 'NotFoundError'
      throw err
    }
    const f = new MemoryFile()
    this.#files.set(name, f)
    return f
  }

  async removeEntry(name) {
    this.#files.delete(name)
    this.#dirs.delete(name)
  }
}

/** In-memory OPFS root (`getDirectoryHandle` / file handles). */
export function memoryOpfsRoot() {
  return new MemoryDirectory()
}
