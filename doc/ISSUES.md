Binding layer for Flutter?

~~Lamport clock pro/con?~~ **Resolved: HLC** (SPEC.md §2.4). Causality comes from the `deps` graph; the clock only serves LWW tiebreak, time-bucketed Merkle sync, and time-range queries — all need wall-clock proximity, which Lamport lacks (chattiest peer would win every LWW conflict). HLC degrades to Lamport behavior under broken clocks. Remaining work: future-clock poisoning mitigation (FINDINGS.CODEX.md H1) — acceptance/quarantine rule for far-future `physical_time`, not a clock swap.
