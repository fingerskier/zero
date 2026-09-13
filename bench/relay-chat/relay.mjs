// Spawn a real `zerodb-relay` on loopback with periodic stats lines, and
// sample its RSS / CPU from /proc (Linux only; null elsewhere).
import { spawn, spawnSync } from 'node:child_process'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

export const repo = join(fileURLToPath(new URL('.', import.meta.url)), '../..')

export function relayBin(release) {
  const name = process.platform === 'win32' ? 'zerodb-relay.exe' : 'zerodb-relay'
  return join(repo, 'target', release ? 'release' : 'debug', name)
}

export function ensureRelayBuilt({ release = false } = {}) {
  const args = ['build', '-p', 'zerodb-relay', '--locked']
  if (release) args.push('--release')
  const built = spawnSync('cargo', args, { cwd: repo, encoding: 'utf8' })
  if (built.status !== 0) {
    throw new Error(`cargo ${args.join(' ')} failed:\n${built.stderr || built.stdout}`)
  }
}

const CLK_TCK = 100 // Linux default; only used for the CPU-ms estimate.

export function procSample(pid) {
  try {
    const status = readFileSync(`/proc/${pid}/status`, 'utf8')
    const stat = readFileSync(`/proc/${pid}/stat`, 'utf8')
    const kb = (k) => Number((status.match(new RegExp(`^${k}:\\s+(\\d+)`, 'm')) || [])[1] || 0)
    // Fields after the ")" of comm: state is index 0, utime index 11, stime index 12.
    const tail = stat.slice(stat.lastIndexOf(')') + 2).split(' ')
    const utime = Number(tail[11])
    const stime = Number(tail[12])
    return { rssKb: kb('VmRSS'), hwmKb: kb('VmHWM'), cpuMs: ((utime + stime) * 1000) / CLK_TCK }
  } catch {
    return null
  }
}

/**
 * Start a relay. Resolves once it prints its listen line. `stats()` returns
 * the last parsed `zerodb-relay stats {...}` line (or null), `stop()` waits
 * for one more stats tick so the final counters are captured, then kills.
 */
export async function startRelay({
  dbPath,
  release = false,
  statsIntervalSecs = 1,
  extraArgs = [],
} = {}) {
  const args = ['--path', dbPath, '--bind', '127.0.0.1:0']
  if (statsIntervalSecs > 0) args.push('--stats-interval-secs', String(statsIntervalSecs))
  args.push(...extraArgs)
  const proc = spawn(relayBin(release), args, { cwd: repo })
  const statsHistory = []
  let last = null
  let stderrTail = ''
  let lineBuf = ''
  const onLine = (line) => {
    const m = line.match(/^zerodb-relay stats (\{.*\})\s*$/)
    if (m) {
      try {
        last = JSON.parse(m[1])
        last.at = Date.now()
        statsHistory.push(last)
      } catch {
        /* ignore */
      }
    } else {
      stderrTail = (stderrTail + line + '\n').slice(-4000)
    }
  }
  proc.stderr.on('data', (chunk) => {
    lineBuf += chunk.toString()
    let i
    while ((i = lineBuf.indexOf('\n')) >= 0) {
      onLine(lineBuf.slice(0, i))
      lineBuf = lineBuf.slice(i + 1)
    }
  })
  const url = await new Promise((resolve, reject) => {
    let ready = false
    const timer = setTimeout(() => {
      if (!ready) {
        proc.kill()
        reject(new Error(`zerodb-relay did not print a listen address:\n${stderrTail}`))
      }
    }, 20000)
    let buf = ''
    const onData = (chunk) => {
      buf += chunk.toString()
      const m = buf.match(/listening on ws:\/\/\S+:(\d+)/)
      if (m) {
        ready = true
        clearTimeout(timer)
        proc.stdout.off('data', onData)
        proc.stderr.off('data', onData)
        resolve(`ws://127.0.0.1:${m[1]}`)
      }
    }
    proc.stdout.on('data', onData)
    proc.stderr.on('data', onData)
    proc.once('error', (e) => {
      if (!ready) {
        clearTimeout(timer)
        reject(e)
      }
    })
    proc.once('exit', (code) => {
      if (!ready) {
        clearTimeout(timer)
        reject(new Error(`zerodb-relay exited ${code} before listen:\n${stderrTail}`))
      }
    })
  })
  const port = Number(url.split(':').pop())
  const waitTick = async () => {
    if (statsIntervalSecs <= 0) return
    const before = statsHistory.length
    const deadline = Date.now() + statsIntervalSecs * 1000 + 1500
    while (statsHistory.length === before && Date.now() < deadline) {
      await new Promise((r) => setTimeout(r, 25))
    }
  }
  return {
    proc,
    pid: proc.pid,
    url,
    port,
    stats: () => last,
    statsHistory,
    waitTick,
    procSample: () => procSample(proc.pid),
    stderrTail: () => stderrTail,
    async stop() {
      await waitTick()
      const final = last
      const sample = procSample(proc.pid)
      proc.kill()
      await new Promise((r) => proc.once('exit', r))
      return { stats: final, proc: sample }
    },
  }
}
