# T0-07. WIT 0.1 and mod host spike

- **Status:** done (closed with T0-13, 2026-11-20)
- **Depends on:** T0-01
- **Owner:** agent
- **PR:** https://github.com/Arburano/Minecraft-rust/pull/9 (merged; interruption criterion still open)

## Goal

Bring `wit/engine-api.wit` to 0.1: a Rust guest built with `wit-bindgen` (target `wasm32-wasip2`) in `mods/examples/hello`, and a host in `modhost` via `wasmtime::component::bindgen!`. Measure: an empty `run-system`, `on-events` with 1 000 events (`list<event>` with strings), `set-blocks` with 4 000 changes; verify epoch interruption of an infinite loop and discarding of a batch of changes.

## Specs

- [docs/modding/wasm-api.md](../../docs/modding/wasm-api.md)
- [docs/modding/overview.md](../../docs/modding/overview.md)
- [docs/adr/0006-wasm-component-model-epoch.md](../../docs/adr/0006-wasm-component-model-epoch.md)

## Deliverables

- Working example mod and host, integration test in `crates/modhost/tests/`.
- Benchmark and numbers in performance-budgets.md.
- Updated WIT (valid per `wasm-tools component wit`).

## Acceptance criteria

- [x] An infinite loop in a mod is interrupted and the server continues its tick (test). Reworded by owner decision No. 11: CI asserts overrun ≤ 10 ms; the 1 ms bound is a p99 latency target on a quiet desktop, measured and reported in performance-budgets (done in [T0-13](T0-13-modhost-interruption-latency.md)).
- [x] World changes from the interrupted call are not applied (test).
- [x] Hook dispatch ≤ 1 µs, ≤ 5 ns per event (or a documented deviation).

## Verification status (2026-11-19)

The guest, pooled host, rollback tests and benchmarks are implemented. The latest workspace test run passed all 56 tests and 2 doctests; strict Clippy, formatting, WIT/component validation and the example also passed. Performance targets are exceeded, with the permitted deviation documented in [performance budgets](../../docs/process/performance-budgets.md#t0-07-measurements-2026-11-19).

The interruption criterion remains open: 130 final quiet interruptions passed, but an earlier run with the corrected timer during concurrent compilation exceeded the 1 ms bound (1.5101 ms). The test has not been relaxed or ignored. See the [implementation report](../../task-reports/t0-07-wit-and-modhost-spike.md).

Update (review, 2026-11-19): PR #9 was merged and CI passed on the PR and on `main`. A review run on a 4 vCPU Linux cloud VM (VM numbers) reproduced the problem: quiet maxima 363–747 µs (5/5 pass), but with all cores busy 5/5 runs failed (1.66–5.80 ms). The strict 1 ms assertion is therefore flaky in CI by construction. The remaining work — owner decision ([open question No. 11](../../docs/open-questions.md)), splitting the test into a CI functional check and a separate latency measurement, overrun diagnosis and cheap reductions — is tracked in [T0-13](T0-13-modhost-interruption-latency.md). T0-07 is closed together with T0-13.

Update (2026-11-20): closed with [T0-13](T0-13-modhost-interruption-latency.md). The CI functional test now asserts ≤ 10 ms (owner decision No. 11) and passed 20/20 consecutive runs under full CPU load; the on-demand latency measurement (1,000 interruptions per run) met the quiet-desktop p99 ≤ 1 ms target in 3 quiet and 3 loaded runs on the owner's laptop. Numbers and diagnosis: [T0-13 report](../../task-reports/t0-13-modhost-interruption-latency.md), [performance budgets](../../docs/process/performance-budgets.md#t0-13-measurements-2026-11-20).

## Out of scope

No permissions, mod registry, or hot reload — stage 3.
