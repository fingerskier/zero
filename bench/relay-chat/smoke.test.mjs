// Keeps the harness runnable in CI: tiny sizes, convergence and shape
// assertions only, no timing thresholds (timing is not a CI gate).
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { runBench } from './run.mjs'

test('relay-chat harness converges and captures every metric family', async () => {
  const r = await runBench({ bots: 2, history: 40, messages: 4, pollMs: 10, gapMs: 20, reconnects: 2 })
  assert.ok(r.meta.historyOps >= 40, `history ${r.meta.historyOps}`)

  const cold = r.scenarios.cold
  assert.ok(cold.received >= 40, `cold received ${cold.received}`)
  assert.ok(cold.merkleLeaves >= 1)
  assert.ok(cold.wire.bytesToRelay > 0 && cold.wire.bytesFromRelay > 0)

  const rc = r.scenarios.reconnect
  for (const b of rc.perBot) {
    // P0-3 as measured: an equal replica still uploads its full history.
    assert.ok(b.sent >= 40, `${b.bot} sent ${b.sent} per reconnect`)
    assert.equal(b.ackAccepted, 0)
    assert.equal(b.ackDuplicate, b.sent)
  }
  assert.equal(rc.syncRequests, 2 * 2)
  // P0-4 as measured: at least one full Merkle build per SYNC_REQUEST.
  assert.ok(rc.relayMerkleBuilds >= rc.syncRequests, `builds ${rc.relayMerkleBuilds} < ${rc.syncRequests}`)

  const d = r.scenarios.delta
  assert.equal(d.writer.ackAccepted, 2)
  assert.equal(d.readerReceived, 2)

  const sp = r.scenarios.sparse
  assert.ok(sp.converged, `sparse msgCounts ${sp.msgCounts.join(',')}`)

  const c = r.scenarios.chat
  assert.ok(c.complete, `chat deliveries ${c.deliveries}/${c.expectedDeliveries}`)
  assert.equal(c.deliveries, c.expectedDeliveries)
  assert.ok(c.latency.p50 > 0)

  assert.ok(r.relay.stats && r.relay.stats.merkle_builds > 0, 'relay stats line parsed')
  if (process.platform === 'linux') assert.ok(r.relay.proc.rssKb > 0)
})
