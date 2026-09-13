# relay-chat — live relay benchmark harness

First slice of LEDGER `perf-bench`. N bot peers (one `zerodb-napi` durable
store per worker thread) share one datastore through a real `zerodb-relay`
on loopback and exchange chat messages. The harness records what the four
PERF.md P0 findings need before Stage 2/3 can be judged.

**Not published numbers.** One machine, loopback, no NAT, no TLS, polling
clients. Results under `bench/results/` are baselines with the run
parameters in their header. CI runs `smoke.test.mjs` for shape and
convergence only; there is no timing gate.

## Run

```bash
cd zerodb-napi && npm ci && npx napi build --platform --release --target "$(rustc -vV | sed -n 's/^host: //p')" && cd ..
node bench/relay-chat/run.mjs --release --bots 4 --history 1000 --messages 20
node bench/relay-chat/run.mjs --bots 2 --history 40 --messages 4 --scenarios chat   # quick
node --test bench/relay-chat/smoke.test.mjs
```

Flags: `--bots` (2), `--history` target op count seeded on bot0 (1000),
`--messages` per bot in the chat round (20), `--poll-ms` client poll
interval (50), `--gap-ms` between a bot's sends (100), `--reconnects` (5),
`--scenarios cold,reconnect,delta,sparse,chat|all`, `--release` (release
relay; default debug), `--keep` (leave the SQLite files under
`target/bench-relay-chat/`), `--out <base>` (default
`bench/results/relay-chat-<date>-b<bots>-h<history>-<build>`). Writes
`<base>.json` and `<base>.md`.

## What is measured

| Source | Metrics |
|--------|---------|
| Counting TCP proxy (`proxy.mjs`) between bots and relay | bytes and connections per direction, per scenario; client-independent |
| `connectRelay` summary per session | `sent`, `ackAccepted`, `ackDuplicate`, `ackRejected`, `received`, `applied`, `skipped`, `merkleNodes`, `merkleLeaves`, wall ms |
| Relay `--stats-interval-secs` line (`RelayStats`) | sessions, ops received/accepted/duplicate/rejected, `sync_requests`, **`merkle_builds`**, node/leaf/delta requests, delta ops sent |
| `/proc/<relay pid>` (Linux) | RSS, high-water RSS, CPU ms |
| Bot worker (`process.hrtime` shared across threads) | delivery latency per message per receiver, `listNodes` scan ms |

Scenarios (PERF.md "Relay" measurement plan):

- **cold** — first other bot joins the seeded datastore (catch-up bytes, walk leaves, relay builds).
- **reconnect** — every bot re-syncs an equal replica `--reconnects` times, concurrently. `sent` per reconnect equal to the history and `ackDuplicate == sent` is P0-3 as measured; `merkle_builds >= sync_requests` is P0-4.
- **delta** — one bot writes one message, the others sync once.
- **sparse** — every bot writes offline, then all sync twice; asserts convergence.
- **chat** — every bot sends `--messages` on a schedule while polling every `--poll-ms`; delivery latency is send→first visible on each other bot. `sent per sync` shows the full upload on every poll.

## First baseline (2026-09-13, release relay, loopback)

Files: `bench/results/relay-chat-2026-09-13-*.md`. Headline, not a claim:

- **Equal-replica reconnect at 1k history costs ~16.5 s wall and ~704 KB
  up per reconnect, every time, all `DUPLICATE`.** The time is not compute:
  the client re-uploads the full history and paces it at the advertised
  `ops_per_second` (100, `WelcomeLimits::advertised()`), so reconnect time
  grows linearly with history (`history / 100` seconds). Cold join of the
  same 1k history takes ~0.5 s and ~738 KB down. This is PERF P0-3 with a
  number on it; relay CPU is not the bottleneck at this size.
- **One full Merkle build per `SYNC_REQUEST`** (`merkle_builds ==
  sync_requests` on equal replicas; more when a walk descends). P0-4 as
  measured.
- **At 10k history the same reconnect is ~170.6 s and ~7.0 MB up** (4 bots
  concurrently, one reconnect each; `merkle_builds == sync_requests == 4`).
  Linear, as predicted.
- **Cold join at 10k took ~68 s for ~7.4 MB down** versus ~0.5 s at 1k: a
  136× cost for 10× history. Download is not paced, so this is the
  receiving client's per-op ingest (PERF P0-1, broad-scan projection
  maintenance) showing up on the wire path.
- A one-op delta costs the same as a reconnect (~16.5 s at 1k, ~170.7 s at
  10k), because the writer's one new op sits behind the full re-upload.
- 16 bots polling concurrently at 1k history: sync p50 ~18.3 s (vs ~16.5 s
  for 2–4 bots), delivery latency p50 ~19.8 s, p95 ~36 s (two polls); all
  1200 deliveries completed. The relay itself is not the bottleneck at
  this size (~11–30 MB RSS; CPU in the low seconds per run).
- Chat delivery latency through the relay is therefore bounded by
  history-sized re-uploads on every poll, not by `--poll-ms`.

Stage 3 "missing-only upload" would remove the re-upload; a per-session
`ops_per_second` that ignores `DUPLICATE`-bound uploads would only hide it.
Neither is started.

## Honest limitations

- **Polling clients.** `connectRelay` is one-shot; `autoConnect` push mode is the direct-peer protocol, so relay delivery latency here is bounded below by `--poll-ms` plus one full sync. Relay-side SUBSCRIBE fan-out is not exercised by the NAPI client.
- **Blocking NAPI calls** run one per worker thread; concurrency is real across bots but each bot serialises its own sync and writes.
- **P0-1** (local projection scans) and **P0-2** (direct-peer OpId manifest) are not exercised here. P0-1 is the local Stage 0 fixture (`perf_s0`); P0-2 needs a direct `serve`/`connectPeer` scenario set.
- Bytes are TCP payload through the proxy (WebSocket framing included, TLS excluded).
- No DataChannel path until M4a-browser-dc lands.
- `--release` builds only the relay; the NAPI addon is whatever `zerodb-napi/` last built (`npm run build` is release).

## Layout

| File | Role |
|------|------|
| `run.mjs` | CLI + `runBench(opts)`; scenario orchestration; writes JSON/markdown |
| `bot-worker.mjs` | One NAPI store per worker thread; `seed`/`say`/`sync`/`chat` commands |
| `bots.mjs` | Worker RPC wrapper |
| `relay.mjs` | Spawns `zerodb-relay --stats-interval-secs 1`, parses stats lines, samples `/proc` |
| `proxy.mjs` | Counting TCP proxy |
| `report.mjs` | Percentiles and markdown tables |
| `smoke.test.mjs` | CI shape/convergence test |
