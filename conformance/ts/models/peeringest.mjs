// Peer-side ingest model for M3c-c named rejects + SchemaEpoch n=1.
// Reuses the independent TS PeerStore (never NAPI-backed).

import { PeerStore, epochFirst } from '../peer/store.mjs'
import { hexToBytes } from './cbor.mjs'
import { peerRejects } from './registry.mjs'

const KNOWN = new Set(peerRejects.reasons)

function storeFrom(v) {
  const seed = v.seed ? hexToBytes(v.seed) : undefined
  const salt = v.salt ? hexToBytes(v.salt) : undefined
  const wall = v.clock ?? 1_700_000_000_000
  const store = new PeerStore({
    seed,
    salt,
    clock: () => wall,
  })
  if (v.datastore) store.adoptDatastore(v.datastore)
  return store
}

export function runPeerIngestVector(v) {
  const store = storeFrom(v)
  const setup = epochFirst(v.setup || [])
  for (const w of setup) {
    const r = store.ingest(w, { expectedDs: v.expected_ds || store.dsHex })
    if (r !== 'applied' && r !== 'duplicate') {
      throw new Error(`setup ingest ${w.id}: ${r}`)
    }
  }

  const ingest = v.ingest || []
  const got = []
  for (const w of ingest) {
    const r = store.ingest(w, { expectedDs: v.expected_ds || store.dsHex })
    got.push({ op_id: w.id, result: r })
  }

  const expect = v.expect || {}
  if (expect.schema_epoch != null && store.schemaEpoch !== expect.schema_epoch) {
    throw new Error(`schemaEpoch ${store.schemaEpoch}, expected ${expect.schema_epoch}`)
  }
  if (expect.applied) {
    const applied = got.filter((g) => g.result === 'applied').map((g) => g.op_id)
    if (JSON.stringify(applied) !== JSON.stringify(expect.applied)) {
      throw new Error(`applied ${JSON.stringify(applied)}, expected ${JSON.stringify(expect.applied)}`)
    }
  }
  if (expect.rejects) {
    const rejects = got
      .filter((g) => g.result !== 'applied' && g.result !== 'duplicate')
      .map((g) => ({ op_id: g.op_id, reason: g.result }))
    if (JSON.stringify(rejects) !== JSON.stringify(expect.rejects)) {
      throw new Error(`rejects ${JSON.stringify(rejects)}, expected ${JSON.stringify(expect.rejects)}`)
    }
    for (const r of rejects) {
      if (!KNOWN.has(r.reason)) throw new Error(`unknown peer reject ${r.reason}`)
    }
  }
  if (expect.lww) {
    for (const row of expect.lww) {
      const gotVal = store.getLww(row.node, row.path)
      if (gotVal !== row.value) {
        throw new Error(`lww ${row.node}/${row.path} ${gotVal}, expected ${row.value}`)
      }
    }
  }
}
