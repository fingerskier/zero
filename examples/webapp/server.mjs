/**
 * ZeroDB peer server — one LocalStore per process.
 *
 * Serves a single-pane UI over its own store, exposes a WebSocket sync
 * listener (db.serve), and — when ZERO_PEER is set — keeps itself converged
 * with that peer via db.autoConnect (dirty-flag push + interval poll).
 *
 * Run two instances and watch them converge:
 *   PORT=8787 WS_PORT=9787 node server.mjs
 *   PORT=8788 WS_PORT=9788 ZERO_PEER=ws://127.0.0.1:9787 node server.mjs
 */
import { createServer } from 'node:http'
import { readFileSync, mkdirSync } from 'node:fs'
import { join, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'
import { createRequire } from 'node:module'

const require = createRequire(import.meta.url)
const { Database } = require('../../zerodb-napi/index.js')

const root = dirname(fileURLToPath(import.meta.url))
const dataDir = join(root, 'data')
mkdirSync(dataDir, { recursive: true })

const port = Number(process.env.PORT || 8787)
const wsPort = Number(process.env.WS_PORT || port + 1000)
const peerUrl = process.env.ZERO_PEER || null
const dbPath = process.env.ZERO_DATA || join(dataDir, `peer-${port}.sqlite`)

function openOrInit(path) {
  try {
    return Database.open(path)
  } catch {
    return Database.init(path)
  }
}

const db = openOrInit(dbPath)
// ZERO_LAN=1 binds the sync listener on 0.0.0.0 — plaintext/unauthenticated,
// trusted private LAN only.
const actualWsPort = db.serve(wsPort, process.env.ZERO_LAN === '1')
if (peerUrl) db.autoConnect(peerUrl, 1000)

// SSE clients
const clients = new Set()
let lastSync = null

db.subscribe((event) => {
  if (event.kind === 'sync' || event.kind === 'sync-error') lastSync = event
  const payload = `data: ${JSON.stringify(event)}\n\n`
  for (const res of clients) res.write(payload)
})

function json(res, code, body) {
  res.writeHead(code, { 'content-type': 'application/json' })
  res.end(JSON.stringify(body))
}

function readBody(req) {
  return new Promise((resolve, reject) => {
    let buf = ''
    req.on('data', (c) => (buf += c))
    req.on('end', () => {
      try {
        resolve(buf ? JSON.parse(buf) : {})
      } catch (e) {
        reject(e)
      }
    })
    req.on('error', reject)
  })
}

function state() {
  const report = db.inspect(dbPath)
  return {
    peerId: db.peerId(),
    datastoreId: db.datastoreId(),
    ops: db.opCount(),
    nodes: report.nodes,
    wsPort: actualWsPort,
    peerUrl,
    lastSync,
  }
}

const server = createServer(async (req, res) => {
  const url = new URL(req.url, 'http://localhost')
  const parts = url.pathname.split('/').filter(Boolean)

  try {
    if (req.method === 'GET' && url.pathname === '/') {
      res.writeHead(200, { 'content-type': 'text/html; charset=utf-8' })
      res.end(readFileSync(join(root, 'index.html')))
      return
    }

    // GET /events — SSE stream
    if (req.method === 'GET' && url.pathname === '/events') {
      res.writeHead(200, {
        'content-type': 'text/event-stream',
        'cache-control': 'no-cache',
        connection: 'keep-alive',
      })
      res.write(': connected\n\n')
      clients.add(res)
      req.on('close', () => clients.delete(res))
      return
    }

    // GET /api/state
    if (req.method === 'GET' && url.pathname === '/api/state') {
      json(res, 200, state())
      return
    }

    // POST /api/<create|set|delete>
    if (req.method === 'POST' && parts[0] === 'api') {
      const body = await readBody(req)
      if (parts[1] === 'create') {
        json(res, 200, { node: db.createNode(body.label || 'Item') })
        return
      }
      if (parts[1] === 'set') {
        json(res, 200, { op: db.setLww(body.node, body.key, String(body.value)) })
        return
      }
      if (parts[1] === 'delete') {
        json(res, 200, { op: db.deleteNode(body.node) })
        return
      }
    }

    json(res, 404, { error: 'not found' })
  } catch (e) {
    json(res, 400, { error: String(e.message || e) })
  }
})

server.listen(port, '127.0.0.1', () => {
  console.log(`zerodb peer: http://localhost:${port}  sync: ws://127.0.0.1:${actualWsPort}`)
  console.log(`store: ${dbPath}`)
  console.log(`peer id: ${db.peerId().slice(0, 12)}…${peerUrl ? `  auto-sync → ${peerUrl}` : ''}`)
})
