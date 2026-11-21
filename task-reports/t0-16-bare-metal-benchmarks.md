# T0-16: bare-metal confirmation of VM-only benchmarks

## What was done

Confirmed the "Measured (VM …)" budget rows on real hardware, per the stage 0
exit criterion (`docs/process/testing-and-benchmarks.md`).

- Machine: owner's laptop, ASUS TUF FX505DU, Ryzen 7 3750H (4 cores / 8
  logical), 13.94 GiB RAM, Windows 11 build 28000, physical. Rust 1.97.1
  (pinned), release profile (thin LTO, 1 codegen unit). AC power, "High
  performance" scheme + "Best performance" overlay (verified before the
  runs); the owner closed other programs and left the machine idle.
- Protocol: `cargo bench -p world`, `cargo bench -p mesh`, `cargo bench -p
  light`, **three runs each** (2026-11-21 19:40–20:09), criterion defaults
  (100 samples, 3 s warm-up, 5 s measurement), T0-12 corpus seed 0.
- Continuous power/frequency logging: `power-log.txt` logged battery state
  and processor performance every 20 seconds (86 samples total). All samples
  confirmed AC power (`battery=2`), 100% battery charge, and active CPU boost
  (131–158%, min 131%).
- Previous attempt 1 runs (15:21–15:51) were demoted to `attempt-1-power-uncertain/`
  because the power state during the protocol could not be guaranteed and
  the laptop latched at base clock afterwards (documented in
  `attempt-1-power-uncertain/README.md`).
- All raw console outputs committed in
  [`docs/research/bare-metal-2026-11-21/`](../docs/research/bare-metal-2026-11-21/summary.md)
  (`world-run1.txt` … `light-run3.txt`, `power-log.txt`), plus `attempt-1-power-uncertain/`.
- [`summary.md`](../docs/research/bare-metal-2026-11-21/summary.md): machine
  and protocol, per-benchmark median of 3 runs + spread, VM vs bare-metal
  comparison, budget verdicts, frequency/power verification, owner decision.
- `docs/process/performance-budgets.md`: all 8 affected client rows updated
  to bare-metal values with the VM numbers kept in brackets:
  - **Confirmed (5):** `unpack_into` (2.8–3.0 µs paletted ≤ 5 µs),
    `fast_mesh` (53.9 µs ≤ 0.2 ms), mask update (4.4 ns ≤ 20 ns), render batch
    full re-copy (1.67 µs ≤ 50 µs), light add/remove (worst p95 24.7 / 20.5 µs
    ≤ 100 / 200 µs; corpus p95 2.8 / 14.2 µs).
  - **Missed (3), recorded as "Measured deviation" with targets unchanged:**
    `get` (8.3–8.8 ns paletted vs ≈ 3.8 ns reference), `set` (12.7–29.4 ns
    vs ≤ 10 ns at every width), gather + masks (112.8 µs vs ≤ 100 µs p50).
- Removed the "bare-metal confirmation pending" notes from the T0-03,
  T0-04, T0-05 and T0-08 reports and added lines pointing to the summary.
- `tasks/README.md`: carry-over item marked done; new stage 1 carry-over
  item for optimizing the missed hot paths; wave table updated.
- No code changes. Verification: `cargo fmt --all --check`,
  `cargo clippy --workspace --all-targets` (`-D warnings`),
  `cargo test --workspace`, `cargo bench --workspace --no-run` — all green
  on the same machine.

## What went wrong

1. **Attempt 1 ran with uncertain power / dropped to base clock afterwards.**
   In the initial 15:21–15:51 run, the machine operated on battery at some
   point and later latched at base clock (2300 MHz, no boost, ≈ 1.5× slower).
   Resolution: Attempt 1 was demoted to `attempt-1-power-uncertain/` and kept
   as evidence only. The suite was rerun (Attempt 2, 19:40–20:09) with confirmed
   AC power and a continuous 20-second frequency/battery log (`power-log.txt`)
   verifying boost was active across all 9 runs.
2. **Tooling friction on this Windows build.** `WMIC` is absent (used
   PowerShell CIM instead); `Get-Counter` fails because the counter names
   are localized (used the WMI performance class instead); Git Bash expanded
   `$_` inside a double-quoted PowerShell command (switched to single-quoted
   commands).

## Issues encountered

- **Three targets missed on bare metal.** Per the task (step 4) and
  AGENTS.md, the agent stopped and asked the owner instead of changing
  targets. Owner decision (2026-11-21): record the measured deviations, keep
  the targets, add a follow-up optimization task to the stage 1 carry-over.
  Interpretation: the misses are concentrated in random-access,
  memory-latency-bound paths; mobile Zen+ vs the server Xeon behind the VM
  numbers explains the direction, but the ≈ 2–3× gap is not explained by
  clock alone and needs profiling (cache behavior, 4K aliasing, layout)
  before any target renegotiation. The T0-03 hope that bare metal would read
  ≤ 10 ns at all widths is not confirmed: even the widths unaffected by
  aliasing read ≈ 20–21 ns.
- **The ≤ 5% spread / fixed-frequency ideal is not achievable on this
  laptop.** Frequency cannot be pinned from software; spreads reach ≈ 22.8%
  on 3-bit synthetic set (though mesh and light spreads stayed ≤ 8.7% and
  ≤ 6.3%). The confirmed values are development measurements on a
  thermally-limited laptop, not a production capacity guarantee.
- **Single-edit light maxima** up to 83.3 µs (run 2, remove_torch) exceed the
  median, but the budget is p95 (met with margin: ≤ 14.2 µs corpus, ≤ 20.5 µs
  worst synthetic); same scheduler/cache-noise caveat as the VM runs in T0-08.
- **Disk C: stayed 98–99% full** during the task; no build failures this
  time (the stale-artifact cleanup from T0-09 still holds). The disk
  pressure remains a machine-health issue for the owner.
- Out of scope per the task: the QUIC shaped-link re-run (`tc netem` needs
  Linux) stays a stage 1 carry-over item.
