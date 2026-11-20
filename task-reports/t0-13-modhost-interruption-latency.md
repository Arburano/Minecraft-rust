# T0-13: Mod interruption latency — split criterion, diagnose and reduce overrun

Task: [tasks/stage-0/T0-13-modhost-interruption-latency.md](../tasks/stage-0/T0-13-modhost-interruption-latency.md). Closes [T0-07](../tasks/stage-0/T0-07-wit-and-modhost-spike.md). Owner decision: [open question No. 11](../docs/open-questions.md) (answered 2026-11-19, merged in PR #12): CI functional bound ≤ 10 ms; ≤ 1 ms p99 on a quiet desktop is a measured latency target, not a CI assertion.

## What was done

1. **Split the criterion.** The functional CI test `infinite_loop_is_interrupted_and_tick_continues` (`crates/modhost/tests/component.rs`) now asserts `overrun <= CI_OVERRUN_BOUND` with `CI_OVERRUN_BOUND = 10 ms` and the comment `// Decision: open question 11`. It keeps 10 interruptions, asserts all 4,000 staged writes are discarded, that the trapped worker refuses further calls, and that a healthy worker continues the tick. No retries, not ignored. The old strict 1 ms assertion is gone from CI.
2. **On-demand latency measurement.** `crates/modhost/tests/interruption_latency.rs` (`#[ignore]`, run with `--release -- --ignored --nocapture`): 1,000 interruptions by default (`MODHOST_LATENCY_COUNT`), optional raw samples (`MODHOST_LATENCY_SAMPLES`, absolute path; one line per interruption: `overrun_ns wake_ns trap_ns post_return_ns`). Prints p50/p90/p99/max for four metrics and an informational MET/NOT MET line for the quiet-desktop 1 ms p99 target. The hook budget is 100 ms so the 4,000-write staging phase completes even under full CPU load and every measured interruption is the epoch watchdog stopping the infinite loop, not the host-import deadline check firing during staging.
3. **Overrun diagnostics without hot-path allocation.** `ModWorker::last_interruption()` returns `InterruptionTiming { deadline, interrupt, returned }`: the epoch callback records `interrupt_at` when it returns `UpdateDeadline::Interrupt`, `call()` records `returned_at` on the deadline path. Two `Option<Instant>` writes, no allocation. The test derives watchdog wake (deadline → interrupt), trap/unwind (interrupt → return) and post-return (return → observer, i.e. measuring-thread preemption) from these instants.
4. **Cheap reductions, each measured before/after (kept only what helps):**
   - **Kept: interruption backtraces off** — `Config::wasm_backtrace_max_frames(None)` (the Wasmtime 50 replacement for the deprecated `wasm_backtrace(false)`). Trap/unwind p50 fell from ≈ 18 µs to ≈ 7–8 µs.
   - **Kept: raised thread priorities** (`thread-priority` 3.1.1, MIT; new dependency justified in the PR, version in `[workspace.dependencies]`, no `unsafe` in the repository). The watchdog thread runs at `ThreadPriority::Max`, and — the decisive part — the **calling worker thread** is elevated for the duration of every guest call (`CallPriority` RAII guard, restored on every exit path, best-effort). The epoch check executes on the worker thread, so an off-CPU worker cannot observe the watchdog's increment; elevating only the watchdog left loaded p99 at 19.7 ms and the functional test failed 2/5 under load. Per-call elevation brought loaded p99 to 0.525 ms and the functional test to 5/5. The owner approved keeping the per-call elevation despite the hook-cost increase (≈ +1.06 µs, three priority syscalls per call).
   - **Kept: in-flight gating.** An `AtomicUsize` in-flight counter plus `park`/`unpark` makes the watchdog tick only while at least one guest call is in flight; an idle server parks instead of waking 5,000 times per second. Unpark tokens are not lost (a call starting between the load and the park makes park return immediately). Overrun did not increase (loaded p99 0.520 ms vs 0.525 ms without gating). Covered by the unit test `watchdog_parks_while_idle_and_ticks_while_in_flight`.
   - **Rejected: 100 µs tick.** No measurable overrun gain over 200 µs (quiet p99 0.553 vs 0.558 ms; loaded p99 0.522 vs 0.520 ms; the effective Windows wake granularity here is ≈ 0.5 ms regardless of the requested sleep), while the empty hook grew 2.209 → 2.4406 µs (+4.95%, p = 0.03) from doubled wakeups. Kept 200 µs.
5. **Load generator and evidence script.** `crates/modhost/examples/cpu_load.rs` spins one thread per logical CPU. `research/modhost/run-latency.cmd DATE "MACHINE STRING"` records 3 quiet + 3 loaded latency runs (release, 1,000 interruptions each, raw samples kept) and 20 consecutive functional-test runs under load (debug, CI-equivalent) into `docs/research/modhost/`.
6. **Docs aligned with the owner's answer:** PRD §"Security and control" per-mod budgets line, `docs/modding/wasm-api.md` watchdog paragraph, `docs/process/performance-budgets.md` overrun row and the new T0-13 measurement section. T0-07 criterion ticked in its reformulated form; T0-07 and T0-13 closed.

### Measurements

Machine: ASUS TUF Gaming FX505DU, AMD Ryzen 7 3750H (4 cores / 8 logical processors), ≈ 13.94 GiB usable RAM, Windows 11 build 28000; physical machine, not a VM. Rust 1.97.1, Wasmtime 50.0.2, Criterion 0.8.2. Release profile with thin LTO and one codegen unit; the functional-test evidence runs are debug, as in CI. Date: 2026-11-20. CPU frequency was not pinned. "Quiet" = the owner closed heavy applications beforehand; background activity was not controlled. "Loaded" = `cpu_load.exe` spinning on all 8 logical CPUs. Each latency run performs 1,000 interruptions with a 100 ms hook budget unless marked *.

**Baseline reference** (original always-ticking 200 µs watchdog, backtraces on, normal priorities; diagnostic API added; measured in the same session as the final configuration):

| Metric | p50 | p90 | p99 | max |
| --- | --- | --- | --- | --- |
| overrun, quiet | 0.298 ms | 0.522 ms | 1.019 ms | 26.041 ms |
| overrun, loaded | 6.437 ms | 23.140 ms | 35.199 ms | 78.794 ms |

Baseline hooks bench (30 samples, 1 s warmup, 3 s measurement): `empty_run_system` 1.2994 µs [1.2122, 1.4268]; `on_events_1000_strings` 312.30 µs; `set_blocks_4000` 148.20 µs [138.90, 159.68].

**Optimization steps** (1,000 interruptions per cell; * = earlier 10 ms hook budget, a mixed population in which some interruptions under load came from the host-import deadline check during staging — not comparable with the 100 ms rows; kept for the record):

| Configuration | quiet p50/p99/max, ms | loaded p50/p99/max, ms |
| --- | --- | --- |
| baseline * | 0.354 / 0.566 / 0.973 | 8.425 / 33.941 / 47.061 |
| + backtraces off * | 0.335 / 0.542 / 1.009 | 7.216 / 28.547 / 47.838 |
| + watchdog priority only | 0.273 / 0.552 / 9.324 | 0.314 / 19.716 / 29.919 |
| + per-call worker priority | 0.270 / 0.555 / 1.854 | 0.306 / 0.525 / 1.004 |
| + in-flight gating (final, 200 µs) | 0.281 / 0.558 / 1.000 | 0.224 / 0.520 / 0.975 |
| 100 µs tick experiment (rejected) | 0.293 / 0.553 / 1.045 | 0.291 / 0.522 / 0.568 |

**Overrun breakdown (final configuration).** Watchdog wake dominates: quiet p50 ≈ 0.31–0.34 ms (tick residual plus scheduler latency), p99 ≈ 0.62 ms. Trap/unwind after disabling backtraces: p50 ≈ 13–15 µs in the final evidence runs (≈ 7–8 µs in the earlier step measurements; both far below the wake component), p99 ≤ 30 µs. Post-return (measuring-thread preemption) is reported separately: under load its max reached ≈ 35 ms while the engine-internal overrun stayed ≤ 4.038 ms — the observer, not the interruption, was preempted. Under load the raised worker priority keeps the wake path short; the remaining tail is OS scheduling, not engine work.

**Final evidence** (`research\modhost\run-latency.cmd 2026-11-20 "ASUS TUF FX505DU; Ryzen 7 3750H; 14 GiB; Windows 11 build 28000"`, outputs and raw samples in [docs/research/modhost/](../docs/research/modhost/)):

| Condition, 3 runs × 1,000 interruptions | p50 | p99 | max |
| --- | --- | --- | --- |
| Quiet, overrun | 0.327 / 0.324 / 0.353 ms | 0.633 / 0.633 / 0.637 ms | 2.070 / 1.340 / 2.961 ms |
| Loaded, overrun | 0.301 / 0.300 / 0.312 ms | 0.539 / 0.534 / 0.534 ms | 0.694 / 4.038 / 0.984 ms |

The quiet-desktop p99 ≤ 1 ms target was met in all six runs, including the loaded ones on this machine. The functional CI test (debug, as CI) passed **20/20 consecutive runs under full CPU load**, per-run maxima 356.4–973.2 µs — an order of magnitude below the 10 ms bound.

**Hook cost.** T0-07 baseline 1.1508 µs → same-session baseline reference 1.2994 µs [1.2122, 1.4268] → final configuration 2.2873 µs [2.1857, 2.3925] and 2.2675 µs [2.1603, 2.3949] (two 30-sample runs on the idle machine). The ≈ +1 µs over the same-session baseline is the three thread-priority syscalls per call (get/set/restore) plus the in-flight counter/unpark; the owner approved this tradeoff. A first final-configuration measurement taken immediately after the loaded evidence phase gave 3.5844 µs; it was discarded as a machine-state outlier and re-measured twice on the idle machine with consistent results. The deviation from the ≤ 1 µs dispatch target grows to ≈ 128%; the target itself is unchanged and the default 0.5 ms per-tick budget absorbs the added microsecond. Event and block batches in the final runs (196.9–204.2 µs and 90.7–91.0 µs) are faster than the earlier same-day runs (312.3 / 148.2 µs) purely from machine state; the optimizations do not touch the marshalling path, and per the T0-07 note, differences at that scale across sessions are not optimization results.

### Verification

Local, 2026-11-20, all green: `cargo test --workspace` — 79 tests + 2 doctests passed, 0 failed (includes the functional interruption test and the new watchdog-gating unit test); `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings` — no warnings; `cargo bench --workspace --no-run`. Evidence matrix: 3 quiet + 3 loaded latency runs and 20/20 loaded functional runs (above). `origin/main` (T0-11/T0-12, PR #13/#14) was merged into the branch before the PR; conflicts in `tasks/README.md` and `Cargo.lock` resolved keeping both sides. [PR #15](https://github.com/Arburano/Minecraft-rust/pull/15) CI green on the merged tree (run 44691005658).

## What went wrong

- The strict 1 ms CI assertion was flaky by construction (T0-07 evidence: quiet passes, 5/5 loaded failures on the Linux VM). Fixed by the owner-approved split, not by retries or ignoring.
- Raising only the watchdog priority did not fix the loaded tail: the functional test still failed 2/5 under load with overruns > 10 ms. Diagnosis: the epoch check runs on the worker thread, and a preempted worker cannot observe the increment. Fixed by per-call worker elevation (owner-approved cost).
- With the original 10 ms hook budget, loaded latency runs panicked on `last_interruption().expect(...)`: the host-import deadline check trapped during the 4,000-write staging phase (preempted > 10 ms), producing `Error::Deadline` without an epoch interrupt. Fixed by raising the measurement hook budget to 100 ms; the earlier 10 ms loaded rows are a mixed population and are marked as such.
- `option_env!("PROFILE")` reported "unknown": cargo does not set `PROFILE` when compiling test crates. Replaced with `cfg!(debug_assertions)`.
- Wasmtime 50 API surprises: the epoch callback receives `StoreHookState<T>` by value (needs `|mut state|` for `data_mut()`), and `wasm_backtrace(false)` is deprecated in favor of `wasm_backtrace_max_frames(None)`.
- The 100 µs tick hypothesis failed: Windows effective wake granularity (≈ 0.5 ms) swallows the finer request, and the doubled wakeup rate cost ≈ 5% on the empty hook. Rejected with numbers.

## Issues encountered

- `cargo test` runs with the crate directory as CWD: a relative `MODHOST_LATENCY_SAMPLES` path failed with os error 3. The script and documentation use absolute paths.
- Every `lib.rs` change costs a ≈ 6–7 minute release relink (thin LTO, one codegen unit, Wasmtime); the bench profile relinks separately. Builds were batched and run in the background; measurement runs never overlapped a build.
- Git Bash: `&` backgrounds a whole `&&` chain, so `cpu_load.exe` is launched on its own line; `taskkill //F //IM cpu_load.exe` needs doubled slashes.
- The rustfmt hook touches files after writes; edits after a hook run need a fresh read.
- The baseline-reference quiet run still showed a 26 ms max despite closed applications: background activity is not controlled on this machine, which is exactly why the target is stated as p99 and the CI bound is 10 ms. Reported honestly rather than cherry-picked.
- One release build was interrupted by the owner mid-session; process state was checked (no stray cargo/rustc/cpu_load), the working tree was intact, and the build was restarted without data loss.
- The first final-configuration hooks bench, taken immediately after the loaded evidence phase, reported 3.5844 µs for the empty hook while the longer benchmarks stayed normal; two re-runs on the idle machine gave 2.2873 / 2.2675 µs. The outlier was discarded and re-measured, not averaged in.
- Running the evidence `.cmd` from Git Bash required `MSYS_NO_PATHCONV=1 MSYS2_ARG_CONV_EXCL='*' cmd /c …`: MSYS otherwise mangles the script path and the quoted machine string (semicolons/spaces) passed to cmd.exe.
- The post-merge local verification first failed with "no space on device" (os error 112): the C: drive filled up during the session's many profile builds. Deleted the regenerable `target/debug/incremental` cache and re-ran; GitHub CI on the same tree was already green.

**Limitations.** All numbers are from one physical laptop; frequency not pinned; the load is a spin loop, not a real server workload. The final configuration was not re-measured on the Linux VM — the CI functional bound (≤ 10 ms) is exercised on GitHub runners by the PR CI itself. The quiet-desktop 1 ms p99 target is informational and can legitimately fail on a busy desktop.