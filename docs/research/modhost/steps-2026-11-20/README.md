# Optimization-step experiments (T0-13, 2026-11-20)

Raw outputs behind the step table in the [T0-13 report](../../../../task-reports/t0-13-modhost-interruption-latency.md). Same machine and day as the final evidence; release profile; 1,000 interruptions per latency run. Prefixes:

- `baseline-*` — original code, **10 ms hook budget** (mixed population under load: some interruptions came from the host-import deadline check during staging, marked * in the report).
- `nobacktrace-*` — + `wasm_backtrace_max_frames(None)`, 10 ms budget (*).
- `priority-*` — + watchdog thread at Max priority, 100 ms budget.
- `callpriority-*` — + per-call worker-thread priority elevation, 100 ms budget.
- `gating-*` — + in-flight gating of the watchdog (final configuration, 200 µs tick), 100 ms budget.
- `tick100-*` — gating with a 100 µs tick (rejected: no gain, +5% empty-hook cost), 100 ms budget.
- `baseline-ref-*` — same-session baseline reference: original always-ticking 200 µs watchdog, backtraces on, normal priorities, diagnostic API only, 100 ms budget.
- `bench-baseline-ref.txt`, `bench-callpriority.txt`, `bench-tick100.txt`, `bench-final.txt`, `bench-final-rerun{1,2}.txt` — Criterion hooks benches (30 samples, 1 s warmup, 3 s measurement). `bench-final.txt` was taken immediately after the loaded evidence phase and is a machine-state outlier (empty hook 3.5844 µs); the two reruns on the idle machine (2.2873 / 2.2675 µs) are the reported final numbers.

`*-samples.txt` files hold one raw line per interruption: `overrun_ns wake_ns trap_ns post_return_ns`.