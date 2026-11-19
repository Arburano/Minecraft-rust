# T0-07: WIT 0.1 and mod host spike

Status: implementation and measurements recorded on 2026-11-19; task remains in progress because interruption timing is not consistently within the acceptance bound. No completion PR or remote CI run yet.

## What was done

- Generated host bindings with `wasmtime::component::bindgen!` and SDK/guest bindings with wit-bindgen for `engine:api@0.1.0`. The existing WIT signatures remain unchanged; transactional semantics are clarified.
- Added the real Rust `mods/examples/hello` component for `wasm32-wasip2`, with six systems covering empty calls, valid writes, infinite loops, traps, rejected writes and alternate-state writes.
- Kept the guest and SDK `no_std`. The repository owner explicitly approved the isolated canonical-ABI allocator exception in [ADR-0012](../docs/adr/0012-no-std-guest-allocator.md). The compiled guest imports only the four engine interfaces, not WASI clock or I/O capabilities.
- Added a shared epoch-enabled Wasmtime engine, pooled instances, `InstancePre`, per-call wall-clock deadlines and a 200 µs watchdog. Fuel is not enabled. Wasmtime was already selected in the workspace/ADR-0006; this task begins using that dependency rather than introducing a different runtime.
- Added worker-owned single-section state with atomic block/component/message staging. Rejected writes invalidate the entire hook even if the guest catches the error. Traps discard pending changes without changing committed host state, and that state can be transferred to a replacement instance. The world is not behind a lock.
- Added a headless example, six host-state unit tests and seven integration tests against the compiled guest. Tests include a scalar reference comparison of all 4,096 section cells, Unicode/empty/repeated event batches, invalid and expired calls, rollback and recovery.
- Added Criterion benchmarks for empty `run-system`, 1,000 typed string events and 4,000 actual block changes, alternating states between iterations. Compilation and instance initialization are excluded; canonical ABI work and transaction commit are included.
- Added the owner-approved `wasm32-wasip2` target to the existing CI toolchain setup. No other CI behavior was changed. Updated the API guide, performance budgets, PRD measurement note and task status.

### Verification

The latest full `cargo test --workspace` run passed 56 unit/integration tests and two doctests, including all 13 modhost tests. `cargo clippy --workspace --all-targets -- -D warnings`, guest-target Clippy, `cargo fmt --all --check`, WIT parsing and compiled-component validation passed. Workspace benchmark compilation passed; the final hooks benchmark also compiled and ran. The headless example using the current guest printed `6 systems declared` and `4000 block changes committed`.

Local build artifacts used the owner-approved `D:/qoder-build-cache/t0-07-modhost` directory, with two Cargo build jobs and debug symbols disabled for local dev/test profiles. Release measurements retained the repository's thin-LTO profile. Tests build the guest with `--locked --release`, `CARGO_PROFILE_RELEASE_PANIC=abort` and a separate guest target directory; missing guest targets fail rather than skip tests.

Benchmarks ran on a physical ASUS TUF Gaming FX505DU, Ryzen 7 3750H (4 cores / 8 logical processors), approximately 13.94 GiB usable RAM, Windows 11 build 28000, Rust 1.97.1, Wasmtime 50.0.2 and Criterion 0.8.2. Three runs used 30 samples, 1 s warmup and 3 s requested measurement time each. Median run estimates were:

| Operation | Measured |
| --- | --- |
| Empty hook, including host wrapper | 1.1508 µs |
| 1,000 `Message` events with strings | 308.27 µs/batch, 308.27 ns/event |
| 4,000 alternating-state block writes | 139.88 µs/batch, 34.97 ns/change |

Full run estimates, confidence intervals, scope and deviations are in [performance budgets](../docs/process/performance-budgets.md#t0-07-measurements-2026-11-19). Frequency/background activity was not controlled; the event and block estimates varied by more than 5% across runs. Earlier VM primitive-list measurements are not an equivalent before/after baseline.

After the timer change and after compilation finished, three complete integration-suite repetitions passed with ten interruptions each (maximum overruns 689.4, 458.4 and 593.2 µs). Ten further focused repetitions passed all 100 interruptions, with a worst overrun of 604.6 µs. Every interruption test uses a 10 ms budget, verifies 4,000 staged writes were discarded, and continues work on a healthy worker. These quiet-machine passes do not erase the earlier corrected-timer failure described below.

## What went wrong

- Initial compilation exhausted C: disk space. After owner approval, this task's Cargo cache moved to D:; unrelated files and the other session's clone were not deleted.
- Some early compiler processes terminated with Windows error codes before completing. Later builds succeeded; an exact cause for those terminations was not established.
- A standard-library guest linked unwanted WASI clock/I/O imports. Switching to `no_std` exposed the missing `cabi_realloc` export on `wasm32-wasip2`. The owner-approved allocator module resolved linking while keeping engine-only imports; this is not a blanket unsafe exemption.
- An early missing example brace and missing benchmark crate documentation were caught by formatting/lint checks and corrected.
- The initial watchdog used `park_timeout(200 µs)`, which yielded measured overruns of 4.7229 ms and 4.3402 ms on Windows. Rust's Windows implementation uses a high-resolution timer for `thread::sleep`; switching to that primitive substantially reduced the observed latency.
- The corrected timer still produced a 1.5101 ms overrun during a repeated integration run while compilation was active. That test sequence stopped on failure. Subsequent quiet repetitions passed, but the failure remains relevant and the strict 1 ms assertion has not been weakened, ignored or retried automatically inside the test.
- Switching between package-only and workspace Cargo commands changed feature unification and caused expensive Wasmtime/Cranelift recompilation. A redundant task-owned benchmark build was stopped; final measurements used the workspace bench selection to reuse its dependencies. Guest builds use a separate target directory to avoid nested Cargo build-directory lock contention.

## Issues encountered

- **Acceptance blocker:** the observed 1.5101 ms corrected-timer overrun exceeds the required 1 ms. Ordinary OS scheduling is not a hard real-time guarantee. The task stays `in-progress`; load behavior needs resolution or an explicit owner decision about the requirement before completion. Quiet passing runs are reported as evidence, not as proof of a universal bound. Remote CI has not been exercised.
- **Permitted performance deviation:** end-to-end empty dispatch is approximately 15.1% over its target; the typed string-event path is approximately 61.7 times its target. Targets remain unchanged. The measurement includes string/list allocation, guest work and host-state updates; it is not just primitive-list transfer. Further profiling/optimization is needed before claiming production performance or allocation-free hot paths.
- **Spike boundaries:** the region is a single section and the component store is a simple host map, not production ECS or a complete persistent-state quota system. Canonical ABI marshalling allocates before host handlers can validate lifted lists. Epoch checks cannot preempt Rust host imports; bounded handlers check deadlines and the hook is checked before commit. Commit itself is host work, not guest execution.
- Permissions, mod registry, hot reload, disk compilation caching, rate-limited log delivery and complete server scheduler integration remain outside this spike. The logging import is currently a deadline-checked sink. The tick-continuation integration test exercises a healthy worker and `engine_core::Tick`, not a running multiplayer server.
