# WASM API

Contract: [`../../wit/engine-api.wit`](../../wit/engine-api.wit), package `engine:api@0.1.0`. Syntax check: `wasm-tools component wit wit/engine-api.wit`.

## Running the stage-0 spike

```bash
rustup target add wasm32-wasip2
CARGO_PROFILE_RELEASE_PANIC=abort cargo build -p hello-mod --target wasm32-wasip2 --release
cargo run -p modhost --example hello -- target/wasm32-wasip2/release/hello_mod.wasm
cargo test -p modhost -- --nocapture
cargo bench -p modhost --bench hooks
```

Tests and benchmarks build the actual Rust guest automatically into a separate `target/guest` directory (or `$CARGO_TARGET_DIR/guest`). A missing target is an error, not a skipped test. The guest uses the SDK's generated bindings and a `no_std` allocator boundary approved in [ADR-0012](../adr/0012-no-std-guest-allocator.md); its imports are exclusively `engine:api`, with no WASI clock, filesystem, network, or environment.

`ModHost` owns a shared epoch-enabled engine and watchdog; `PreparedMod` caches pre-linking and creates pooled worker instances. The spike uses a worker-owned, single-section `Region` fixture and caller-provided state IDs rather than a complete server region scheduler or mod registry. It does not implement permissions, reload, a disk compilation cache, or production ECS storage.

Each hook stages block writes, component registrations/values, and outgoing messages. Reads see staged writes; only successful, within-budget calls commit. A rejected write poisons the transaction even if the guest ignores its error. Traps and deadlines discard all pending changes. A trapping component instance is not reused; the caller can continue other systems and instantiate a replacement with the retained host state. A failed transaction without a Wasm trap does not poison the instance.

The epoch watchdog requests checks every 200 microseconds; store callbacks compare the actual per-hook deadline, so an early epoch cannot interrupt an unexpired hook. This is not an OS real-time guarantee: the integration test measures the complete deadline-to-return overrun and asserts the task's 1 ms bound. Wasm epochs cannot preempt Rust host imports; those are bounded and check deadlines, and the hook is checked again before commit.

The spike caps each guest memory at 32 MiB, staged block/component writes at 16,384 per hook, component registrations at 64, and staged component/message payloads at 1 MiB. These are spike limits, not a complete production resource-accounting policy. Canonical ABI lifting/lowering of owned strings/lists allocates, as does the simple host component map; transaction vectors retain their capacity. The benchmark includes these costs instead of treating the old `list<u32>` experiment as a measurement of typed string events.

## Interfaces

| Interface | Direction | Purpose |
| --- | --- | --- |
| `types` | shared | positions, states, events, errors |
| `world-access` | host → guest (import) | reading and batched modification of blocks of its own region, `send-to` |
| `components` | import | registration of mod components, batched reading/writing of state in the host |
| `host-log` | import | logging with levels (rate-limited) |
| `guest` | mod export | `init` (declaration of systems and subscriptions), `on-events`, `run-system` |

## API design rules (mandatory for changes)

1. **Batching.** No function works with a single block or a single entity if it can be called for a list. Earlier VM measurements of primitive lists do not predict the cost of typed string events; the spike's end-to-end results and deviations are in [performance budgets](../process/performance-budgets.md#t0-07-measurements-2026-11-19).
2. **Target:** ≤ 1 µs per hook dispatch, ≤ 5 ns per event in a batch.
3. **State in the host.** The API does not encourage keeping the world in the mod's linear memory.
4. **Region.** Everything that reads or changes the world acts within the region; outside it — an `out-of-region` error, not a silent denial.
5. **Atomicity.** World changes from one call are applied together or not applied at all.
6. **No global time.** The mod receives the tick number from the host; the system clock is unavailable (test determinism).

## Versioning

- Semver of the package. A minor version only adds (new functions, new variants at the end). A major version may break.
- The engine supports the current and previous major via an adapter component.
- Any change to `wit/engine-api.wit` is a separate PR with updates to this document, `mod-sdk` and the test mod; a breaking change — with an ADR.

## Future interfaces (not in 0.1)

UI (menu and HUD widgets), render passes in WGSL (level 3), queries to global services (asynchronous), interfaces between mods (a mod exports its own WIT).
