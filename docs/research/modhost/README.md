# Modhost interruption-latency evidence (T0-13, 2026-11-20)

Machine: ASUS TUF FX505DU, Ryzen 7 3750H (8 logical CPUs), Windows 11 build 28000, physical. Rust 1.97.1, Wasmtime 50.0.2. Quiet = heavy applications closed by the owner, background not controlled; loaded = `cpu_load.exe` spinning on every logical CPU. Reproduce with `research\modhost\run-latency.cmd DATE "MACHINE STRING"` from the repository root in cmd.exe (prebuilt release tests, debug component test and cpu_load example required).

- `runs-2026-11-20.txt` — script header (machine, toolchain) and final status.
- `latency-{quiet,loaded}-2026-11-20-run{1,2,3}.txt` — final configuration, release, 1,000 interruptions per run, 100 ms hook budget; p50/p90/p99/max of overrun, watchdog wake, trap/unwind, post-return.
- `latency-*-samples.txt` — one raw line per interruption: `overrun_ns wake_ns trap_ns post_return_ns`.
- `functional-loaded-2026-11-20.txt` — 20 consecutive debug runs of the CI functional test under full CPU load (≤ 10 ms bound, owner decision No. 11); all passed.
- `steps-2026-11-20/` — raw outputs of the optimization-step experiments and the same-session baseline reference; see the [T0-13 report](../../../task-reports/t0-13-modhost-interruption-latency.md) for the configuration behind each prefix and the mixed-population caveat on the 10 ms-budget runs.