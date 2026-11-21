# T0-16. Bare-metal confirmation of VM-only benchmarks

- **Status:** done (2026-11-21)
- **Depends on:** T0-03, T0-04, T0-05, T0-08, T0-12 (all done)
- **Owner:** agent running **on the owner's laptop** (Claude Code locally); human only keeps the laptop on AC power and idle

## Goal

Several budget rows are "Measured (VM …)" with ±30% noise: section `get`/`set`/`unpack_into` (T0-03), gather + masks, `fast_mesh`, mask update (T0-04), render batch copy (T0-05), lighting add/remove (T0-08). The stage 0 exit criterion needs them confirmed on real hardware (`docs/process/testing-and-benchmarks.md`: final numbers on bare metal).

## Steps

1. [human] AC power, Windows power mode "Best performance", close other programs, do not use the laptop during the runs.
2. `git pull`, then **three** runs of each:
   ```text
   cargo bench -p world
   cargo bench -p mesh
   cargo bench -p light
   ```
   Keep the console output of every run in `docs/research/bare-metal-2026-MM-DD/` (`world-run1.txt`, …).
3. Write `docs/research/bare-metal-2026-MM-DD/summary.md`: machine (ASUS TUF FX505DU, Ryzen 7 3750H, RAM, Windows build, Rust version), per benchmark the median of the 3 runs and the spread, VM value vs bare-metal value.
4. Update every affected row in `docs/process/performance-budgets.md` to "Measured (bare metal …)", keeping the VM number in brackets. If a target is **missed** on bare metal: stop and ask the owner (AGENTS.md), do not change targets.
5. Remove the "bare-metal confirmation pending" notes from the T0-03/04/05/08 reports' budget references (add a line pointing to the summary), and the corresponding carry-over item in `tasks/README.md`.

## Acceptance criteria

- [x] Raw outputs of 3 runs per crate committed; summary written.
- [x] Budget rows updated (or the owner consulted on a miss).
- [x] Report `task-reports/t0-16-bare-metal-benchmarks.md`, CI green.

## Results (2026-11-21)

Nine runs (3 × `world`, `mesh`, `light`) on the owner's laptop, raw outputs
and the full tables in
[docs/research/bare-metal-2026-11-21/](../../docs/research/bare-metal-2026-11-21/summary.md).

- Confirmed: `unpack_into` (2.8–3.0 µs), `fast_mesh` (53.9 µs), mask update
  (4.4 ns), render batch full re-copy (1.67 µs), light add/remove (worst p95
  24.7 / 20.5 µs; corpus p95 2.8 / 14.2 µs).
- Missed, recorded as deviations with targets kept (owner decision
  2026-11-21): `get` 8.3–8.8 ns paletted (≈ 3.8 ns reference), `set`
  12.7–29.4 ns (≤ 10 ns), gather + masks 112.8 µs (≤ 100 µs p50).
  Follow-up optimization task added to the stage 1 carry-over.
- Frequency and power: attempt 1 (run earlier with uncertain power and
  latching at base clock) was demoted to `attempt-1-power-uncertain/`.
  The reported numbers are from attempt 2, verified with continuous AC power
  and CPU boost active (131–158%) logged in `power-log.txt`.

## Out of scope

The QUIC shaped-link re-run (`tc netem` needs Linux) stays a stage 1 carry-over item.
