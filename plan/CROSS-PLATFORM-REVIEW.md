# ZeroDB cross-platform sharing review

Date: 2026-09-05. Reviewed upstream `5c40500` (origin/main), not the older local performance-review branch. This is an implementation/plan review, not a security audit or a GA declaration.

## Verdict

The core has substantial experimental evidence. The product goal is still unmet: independent apps and agents must share one authorized datastore across browser, CLI and desktop, with durable offline edits and recovery. Several working prototypes are not yet a supported cross-platform product. M3c completion alone must not be marketed as this outcome.

## Evidence and gaps

| Area | Evidence in reviewed tree | Assessment / missing product work |
|---|---|---|
| Local Rust/SQLite/CLI | `zerodb-storage/tests/`, `zerodb-cli/src/main.rs`; M1 ledger exit | Experimental local durability established historically; CLI exposes local/TCP exchange, not relay enrollment and agent automation lifecycle. |
| Node | `zerodb-napi/index.d.ts`, `test/m2-*`, `m3a-relay.test.mjs` | Native SDK and one-shot `connectRelay` exist. `autoConnect`/`serve` use legacy peer v2; not a unified secure relay autosync SDK. Source-build instructions are not installable multi-platform releases. |
| Durable relay/security | `zerodb-relay`, storage/relay E5–E8, `limits.rs`, `import_replay_equiv.rs` | Real implementation, not just prose. M3b/H10 remain open; draft transcript and limits are not security sign-off. |
| Independent TS | `conformance/ts/peer/` | NAPI-free, in-memory reference peer; supports initial SchemaEpoch/CreateNode/LWW and live relay catch-up. Smoke is TS→Rust relay→TS, not Rust storage↔TS peer partition/rejoin H9 evidence. Not a general SDK or browser package. |
| Browser | `zerodb-wasm`, `examples/browser-peer`, `apps/todo` | WASM memory + IndexedDB op journal exists. Driver speaks legacy `Hello/HelloOk/OpsMsg` sync v2 to NAPI `serve`, not RELAY 0.2. Browser identity seed is origin-readable. Durable acknowledgement, quota failure, reload, multi-tab ownership and authenticated WSS need browser-level tests. |
| Desktop | Rust/NAPI are reusable | No packaged desktop exemplar or explicit installed-app lifecycle/OS matrix in the plan. Choose one shell initially; do not build multiple shells before proving sharing. |
| Agents | CLI/Node are useful primitives | No dedicated bounded tool contract, credentials/enrollment UX, durable mutation retry IDs or resumable watch acceptance. MCP should be a thin adapter over the same authorization boundary, not another database. |
| Shared meaning | Schema IR and signed initial epoch exist | Sharing bytes is insufficient: apps need the same datastore, schema/version, IDs, CRDT types, provenance and supported subset. Unknown epochs must remain fail-closed. Full schema evolution stays M4b. |
| Release/CI | Rust Ubuntu, NAPI Ubuntu+Windows, TS conformance and TS peer jobs | CI configuration is not proof of runs on every platform. No macOS job or real browser matrix in this workflow; browser-model tests are not real browser persistence tests. |

## Priority findings

1. **P0 product gap: protocol split.** Promote one authenticated RELAY/WSS application route across adapters. Keep trusted-LAN legacy peer v2 explicitly experimental; no automatic security downgrade on failure.
2. **P0 product gap: enrollment and authority.** Distinct app/device/agent identities, explicit datastore invitation/grant/revoke, no founder-seed sharing. Use separate datastores for trust boundaries; filtered queries do not enforce confidentiality. Revocation cannot erase previously disclosed plaintext. H10 remains open until its checklist is satisfied.
3. **P0 assurance gap: H9 and release profile.** Land independent Rust-storage↔TS-peer positive/negative exchange, offline/reorder/duplicate/restart tests. Explicitly publish supported operations, versions, limitations and unresolved security dispositions. No milestone rename silently waives security.
4. **P1 delivery gap: browser durability and platform packaging.** Browser RELAY adapter, WSS deployment recipe, durable local ACK semantics, reload/offline/eviction behavior; installed Node/CLI and one desktop shell with a published OS/browser support matrix.
5. **P1 product gap: agent-safe API.** Non-interactive structured CRUD/query/sync/watch and enrollment, bounded results, cancellation and stable errors, retry idempotency at the command layer (not just OpId dedup), resumable change cursor and per-agent credentials. Authenticated local IPC if a sidecar is used; no unauthenticated public bridge.
6. **P1 planning drift.** README says M3b while PLAN says M3c; Reqall roadmap predates delivered work; old Reqall relay record describes CBOR and features no longer matching the live profile. Refresh without rewriting historical closed decisions.

## Acceptance target: XP-1

Two different applications (not two tabs of the same app), plus a non-interactive agent, share one datastore through the durable relay. Participants include a browser, native CLI, and one desktop shell. Each has a distinct authorized identity. One creates a Todo; another edits it; the agent consumes a change and makes a bounded authorized update. The shared schema includes title/LWW, done/Flag, tags/ORSet and tombstones once supported by every participating production adapter; never silently reinterpret an unsupported CRDT as LWW.

Partition peers, perform concurrent edits, hard-stop/restart clients and relay, reconnect, and compare accepted OpId sets and canonical materialized queries after quiescence. Assert duplicate/retried tool requests do not double-increment or create duplicate entities. Test encrypted properties, unauthorized/wrong-datastore writes, revocation before reconnect, unknown schema/version and resource exhaustion. A non-recipient relay must not see encrypted plaintext. Replicated public values remain public to holders.

Run browser reload, offline launch, IndexedDB commit/quota failure and multi-tab ownership tests in real browsers. Demonstrate packaged CLI and desktop recovery on the declared OS matrix. Publish unsupported combinations rather than claiming 'runs anywhere'. XS/XP labels are delivery acceptance tracks, not new wire formats. No target date until DQ-12 capacity is ratified.

## Sequencing

1. M3c/H9 + security disposition + install/compatibility profile. Existing epoch and TS peer slices are implemented narrowly; full gate open.
2. XS-1 sharing contract and common adapter surface; XS-2 identities/grants/revocation lifecycle. These can be specified/tested while M3c closes, not shipped as secure before prerequisite evidence.
3. M4a-share: browser RELAY/WSS and proven local durability; agent CLI/Node facade and one desktop shell. Split these from optional React hooks, OPFS and WebRTC. IndexedDB is sufficient initially only if durability tests pass.
4. XP-1 integrated cross-app/cross-agent pilot gate. Must pass before advertising cross-platform sharing readiness. Does not rename `v0.1.0` or claim GA.
5. M4b migration/snapshot/upgrade, M5 operability/lifecycle/assurance, then broader M6 ecosystem. Performance Stage 2/3 remain measurement-triggered; measure pilot restore/sync growth and publish budgets before expanding scale. GC remains disabled.

## Verification

Fresh Linux execution at the reviewed revision:
- `node conformance/ts/runner.mjs --lane required`: 115 passed, 0 failed.
- `node --test conformance/ts/peer/store.test.mjs conformance/ts/peer/napi-graph.test.mjs`: 10 passed, 0 failed, 0 skipped.

- `node --test conformance/ts/peer/smoke.test.mjs`: 1 passed, 0 failed, 0 skipped; real Rust relay process, TS write and second-TS-peer catch-up.
- `cargo test --workspace --locked`: exit 0; aggregate 264 passed, 0 failed, 7 ignored across 59 test-result summaries. Ignored tests are fixture/golden-generation helpers. This command does not run the NAPI JavaScript suite or real browser tests.
- Documentation: `git diff --check` passed; relative links in all six changed Markdown files resolve.

Logs: `/home/ubuntu/work/zero-aporia-rust-tests.log`, `/home/ubuntu/work/zero-aporia-ts-smoke.log`; results also recorded in Reqall. Browser, desktop, installed-package and cross-OS runs are not implied by the above. The new sharing tracks are planned acceptance criteria, not implemented features.
