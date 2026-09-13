# relay-chat baseline — 2026-09-13T00:26:21.941Z

bots=16 history=1000 messages=5 pollMs=50 relay=release node=v26.7.0 git=bf25af0

**Not published numbers.** One machine, loopback, debug/release as stated; see README for what is and is not measured.

## Cold join (fresh bot joins the seeded datastore)

| history ops | ms | received | merkleLeaves | bytes→relay | bytes←relay | relay merkle builds |
|---|---|---|---|---|---|---|
| 1000 | 429.8 | 1000 | 1 | 34901 | 737841 | 2 |

## Equal-replica reconnect (P0-3 / P0-4)

| bot | reconnects | sent/reconnect | ackDuplicate | ackAccepted | merkleNodes | merkleLeaves | ms p50 | ms p95 |
|---|---|---|---|---|---|---|---|---|
| bot0 | 2 | 1000 | 1000 | 0 | 0 | 0 | 16514.7 | 16515.7 |
| bot1 | 2 | 1000 | 1000 | 0 | 0 | 0 | 16525.8 | 16546.2 |
| bot2 | 2 | 1000 | 1000 | 0 | 0 | 0 | 16530.2 | 16536.2 |
| bot3 | 2 | 1000 | 1000 | 0 | 0 | 0 | 16543.1 | 16558.9 |
| bot4 | 2 | 1000 | 1000 | 0 | 0 | 0 | 16536.7 | 16569.6 |
| bot5 | 2 | 1000 | 1000 | 0 | 0 | 0 | 16549.9 | 16556.0 |
| bot6 | 2 | 1000 | 1000 | 0 | 0 | 0 | 16573.3 | 16581.1 |
| bot7 | 2 | 1000 | 1000 | 0 | 0 | 0 | 16565.3 | 16578.0 |
| bot8 | 2 | 1000 | 1000 | 0 | 0 | 0 | 16564.8 | 16576.8 |
| bot9 | 2 | 1000 | 1000 | 0 | 0 | 0 | 16566.2 | 16571.6 |
| bot10 | 2 | 1000 | 1000 | 0 | 0 | 0 | 16541.6 | 16563.1 |
| bot11 | 2 | 1000 | 1000 | 0 | 0 | 0 | 16568.2 | 16580.2 |
| bot12 | 2 | 1000 | 1000 | 0 | 0 | 0 | 16557.0 | 16564.5 |
| bot13 | 2 | 1000 | 1000 | 0 | 0 | 0 | 16572.1 | 16574.3 |
| bot14 | 2 | 1000 | 1000 | 0 | 0 | 0 | 16558.1 | 16570.6 |
| bot15 | 2 | 1000 | 1000 | 0 | 0 | 0 | 16567.3 | 16579.2 |

wire per reconnect: 703698 bytes→relay, 60378 bytes←relay; relay: merkle builds 32 for 32 SYNC_REQUESTs; ops_duplicate 32000

## One-op delta (one bot writes, the others sync once)

| writer sent | writer accepted | reader received | reader ms p50 | reader ms p95 | bytes→relay total | bytes←relay total | relay merkle builds |
|---|---|---|---|---|---|---|---|
| 1002 | 2 | 2 | 16600.4 | 16606.0 | 11267124 | 996075 | 46 |

## Sparse divergence (every bot writes offline, then all sync twice)

| ops per bot | pass | sent total | ackAccepted | ackDuplicate | received total | ms p50 | ms p95 | bytes→relay | bytes←relay |
|---|---|---|---|---|---|---|---|---|---|
| 4 | 1 | 16096 | 64 | 16032 | 948 | 17497.8 | 17519.6 | 11366699 | 1680644 |
| 4 | 2 | 17044 | 0 | 17044 | 12 | 17665.6 | 17679.1 | 12026950 | 1039617 |

## Chat (all bots send on a schedule while polling)

messages 80, deliveries 1200/1200, elapsed 57367 ms

| metric | n | min | p50 | p95 | p99 | max |
|---|---|---|---|---|---|---|
| delivery latency ms | 1200 | 18265.2 | 19797.5 | 36406.4 | 37558.3 | 39557.4 |
| sync ms | 35 | 17617.8 | 18277.0 | 19854.7 | 19857.1 | 19857.1 |
| sent per sync (full upload) | 35 | 1066 | 1076 | 1218 | 1218 | 1218 |
| listNodes scan ms | 19 | 2.2 | 4.7 | 12.3 | 12.3 | 12.3 |

wire: 35 connections, 26767163 bytes→relay, 4078066 bytes←relay; relay merkle builds 92, sync requests 35

## Relay process

```json
{
  "stats": {
    "sessions_accepted": 131,
    "sessions_authed": 131,
    "live_connections": 0,
    "ops_received": 119950,
    "ops_accepted": 1226,
    "ops_duplicate": 118724,
    "ops_rejected": 0,
    "sync_requests": 131,
    "merkle_builds": 267,
    "merkle_node_requests": 70,
    "merkle_leaf_requests": 66,
    "delta_requests": 66,
    "delta_ops_sent": 18390,
    "at": 1789259371885
  },
  "proc": {
    "rssKb": 13924,
    "hwmKb": 41664,
    "cpuMs": 10980
  },
  "statsTicks": 190
}
```
