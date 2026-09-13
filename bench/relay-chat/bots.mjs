import { Worker } from 'node:worker_threads'
import { fileURLToPath } from 'node:url'
import { repo } from './relay.mjs'

const workerFile = fileURLToPath(new URL('./bot-worker.mjs', import.meta.url))

export class Bot {
  constructor(name, path, relayUrl) {
    this.name = name
    this.path = path
    this.worker = new Worker(workerFile, { workerData: { name, path, relayUrl, repo } })
    this.pending = new Map()
    this.nextId = 1
    this.ready = new Promise((resolve, reject) => {
      this.worker.once('error', reject)
      this.worker.on('message', (m) => {
        if (m.ready) return resolve()
        const p = this.pending.get(m.id)
        if (!p) return
        this.pending.delete(m.id)
        if (m.error) p.reject(new Error(`${name}: ${m.error}`))
        else p.resolve(m.result)
      })
    })
  }

  call(cmd, args) {
    const id = this.nextId++
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject })
      this.worker.postMessage({ id, cmd, args })
    })
  }

  info() {
    return this.call('info')
  }
  join(ds) {
    return this.call('join', { ds })
  }
  sync() {
    return this.call('sync')
  }
  say(text) {
    return this.call('say', { text })
  }
  seed(count, prefix) {
    return this.call('seed', { count, prefix })
  }
  opCount() {
    return this.call('opCount')
  }
  msgCount() {
    return this.call('msgCount')
  }
  chat(args) {
    return this.call('chat', args)
  }
  async close() {
    try {
      await this.call('close')
    } catch {
      /* ignore */
    }
    await this.worker.terminate()
  }
}

export async function spawnBots(count, relayUrl, pathFor) {
  const bots = []
  for (let i = 0; i < count; i += 1) {
    const name = `bot${i}`
    bots.push(new Bot(name, pathFor(name), relayUrl))
  }
  await Promise.all(bots.map((b) => b.ready))
  return bots
}
