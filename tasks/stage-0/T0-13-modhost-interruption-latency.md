# T0-13. Mod interruption latency: split the criterion, diagnose and reduce overrun

- **Status:** in-progress
- **Depends on:** T0-07 (implementation merged in PR #9); [open question No. 11](../../docs/open-questions.md) — answered 2026-11-19, the task can start
- **Owner:** agent

## Background

T0-07 acceptance criterion "an infinite loop in a mod is interrupted ≤ 1 ms after the deadline" is not met under CPU load. The epoch watchdog is an ordinary OS thread (`crates/modhost/src/lib.rs`, `EpochWatchdog`, `thread::sleep(200 µs)` + `increment_epoch`); when all cores are busy the OS does not schedule it in time. This is not a Windows bug and cannot be guaranteed without a real-time OS.

Evidence:

- T0-07 report (Ryzen 7 3750H, Windows 11, physical): quiet runs ≤ 0.69 ms; one run during concurrent compilation 1.5101 ms.
- Review check on 2026-11-19 (cloud VM, 4 vCPU, Linux — VM numbers): `infinite_loop_is_interrupted_and_tick_continues` quiet, 5 runs × 10 interruptions: maxima 747, 474, 376, 363, 383 µs, all passed. With 4 busy-loop processes saturating all 4 vCPUs: 5 of 5 runs **failed**, first failing overruns 5.80, 4.45, 1.66, 3.70, 4.49 ms.
- The strict 1 ms assertion runs in CI (`cargo test --workspace` on shared GitHub runners). CI passed twice by luck; the test is **flaky by construction** and will eventually block unrelated PRs.
- The docs disagree: `docs/PRD.md` §"Per-mod budgets" says epoch is "a watchdog against hangs, not precise accounting: accuracy equals the epoch timer period (e.g. 1 ms)", while `docs/PRD.md` ("Hung mod … interrupted ≤ 1 ms after the deadline") and T0-07 require a hard 1 ms bound.

## Goal

Make CI deterministic, turn the 1 ms bound into a measured latency target, and reduce the overrun where it is cheap — without silently weakening the requirement.

## Steps

1. **Split the test (owner decision No. 11: CI bound ≤ 10 ms; p99 ≤ 1 ms quiet is a measured target, not a CI assertion).**
   - Functional test in CI: the infinite loop is interrupted, all 4,000 staged writes are discarded, the healthy worker continues the tick. Overrun bound ≤ 10 ms per decision No. 11 (comment next to the constant: `// Decision: open question 11`). This still catches real regressions (`park_timeout` gave 4.7 ms; a hang is seconds).
   - Latency measurement outside default CI: an `#[ignore]` test or a Criterion/custom bench that performs ≥ 1,000 interruptions and prints p50 / p99 / max overrun. Run it quiet and under load (all cores busy) and record both.
   - Do **not** add retries, do not `#[ignore]` the functional test, do not loosen the bound without the owner's decision.
2. **Diagnose where the overrun comes from.** Record the instant the epoch callback returns `UpdateDeadline::Interrupt` separately from the instant `ModWorker::call` returns. Report three parts: watchdog wake latency (deadline → callback), trap/unwind cost (callback → return), measuring-thread preemption. Diagnostics must not allocate in the hot path.
3. **Cheap reductions — each with a before/after measurement (AGENTS.md rule 3), keep only what helps:**
   - `Config::wasm_backtrace(false)` (or the equivalent in Wasmtime 50) so an interruption does not capture a backtrace. Hypothesis, unmeasured.
   - Raise the watchdog thread priority via a crate (e.g. `thread-priority`), no `unsafe` in the repository; justify the new dependency in the PR (rule 5), version in `[workspace.dependencies]`.
   - Tick the watchdog only while at least one hook call is in flight (e.g. an atomic in-flight counter + park/unpark), instead of 5,000 wake-ups per second while the server is idle. Must not increase overrun; measure.
   - Optionally evaluate a 100 µs tick: report the CPU cost of the watchdog vs the overrun gain.
4. **Docs.** Update `docs/process/performance-budgets.md` (row "Deadline-to-interruption overrun": new definition + p50/p99/max quiet and under load, machine, VM flag, run count), `docs/modding/wasm-api.md` (watchdog paragraph), and the PRD lines above so they agree with the owner's answer.
5. **Close T0-07.** When the owner's answer is applied and CI is green, tick the T0-07 interruption criterion in its reformulated form, set T0-07 and T0-13 to `done` in their files and `tasks/README.md`, and write `task-reports/t0-13-modhost-interruption-latency.md`.

## Specs

- [docs/adr/0006-wasm-component-model-epoch.md](../../docs/adr/0006-wasm-component-model-epoch.md) — epoch always on; fuel only in tests/replays (do not switch production to fuel).
- [docs/modding/wasm-api.md](../../docs/modding/wasm-api.md)
- [docs/process/performance-budgets.md](../../docs/process/performance-budgets.md#t0-07-measurements-2026-11-19)
- [T0-07 report](../../task-reports/t0-07-wit-and-modhost-spike.md)

## Acceptance criteria

- [ ] The functional interruption test is deterministic in CI: 20 consecutive local runs with all cores busy pass (report the machine).
- [ ] The latency measurement exists, runs on demand, and its p50/p99/max (quiet and under load) are in performance-budgets with machine and run count.
- [ ] The overrun breakdown (watchdog wake / trap / preemption) is in the task report.
- [ ] Every kept optimization has before/after numbers; rejected ones are listed in the report.
- [ ] Docs (PRD, wasm-api, performance-budgets, T0-07 task) agree with the owner's answer to No. 11.

## Out of scope

Real-time OS scheduling, fuel in production (ADR-0006), permissions, mod registry, hot reload.
