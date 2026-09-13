#!/usr/bin/env node
// relay-chat: N NAPI bot peers exchanging messages through a live
// `zerodb-relay` on loopback, with wire bytes (proxy), relay counters
// (--stats-interval-secs) and end-to-end delivery latency captured.
//
//   node bench/relay-chat/run.mjs --bots 4 --history 1000 --messages 50
//
// Numbers are a baseline for the PERF.md P0 findings, not published figures.
import { parseArgs } from 'node:util'
import { mkdirSync, rmSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { hostname } from 'node:os'
import { execSync } from 'node:child_process'
import { repo, ensureRelayBuilt, startRelay } from './relay.mjs'
import { startProxy } from './proxy.mjs'
import { spawnBots } from './bots.mjs'
import { summarize, toMarkdown } from './report.mjs'

const ALL = ['cold', 'reconnect', 'delta', 'sparse', 'chat']

function gitRev() {
  try {
    return execSync('git rev-parse --short HEAD', { cwd: repo, encoding: 'utf8' }).trim()
  } catch {
    return 'unknown'
  }
}

function sumField(list, f) {
  return list.reduce((n, x) => n + (x[f] || 0), 0)
}

export async function runBench(opts = {}) {
  const o = {
    bots: 2,
    history: 1000,
    messages: 20,
    pollMs: 50,
    gapMs: 100,
    reconnects: 5,
    scenarios: ALL,
    release: false,
    statsIntervalSecs: 1,
    keep: false,
    log: () => {},
    ...opts,
  }
  if (o.scenarios.includes('all')) o.scenarios = ALL
  const wants = (s) => o.scenarios.includes(s)
  const stamp = new Date().toISOString().replace(/[:.]/g, '-')
  const work = join(repo, 'target', 'bench-relay-chat', stamp)
  mkdirSync(work, { recursive: true })
  const pathFor = (name) => join(work, `${name}.sqlite`)

  ensureRelayBuilt({ release: o.release })
  const relay = await startRelay({ dbPath: join(work, 'relay.sqlite'), release: o.release, statsIntervalSecs: o.statsIntervalSecs })
  const proxy = await startProxy(relay.port)
  const bots = await spawnBots(o.bots, proxy.url, pathFor)
  const statsMark = async () => {
    await relay.waitTick()
    return relay.stats() || {}
  }
  const statsDelta = (a, b, k) => (b && a ? (b[k] || 0) - (a[k] || 0) : null)
  const result = {
    meta: {
      date: new Date().toISOString(),
      host: hostname(),
      node: process.version,
      git: gitRev(),
      relayBuild: o.release ? 'release' : 'debug',
      bots: o.bots,
      history: o.history,
      messages: o.messages,
      pollMs: o.pollMs,
      gapMs: o.gapMs,
      reconnects: o.reconnects,
    },
    scenarios: {},
    relay: null,
  }
  try {
    // Seed history on bot0 and push it once; a "message" is two ops.
    const seedMsgs = Math.ceil(o.history / 2)
    o.log(`seeding ${seedMsgs} messages (${seedMsgs * 2} ops) on bot0`)
    const seeded = await bots[0].seed(seedMsgs, 'seed')
    const first = await bots[0].sync()
    const ds = (await bots[0].info()).datastoreId
    result.meta.seed = { messages: seedMsgs, localMs: seeded.ms, uploadMs: first.ms, sent: first.sent, ackAccepted: first.ackAccepted }
    const historyOps = await bots[0].opCount()
    result.meta.historyOps = historyOps

    // Cold join: the first other bot joins with a fully seeded relay.
    if (bots.length > 1) {
      const m = proxy.snapshot()
      const s0 = await statsMark()
      const j = await bots[1].join(ds)
      const s1 = await statsMark()
      if (wants('cold')) {
        result.scenarios.cold = {
          historyOps,
          ms: j.ms,
          received: j.received,
          applied: j.applied,
          merkleNodes: j.merkleNodes,
          merkleLeaves: j.merkleLeaves,
          wire: proxy.since(m),
          relayMerkleBuilds: statsDelta(s0, s1, 'merkle_builds'),
        }
        o.log(`cold join: ${j.ms.toFixed(0)} ms, received ${j.received}`)
      }
      await Promise.all(bots.slice(2).map((b) => b.join(ds)))
    }

    if (wants('reconnect')) {
      const m = proxy.snapshot()
      const s0 = await statsMark()
      const rounds = []
      for (let k = 0; k < o.reconnects; k += 1) rounds.push(await Promise.all(bots.map((b) => b.sync())))
      const s1 = await statsMark()
      const wire = proxy.since(m)
      const n = o.reconnects * bots.length
      result.scenarios.reconnect = {
        reconnects: o.reconnects,
        perBot: bots.map((b, i) => {
          const mine = rounds.map((r) => r[i])
          return {
            bot: b.name,
            n: mine.length,
            sent: sumField(mine, 'sent') / mine.length,
            ackDuplicate: sumField(mine, 'ackDuplicate') / mine.length,
            ackAccepted: sumField(mine, 'ackAccepted') / mine.length,
            merkleNodes: sumField(mine, 'merkleNodes') / mine.length,
            merkleLeaves: sumField(mine, 'merkleLeaves') / mine.length,
            ms: summarize(mine.map((x) => x.ms)),
          }
        }),
        wire,
        wirePerReconnect: { bytesToRelay: wire.bytesToRelay / n, bytesFromRelay: wire.bytesFromRelay / n },
        syncRequests: statsDelta(s0, s1, 'sync_requests'),
        relayMerkleBuilds: statsDelta(s0, s1, 'merkle_builds'),
        relayOpsDuplicate: statsDelta(s0, s1, 'ops_duplicate'),
      }
      o.log(`reconnect: ${n} sessions, ${wire.bytesToRelay} bytes→relay`)
    }

    if (wants('delta') && bots.length > 1) {
      const m = proxy.snapshot()
      const s0 = await statsMark()
      await bots[0].say('delta:1')
      const w = await bots[0].sync()
      const readers = await Promise.all(bots.slice(1).map((b) => b.sync()))
      const s1 = await statsMark()
      result.scenarios.delta = {
        writer: { sent: w.sent, ackAccepted: w.ackAccepted, ackDuplicate: w.ackDuplicate, ms: w.ms },
        readerReceived: sumField(readers, 'received') / readers.length,
        readerMs: summarize(readers.map((r) => r.ms)),
        wire: proxy.since(m),
        relayMerkleBuilds: statsDelta(s0, s1, 'merkle_builds'),
      }
      o.log(`delta: readers received ${result.scenarios.delta.readerReceived} ops each`)
    }

    if (wants('sparse')) {
      const perBot = Math.max(1, Math.ceil(o.messages / 4))
      await Promise.all(bots.map((b, i) => b.seed(perBot, `sparse${i}`)))
      const s0 = await statsMark()
      const passes = []
      for (let pass = 1; pass <= 2; pass += 1) {
        const m = proxy.snapshot()
        const rs = await Promise.all(bots.map((b) => b.sync()))
        passes.push({
          pass,
          sent: sumField(rs, 'sent'),
          ackAccepted: sumField(rs, 'ackAccepted'),
          ackDuplicate: sumField(rs, 'ackDuplicate'),
          received: sumField(rs, 'received'),
          ms: summarize(rs.map((r) => r.ms)),
          wire: proxy.since(m),
        })
      }
      const s1 = await statsMark()
      const counts = await Promise.all(bots.map((b) => b.msgCount()))
      result.scenarios.sparse = {
        opsPerBot: perBot * 2,
        passes,
        converged: counts.every((c) => c === counts[0]),
        msgCounts: counts,
        relayMerkleBuilds: statsDelta(s0, s1, 'merkle_builds'),
      }
      o.log(`sparse: converged=${result.scenarios.sparse.converged} (${counts.join(',')})`)
    }

    if (wants('chat') && bots.length > 1) {
      const m = proxy.snapshot()
      const s0 = await statsMark()
      const expectTotal = o.messages * (bots.length - 1)
      const duration = o.messages * o.gapMs
      const deadlineMs = duration + Math.max(30000, bots.length * o.messages * 500)
      const outs = await Promise.all(
        bots.map((b, i) =>
          b.chat({
            sends: Array.from({ length: o.messages }, (_, k) => ({ at: k * o.gapMs + (i * o.gapMs) / bots.length, text: `${b.name}:${k}` })),
            pollMs: o.pollMs,
            expectTotal,
            deadlineMs,
          }),
        ),
      )
      const s1 = await statsMark()
      const sentAt = new Map()
      for (const out of outs) for (const s of out.sent) sentAt.set(s.text, s.sentAt)
      const latency = []
      for (const out of outs) for (const s of out.seen) if (sentAt.has(s.text)) latency.push(s.seenAt - sentAt.get(s.text))
      const syncs = outs.flatMap((x) => x.syncs)
      result.scenarios.chat = {
        messages: o.messages * bots.length,
        expectedDeliveries: expectTotal * bots.length,
        deliveries: latency.length,
        complete: outs.every((x) => x.complete),
        elapsedMs: Math.max(...outs.map((x) => x.elapsedMs)),
        latency: summarize(latency),
        syncMs: summarize(syncs.map((s) => s.ms)),
        sentPerSync: summarize(syncs.map((s) => s.sent)),
        scanMs: summarize(outs.flatMap((x) => x.scans)),
        syncsPerBot: syncs.length / bots.length,
        wire: proxy.since(m),
        syncRequests: statsDelta(s0, s1, 'sync_requests'),
        relayMerkleBuilds: statsDelta(s0, s1, 'merkle_builds'),
      }
      o.log(`chat: ${latency.length}/${expectTotal * bots.length} deliveries, p50 ${result.scenarios.chat.latency.p50?.toFixed(0)} ms`)
    }
  } finally {
    await Promise.all(bots.map((b) => b.close()))
    const fin = await relay.stop()
    result.relay = { stats: fin.stats, proc: fin.proc, statsTicks: relay.statsHistory.length }
    await proxy.close()
    if (!o.keep) rmSync(work, { recursive: true, force: true })
  }
  return result
}

export function writeResult(result, outBase) {
  mkdirSync(join(outBase, '..'), { recursive: true })
  writeFileSync(`${outBase}.json`, JSON.stringify(result, null, 2) + '\n')
  writeFileSync(`${outBase}.md`, toMarkdown(result))
  return [`${outBase}.json`, `${outBase}.md`]
}

async function main() {
  const { values } = parseArgs({
    options: {
      bots: { type: 'string', default: '2' },
      history: { type: 'string', default: '1000' },
      messages: { type: 'string', default: '20' },
      'poll-ms': { type: 'string', default: '50' },
      'gap-ms': { type: 'string', default: '100' },
      reconnects: { type: 'string', default: '5' },
      scenarios: { type: 'string', default: 'all' },
      release: { type: 'boolean', default: false },
      keep: { type: 'boolean', default: false },
      out: { type: 'string' },
      quiet: { type: 'boolean', default: false },
    },
  })
  const opts = {
    bots: Number(values.bots),
    history: Number(values.history),
    messages: Number(values.messages),
    pollMs: Number(values['poll-ms']),
    gapMs: Number(values['gap-ms']),
    reconnects: Number(values.reconnects),
    scenarios: values.scenarios.split(','),
    release: values.release,
    keep: values.keep,
    log: values.quiet ? () => {} : (m) => console.error(`[relay-chat] ${m}`),
  }
  const result = await runBench(opts)
  const name =
    values.out ||
    join(repo, 'bench', 'results', `relay-chat-${result.meta.date.slice(0, 10)}-b${opts.bots}-h${opts.history}-${opts.release ? 'release' : 'debug'}`)
  const files = writeResult(result, name)
  console.log(toMarkdown(result))
  console.error(`[relay-chat] wrote ${files.join(', ')}`)
}

if (import.meta.url === `file://${process.argv[1]}`) {
  main().catch((e) => {
    console.error(e)
    process.exit(1)
  })
}
