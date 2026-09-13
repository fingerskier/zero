// One bot = one worker thread = one NAPI LocalStore. Every command is a
// synchronous NAPI call, so each bot needs its own thread for bots to sync
// concurrently and for the main-thread proxy to keep forwarding.
import { parentPort, workerData } from 'node:worker_threads'
import { createRequire } from 'node:module'
import { join } from 'node:path'

const require = createRequire(import.meta.url)
const { Database } = require(join(workerData.repo, 'zerodb-napi/index.js'))

const nowMs = () => Number(process.hrtime.bigint()) / 1e6
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))

const db = Database.init(workerData.path)
const name = workerData.name
const relayUrl = workerData.relayUrl
let joinDs = null

function timedSync() {
  const t0 = nowMs()
  const s = db.connectRelay(relayUrl, joinDs ?? undefined)
  const ms = nowMs() - t0
  if (!joinDs) joinDs = db.datastoreId()
  return { ...s, ms, at: t0 }
}

function say(text) {
  const node = db.createNode('Msg')
  db.setLww(node, 'text', text)
  return { node, text, sentAt: nowMs() }
}

function msgNodes() {
  const out = new Map()
  for (const n of db.listNodes()) {
    if (n.label === 'Msg' && n.props && typeof n.props.text === 'string') out.set(n.id, n.props.text)
  }
  return out
}

const handlers = {
  info: () => ({ name, peerId: db.peerId(), datastoreId: db.datastoreId(), opCount: db.opCount() }),
  join: ({ ds }) => {
    joinDs = ds
    return timedSync()
  },
  sync: () => timedSync(),
  say: ({ text }) => say(text),
  seed: ({ count, prefix }) => {
    const t0 = nowMs()
    let last = null
    for (let i = 0; i < count; i += 1) last = say(`${prefix}:${i}`)
    return { count, ms: nowMs() - t0, last }
  },
  opCount: () => db.opCount(),
  msgCount: () => msgNodes().size,
  /**
   * Chat round: send `sends` (each {at: relative ms, text}) on schedule while
   * polling the relay every `pollMs`; record when each foreign Msg node is
   * first visible. Stops once `expectTotal` messages are seen and all sends
   * are done, or at `deadlineMs`.
   */
  chat: async ({ sends, pollMs, expectTotal, deadlineMs }) => {
    const known = msgNodes()
    const sent = []
    const seen = []
    const syncs = []
    const scans = []
    const start = nowMs()
    let next = 0
    while (true) {
      const rel = nowMs() - start
      while (next < sends.length && sends[next].at <= rel) {
        sent.push(say(sends[next].text))
        next += 1
      }
      const s = timedSync()
      syncs.push(s)
      if (s.applied > 0) {
        const t0 = nowMs()
        const nodes = msgNodes()
        const t1 = nowMs()
        scans.push(t1 - t0)
        for (const [id, text] of nodes) {
          if (known.has(id)) continue
          known.set(id, text)
          if (!sent.some((m) => m.node === id)) seen.push({ node: id, text, seenAt: t1 })
        }
      } else {
        for (const m of sent) if (!known.has(m.node)) known.set(m.node, m.text)
      }
      const done = next >= sends.length && seen.length >= expectTotal
      if (done || nowMs() - start > deadlineMs) break
      if (pollMs > 0) await sleep(pollMs)
    }
    return { sent, seen, syncs, scans, elapsedMs: nowMs() - start, complete: seen.length >= expectTotal }
  },
  close: () => {
    db.close()
    return true
  },
}

parentPort.on('message', async ({ id, cmd, args }) => {
  try {
    const result = await handlers[cmd](args || {})
    parentPort.postMessage({ id, result })
  } catch (e) {
    parentPort.postMessage({ id, error: String(e && e.stack ? e.stack : e) })
  }
})
parentPort.postMessage({ ready: true })
