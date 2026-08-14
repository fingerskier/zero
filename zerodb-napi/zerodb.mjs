/**
 * Thin promise/typed facade over the sync NAPI `Database`.
 * NAPI stays the internal binding; this is the public-ish SDK surface.
 */
import { createRequire } from 'node:module'

const require = createRequire(import.meta.url)
const { Database } = require('./index.js')

export class ZeroDB {
  constructor(db) {
    this.db = db
  }

  static async open({ path, create = false } = {}) {
    if (!path) throw new Error('ZeroDB.open requires { path }')
    const db = create ? Database.init(path) : Database.open(path)
    return new ZeroDB(db)
  }

  async applySchema(schemaJson) {
    const raw = typeof schemaJson === 'string' ? schemaJson : JSON.stringify(schemaJson)
    return this.db.applySchema(raw)
  }

  async create(label, props = {}) {
    const id = this.db.createNode(label)
    for (const [key, value] of Object.entries(props)) {
      if (typeof value === 'string') this.db.setLww(id, key, value)
      else if (typeof value === 'boolean' && value) this.db.flagEnable(id, key)
      else if (typeof value === 'boolean') this.db.flagDisable(id, key)
      else if (typeof value === 'number') this.db.setLww(id, key, String(value))
    }
    return id
  }

  async mutate(node, patch = {}) {
    for (const [key, value] of Object.entries(patch)) {
      if (typeof value === 'string') this.db.setLww(node, key, value)
      else if (typeof value === 'boolean' && value) this.db.flagEnable(node, key)
      else if (typeof value === 'boolean') this.db.flagDisable(node, key)
    }
  }

  async query(q, params) {
    return this.db.query(q, params ?? undefined)
  }

  async close() {
    this.db.close()
  }
}

export { Database }
