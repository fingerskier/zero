export function percentile(sorted, p) {
  if (sorted.length === 0) return null
  const idx = Math.min(sorted.length - 1, Math.max(0, Math.ceil((p / 100) * sorted.length) - 1))
  return sorted[idx]
}

export function summarize(values) {
  const xs = values.filter((v) => Number.isFinite(v)).sort((a, b) => a - b)
  if (xs.length === 0) return { n: 0 }
  const sum = xs.reduce((a, b) => a + b, 0)
  return {
    n: xs.length,
    min: xs[0],
    p50: percentile(xs, 50),
    p95: percentile(xs, 95),
    p99: percentile(xs, 99),
    max: xs[xs.length - 1],
    mean: sum / xs.length,
  }
}

const fmt = (v) => (v == null ? '-' : typeof v === 'number' ? (Number.isInteger(v) ? String(v) : v.toFixed(1)) : String(v))

export function table(headers, rows) {
  const line = (cells) => `| ${cells.map(fmt).join(' | ')} |`
  return [line(headers), `|${headers.map(() => '---').join('|')}|`, ...rows.map(line)].join('\n')
}

export function latencyRow(label, s) {
  return [label, s.n, s.min, s.p50, s.p95, s.p99, s.max]
}

export function toMarkdown(result) {
  const { meta, scenarios, relay } = result
  const out = []
  out.push(`# relay-chat baseline — ${meta.date}`)
  out.push('')
  out.push(
    `bots=${meta.bots} history=${meta.history} messages=${meta.messages} pollMs=${meta.pollMs} relay=${meta.relayBuild} node=${meta.node} git=${meta.git}`,
  )
  out.push('')
  out.push('**Not published numbers.** One machine, loopback, debug/release as stated; see README for what is and is not measured.')
  out.push('')
  if (scenarios.cold) {
    const c = scenarios.cold
    out.push('## Cold join (fresh bot joins the seeded datastore)')
    out.push('')
    out.push(
      table(
        ['history ops', 'ms', 'received', 'merkleLeaves', 'bytes→relay', 'bytes←relay', 'relay merkle builds'],
        [[c.historyOps, c.ms, c.received, c.merkleLeaves, c.wire.bytesToRelay, c.wire.bytesFromRelay, c.relayMerkleBuilds]],
      ),
    )
    out.push('')
  }
  if (scenarios.reconnect) {
    const r = scenarios.reconnect
    out.push('## Equal-replica reconnect (P0-3 / P0-4)')
    out.push('')
    out.push(
      table(
        ['bot', 'reconnects', 'sent/reconnect', 'ackDuplicate', 'ackAccepted', 'merkleNodes', 'merkleLeaves', 'ms p50', 'ms p95'],
        r.perBot.map((b) => [b.bot, b.n, b.sent, b.ackDuplicate, b.ackAccepted, b.merkleNodes, b.merkleLeaves, b.ms.p50, b.ms.p95]),
      ),
    )
    out.push('')
    out.push(
      `wire per reconnect: ${r.wirePerReconnect.bytesToRelay.toFixed(0)} bytes→relay, ${r.wirePerReconnect.bytesFromRelay.toFixed(0)} bytes←relay; relay: merkle builds ${r.relayMerkleBuilds} for ${r.syncRequests} SYNC_REQUESTs; ops_duplicate ${r.relayOpsDuplicate}`,
    )
    out.push('')
  }
  if (scenarios.delta) {
    const d = scenarios.delta
    out.push('## One-op delta (one bot writes, the others sync once)')
    out.push('')
    out.push(
      table(
        ['writer sent', 'writer accepted', 'reader received', 'reader ms p50', 'reader ms p95', 'bytes→relay total', 'bytes←relay total', 'relay merkle builds'],
        [[d.writer.sent, d.writer.ackAccepted, d.readerReceived, d.readerMs.p50, d.readerMs.p95, d.wire.bytesToRelay, d.wire.bytesFromRelay, d.relayMerkleBuilds]],
      ),
    )
    out.push('')
  }
  if (scenarios.sparse) {
    const s = scenarios.sparse
    out.push('## Sparse divergence (every bot writes offline, then all sync twice)')
    out.push('')
    out.push(
      table(
        ['ops per bot', 'pass', 'sent total', 'ackAccepted', 'ackDuplicate', 'received total', 'ms p50', 'ms p95', 'bytes→relay', 'bytes←relay'],
        s.passes.map((p) => [s.opsPerBot, p.pass, p.sent, p.ackAccepted, p.ackDuplicate, p.received, p.ms.p50, p.ms.p95, p.wire.bytesToRelay, p.wire.bytesFromRelay]),
      ),
    )
    out.push('')
  }
  if (scenarios.chat) {
    const c = scenarios.chat
    out.push('## Chat (all bots send on a schedule while polling)')
    out.push('')
    out.push(`messages ${c.messages}, deliveries ${c.deliveries}/${c.expectedDeliveries}${c.complete ? '' : ' (INCOMPLETE)'}, elapsed ${c.elapsedMs.toFixed(0)} ms`)
    out.push('')
    out.push(
      table(
        ['metric', 'n', 'min', 'p50', 'p95', 'p99', 'max'],
        [
          latencyRow('delivery latency ms', c.latency),
          latencyRow('sync ms', c.syncMs),
          latencyRow('sent per sync (full upload)', c.sentPerSync),
          latencyRow('listNodes scan ms', c.scanMs),
        ],
      ),
    )
    out.push('')
    out.push(`wire: ${c.wire.connections} connections, ${c.wire.bytesToRelay} bytes→relay, ${c.wire.bytesFromRelay} bytes←relay; relay merkle builds ${c.relayMerkleBuilds}, sync requests ${c.syncRequests}`)
    out.push('')
  }
  if (relay) {
    out.push('## Relay process')
    out.push('')
    out.push('```json')
    out.push(JSON.stringify(relay, null, 2))
    out.push('```')
  }
  return out.join('\n') + '\n'
}
