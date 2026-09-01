#!/usr/bin/env node
// Independent TypeScript RELAY 0.2 peer process.
// Pure encoder/decoder + semantic store; NOT the SDK, never NAPI-backed.

import { PeerStore } from './store.mjs'
import { sync } from './client.mjs'
import { connectRelay } from './ws.mjs'

function usage() {
  return `Usage:
  node conformance/ts/peer/cli.mjs --url ws://127.0.0.1:PORT [options]

Options:
  --join <datastore-hex>   catch up that datastore (empty store adopts it)
  --schema                 write signed SchemaEpoch n=1 / prev=null / empty migration
  --create <label>         CreateNode (default label Todo when --schema or --set)
  --set <path=value>       SetProperty LWW (requires a node from --create)
  --print-ds               print datastore id after sync
`
}

function argValue(argv, name) {
  const i = argv.indexOf(name)
  if (i < 0 || i + 1 >= argv.length) return null
  return argv[i + 1]
}

function hasFlag(argv, name) {
  return argv.includes(name)
}

export async function runCli(argv, io = {}) {
  const log = io.log || console.log
  if (hasFlag(argv, '--help') || hasFlag(argv, '-h')) {
    log(usage())
    return { ok: true }
  }
  const url = argValue(argv, '--url')
  if (!url) {
    throw new Error(usage())
  }
  const join = argValue(argv, '--join')
  const store = new PeerStore()
  let node = null
  if (hasFlag(argv, '--schema')) {
    store.applySchemaEpoch({ nodes: { Todo: { props: { title: 'lww' } } } })
  }
  const create = argValue(argv, '--create')
  if (create || hasFlag(argv, '--schema') || argValue(argv, '--set')) {
    if (!join) {
      const made = store.createNode(create || 'Todo')
      node = made.node
    }
  }
  const set = argValue(argv, '--set')
  if (set && node) {
    const eq = set.indexOf('=')
    if (eq < 0) throw new Error('--set path=value')
    store.setLww(node, set.slice(0, eq), set.slice(eq + 1))
  }

  const transport = await connectRelay(url)
  try {
    const summary = await sync(store, join, (frame) => transport.request(frame))
    const out = {
      datastore: store.datastoreIdHex(),
      node,
      title: node ? store.getLww(node, 'title') : null,
      schema_epoch: store.schemaEpoch,
      ...summary,
    }
    log(JSON.stringify(out, null, 2))
    return out
  } finally {
    transport.close()
  }
}

const isMain = process.argv[1] && process.argv[1].endsWith('cli.mjs')
if (isMain) {
  runCli(process.argv.slice(2)).catch((err) => {
    console.error(err.message || err)
    process.exit(1)
  })
}
