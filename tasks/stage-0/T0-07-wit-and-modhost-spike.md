# T0-07. WIT 0.1 and mod host spike

- **Status:** todo
- **Depends on:** T0-01
- **Owner:** agent

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

- [ ] An infinite loop in a mod is interrupted ≤ 1 ms after the deadline, and the server continues its tick (test).
- [ ] World changes from the interrupted call are not applied (test).
- [ ] Hook dispatch ≤ 1 µs, ≤ 5 ns per event (or a documented deviation).

## Out of scope

No permissions, mod registry, or hot reload — stage 3.
