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
