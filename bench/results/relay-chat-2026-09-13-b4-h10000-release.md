# relay-chat baseline — 2026-09-13T00:29:32.234Z

bots=4 history=10000 messages=10 pollMs=50 relay=release node=v26.7.0 git=bf25af0

**Not published numbers.** One machine, loopback, debug/release as stated; see README for what is and is not measured.

## Cold join (fresh bot joins the seeded datastore)

| history ops | ms | received | merkleLeaves | bytes→relay | bytes←relay | relay merkle builds |
|---|---|---|---|---|---|---|
| 10000 | 68364.3 | 10000 | 3 | 341528 | 7376769 | 7 |

## Equal-replica reconnect (P0-3 / P0-4)

| bot | reconnects | sent/reconnect | ackDuplicate | ackAccepted | merkleNodes | merkleLeaves | ms p50 | ms p95 |
|---|---|---|---|---|---|---|---|---|
| bot0 | 1 | 10000 | 10000 | 0 | 0 | 0 | 170595.7 | 170595.7 |
| bot1 | 1 | 10000 | 10000 | 0 | 0 | 0 | 170687.6 | 170687.6 |
| bot2 | 1 | 10000 | 10000 | 0 | 0 | 0 | 170652.6 | 170652.6 |
| bot3 | 1 | 10000 | 10000 | 0 | 0 | 0 | 170670.6 | 170670.6 |

wire per reconnect: 7034600 bytes→relay, 597728 bytes←relay; relay: merkle builds 4 for 4 SYNC_REQUESTs; ops_duplicate 40000

## One-op delta (one bot writes, the others sync once)

| writer sent | writer accepted | reader received | reader ms p50 | reader ms p95 | bytes→relay total | bytes←relay total | relay merkle builds |
|---|---|---|---|---|---|---|---|
| 10002 | 2 | 2 | 170726.4 | 170736.5 | 28141499 | 2397746 | 13 |

## Relay process

```json
{
  "stats": {
    "sessions_accepted": 12,
    "sessions_authed": 12,
    "live_connections": 0,
    "ops_received": 90002,
    "ops_accepted": 10002,
    "ops_duplicate": 80000,
    "ops_rejected": 0,
    "sync_requests": 12,
    "merkle_builds": 39,
    "merkle_node_requests": 15,
    "merkle_leaf_requests": 12,
    "delta_requests": 6,
    "delta_ops_sent": 30006,
    "at": 1789260325899
  },
  "proc": {
    "rssKb": 11624,
    "hwmKb": 55756,
    "cpuMs": 8890
  },
  "statsTicks": 953
}
```
