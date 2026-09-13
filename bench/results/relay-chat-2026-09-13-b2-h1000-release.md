# relay-chat baseline — 2026-09-13T00:22:45.699Z

bots=2 history=1000 messages=20 pollMs=50 relay=release node=v26.7.0 git=bf25af0

**Not published numbers.** One machine, loopback, debug/release as stated; see README for what is and is not measured.

## Cold join (fresh bot joins the seeded datastore)

| history ops | ms | received | merkleLeaves | bytes→relay | bytes←relay | relay merkle builds |
|---|---|---|---|---|---|---|
| 1000 | 477.0 | 1000 | 1 | 34901 | 737841 | 2 |

## Equal-replica reconnect (P0-3 / P0-4)

| bot | reconnects | sent/reconnect | ackDuplicate | ackAccepted | merkleNodes | merkleLeaves | ms p50 | ms p95 |
|---|---|---|---|---|---|---|---|---|
| bot0 | 5 | 1000 | 1000 | 0 | 0 | 0 | 16515.0 | 16517.0 |
| bot1 | 5 | 1000 | 1000 | 0 | 0 | 0 | 16514.9 | 16517.7 |

wire per reconnect: 703698 bytes→relay, 60378 bytes←relay; relay: merkle builds 10 for 10 SYNC_REQUESTs; ops_duplicate 10000

## One-op delta (one bot writes, the others sync once)

| writer sent | writer accepted | reader received | reader ms p50 | reader ms p95 | bytes→relay total | bytes←relay total | relay merkle builds |
|---|---|---|---|---|---|---|---|
| 1002 | 2 | 2 | 16522.3 | 16522.3 | 1409234 | 122867 | 4 |

## Sparse divergence (every bot writes offline, then all sync twice)

| ops per bot | pass | sent total | ackAccepted | ackDuplicate | received total | ms p50 | ms p95 | bytes→relay | bytes←relay |
|---|---|---|---|---|---|---|---|---|---|
| 10 | 1 | 2024 | 20 | 2004 | 20 | 16586.1 | 16588.0 | 1426033 | 139044 |
| 10 | 2 | 2044 | 0 | 2044 | 0 | 16516.6 | 16517.2 | 1438532 | 123372 |

## Chat (all bots send on a schedule while polling)

messages 40, deliveries 40/40, elapsed 34536 ms

| metric | n | min | p50 | p95 | p99 | max |
|---|---|---|---|---|---|---|
| delivery latency ms | 40 | 17796.3 | 17882.4 | 17957.6 | 34522.4 | 34522.4 |
| sync ms | 4 | 16514.7 | 16526.4 | 17808.8 | 17808.8 | 17808.8 |
| sent per sync (full upload) | 4 | 1022 | 1024 | 1062 | 1062 | 1062 |
| listNodes scan ms | 2 | 2.3 | 2.3 | 2.5 | 2.5 | 2.5 |

wire: 4 connections, 2938570 bytes→relay, 315186 bytes←relay; relay merkle builds 11, sync requests 4

## Relay process

```json
{
  "stats": {
    "sessions_accepted": 22,
    "sessions_authed": 22,
    "live_connections": 0,
    "ops_received": 21240,
    "ops_accepted": 1102,
    "ops_duplicate": 20138,
    "ops_rejected": 0,
    "sync_requests": 22,
    "merkle_builds": 38,
    "merkle_node_requests": 9,
    "merkle_leaf_requests": 7,
    "delta_requests": 6,
    "delta_ops_sent": 1102,
    "at": 1789259181536
  },
  "proc": {
    "rssKb": 8716,
    "hwmKb": 9764,
    "cpuMs": 1950
  },
  "statsTicks": 217
}
```
