/**
 * ZeroDB back&forth demo server.
 *
 * Hosts two independent LocalStores ("a" and "b") via @zerodb/node, serves a
 * two-pane UI, streams subscribe events over SSE, and syncs the peers by
 * exchanging export bundles both directions.
 *
 * Run: node server.mjs   (http://localhost:8787)
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

function openOrInit(path) {
  try {
    return Database.open(path)
  } catch {
    return Database.init(path)
  }
}

const peers = {
  a: openOrInit(join(dataDir, 'peer-a.sqlite')),
  b: openOrInit(join(dataDir, 'peer-b.sqlite')),
}

// SSE clients per peer
const clients = { a: new Set(), b: new Set() }

for (const [name, db] of Object.entries(peers)) {
  db.subscribe((event) => {
    const payload = `data: ${JSON.stringify(event)}\n\n`
    for (const res of clients[name]) res.write(payload)
  })
}

function json(res, code, body) {
  const data = JSON.stringify(body)
  res.writeHead(code, { 'content-type': 'application/json' })
  res.end(data)
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

function peerState(name) {
  const db = peers[name]
  const report = db.inspect(join(dataDir, `peer-${name}.sqlite`))
  return {
    peerId: db.peerId(),
    datastoreId: db.datastoreId(),
    ops: db.opCount(),
    nodes: report.nodes,
  }
}

function sync() {
  // a→b first; export b AFTER, so an empty b that just adopted a's datastore
  // id (empty-peer bootstrap rule) sends back a compatible bundle.
  const aToB = peers.b.importJson(peers.a.exportJson())
  const bToA = peers.a.importJson(peers.b.exportJson())
  return { aToB, bToA }
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

    // GET /events/:peer — SSE stream
    if (req.method === 'GET' && parts[0] === 'events' && peers[parts[1]]) {
      res.writeHead(200, {
        'content-type': 'text/event-stream',
        'cache-control': 'no-cache',
        connection: 'keep-alive',
      })
      res.write(': connected\n\n')
      clients[parts[1]].add(res)
      req.on('close', () => clients[parts[1]].delete(res))
      return
    }

    // GET /api/:peer/state
    if (req.method === 'GET' && parts[0] === 'api' && peers[parts[1]] && parts[2] === 'state') {
      json(res, 200, peerState(parts[1]))
      return
    }

    // POST /api/sync
    if (req.method === 'POST' && parts[0] === 'api' && parts[1] === 'sync') {
      json(res, 200, sync())
      return
    }

    // POST /api/:peer/<create|set|delete>
    if (req.method === 'POST' && parts[0] === 'api' && peers[parts[1]]) {
      const db = peers[parts[1]]
      const body = await readBody(req)
      if (parts[2] === 'create') {
        json(res, 200, { node: db.createNode(body.label || 'Item') })
        return
      }
      if (parts[2] === 'set') {
        json(res, 200, { op: db.setLww(body.node, body.key, String(body.value)) })
        return
      }
      if (parts[2] === 'delete') {
        json(res, 200, { op: db.deleteNode(body.node) })
        return
      }
    }

    json(res, 404, { error: 'not found' })
  } catch (e) {
    json(res, 400, { error: String(e.message || e) })
  }
})

const port = process.env.PORT || 8787
server.listen(port, '127.0.0.1', () => {
  console.log(`zerodb webapp: http://localhost:${port}`)
  console.log(`peer a: ${peers.a.peerId().slice(0, 12)}…  peer b: ${peers.b.peerId().slice(0, 12)}…`)
})
