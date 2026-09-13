# relay-chat baseline — 2026-09-13T00:18:14.334Z

bots=4 history=1000 messages=20 pollMs=50 relay=release node=v26.7.0 git=bf25af0

**Not published numbers.** One machine, loopback, debug/release as stated; see README for what is and is not measured.

## Cold join (fresh bot joins the seeded datastore)

| history ops | ms | received | merkleLeaves | bytes→relay | bytes←relay | relay merkle builds |
|---|---|---|---|---|---|---|
| 1000 | 578.9 | 1000 | 1 | 34901 | 737841 | 2 |

## Equal-replica reconnect (P0-3 / P0-4)

| bot | reconnects | sent/reconnect | ackDuplicate | ackAccepted | merkleNodes | merkleLeaves | ms p50 | ms p95 |
|---|---|---|---|---|---|---|---|---|
| bot0 | 5 | 1000 | 1000 | 0 | 0 | 0 | 16521.7 | 16523.6 |
| bot1 | 5 | 1000 | 1000 | 0 | 0 | 0 | 16523.9 | 16525.1 |
| bot2 | 5 | 1000 | 1000 | 0 | 0 | 0 | 16522.8 | 16523.4 |
| bot3 | 5 | 1000 | 1000 | 0 | 0 | 0 | 16523.5 | 16525.2 |

wire per reconnect: 703698 bytes→relay, 60378 bytes←relay; relay: merkle builds 20 for 20 SYNC_REQUESTs; ops_duplicate 20000

## One-op delta (one bot writes, the others sync once)

| writer sent | writer accepted | reader received | reader ms p50 | reader ms p95 | bytes→relay total | bytes←relay total | relay merkle builds |
|---|---|---|---|---|---|---|---|
| 1002 | 2 | 2 | 16525.2 | 16527.0 | 2817504 | 247611 | 10 |

## Sparse divergence (every bot writes offline, then all sync twice)

| ops per bot | pass | sent total | ackAccepted | ackDuplicate | received total | ms p50 | ms p95 | bytes→relay | bytes←relay |
|---|---|---|---|---|---|---|---|---|---|
| 10 | 1 | 4048 | 40 | 4008 | 120 | 16724.8 | 16729.7 | 2854427 | 336224 |
| 10 | 2 | 4168 | 0 | 4168 | 0 | 16530.6 | 16532.7 | 2934780 | 251616 |

## Chat (all bots send on a schedule while polling)

messages 80, deliveries 240/240, elapsed 56049 ms

| metric | n | min | p50 | p95 | p99 | max |
|---|---|---|---|---|---|---|
| delivery latency ms | 240 | 17827.9 | 20741.8 | 38231.9 | 39033.1 | 39310.4 |
| sync ms | 11 | 16522.8 | 17794.1 | 18861.2 | 18861.2 | 18861.2 |
| sent per sync (full upload) | 11 | 1042 | 1082 | 1164 | 1164 | 1164 |
| listNodes scan ms | 7 | 1.7 | 2.5 | 3.4 | 3.4 | 3.4 |

wire: 11 connections, 8376516 bytes→relay, 1087832 bytes←relay; relay merkle builds 32, sync requests 11

## Relay process

```json
{
  "stats": {
    "sessions_accepted": 47,
    "sessions_authed": 47,
    "live_connections": 0,
    "ops_received": 45088,
    "ops_accepted": 1202,
    "ops_duplicate": 43886,
    "ops_rejected": 0,
    "sync_requests": 47,
    "merkle_builds": 85,
    "merkle_node_requests": 21,
    "merkle_leaf_requests": 17,
    "delta_requests": 17,
    "delta_ops_sent": 3606,
    "at": 1789258928255
  },
  "proc": {
    "rssKb": 9780,
    "hwmKb": 12276,
    "cpuMs": 4050
  },
  "statsTicks": 234
}
```
