# T1-15. Stage 1 exit measurement

- **Status:** in progress (harness done; owner: laptop runs)
- **Depends on:** T1-01 … T1-14
- **Owner:** agent writes the harness, owner's laptop runs it

## Goal

Measure the stage 1 exit criterion and the remaining client hypothesis: 60 FPS at 12 chunks on the iGPU (Vega 10, open question 12) with 1% low ≥ 45, RAM ≤ 1.5 GB without LOD (internal budget 1 GiB — the "Client RAM" hypothesis), 30 minutes of walking without frame spikes > 33 ms, start to menu ≤ 5 s.

## Specs

- [roadmap](../../docs/process/roadmap.md) — stage 1 exit
- [performance-budgets.md](../../docs/process/performance-budgets.md)
- [stage 0 exit review](../../docs/research/stage-0-exit-2026-11-21.md)

## Deliverables

- Scripted walk/fly route through a fixed-seed world; frame-time log, RAM samples, spike detector.
- Run instructions for the owner's laptop (like T0-14); results in `docs/research/stage-1-exit-YYYY-MM-DD/`.
- GTX 1660 Ti run as additional data; GTX 1050 targets stay unverified (open question 12).

## Harness (done)

- `client --bench-route <SECONDS>` creates a fresh world (seed 12345), flies a square route (1000-block sides, fly speed 0.5 per tick, 12 blocks above the ground), and turns off the focus pause. After a 30 s warm-up it records every frame time and samples process RAM once per second. At the end it writes `frames-<time>.csv` and `summary-<time>.txt` to `<data>/bench/`, deletes the bench world and exits.
- The summary contains average FPS, 1% low (average of the slowest 1% of frames), p50/p99/max frame time, frames over 33 ms, RAM peak and average, the adapter and the render profile.
- `--gpu <NAME>` picks the adapter by name for one run; `--safe-mode` starts without packs (the bench always uses the built-in look).
- `scripts/stage1_exit.ps1` builds the release client, runs it 3 times on the Vega and once on the GTX 1660 Ti.
- Cloud check (VM, Xvfb + llvmpipe, 60 s): 333 blocks flown (≈ 5.5 blocks/s), RAM 353–374 MiB. That is software rendering, so the numbers say nothing about FPS; they only show the harness works.

## Laptop run (owner)

```powershell
git pull
powershell -ExecutionPolicy Bypass -File scripts\stage1_exit.ps1 -Seconds 300 -Runs 1   # quick check: one 5-min run per GPU, ~12 min
powershell -ExecutionPolicy Bypass -File scripts\stage1_exit.ps1                       # full: 3 × 30 min Vega + 1 × 30 min GTX 1660 Ti
```

Plug in the charger, close other programs, and don't touch the window during the run. Send the `summary-*.txt` files from `%APPDATA%\voxel-engine\bench\`. The start-to-menu time is measured separately: launch the game 3 times and note the time until the menu appears (stopwatch is fine).

## Acceptance criteria

- [x] 3 runs on the Vega 10 with all exit numbers reported; misses go to the owner (AGENTS.md). See [stage-1-exit-2026-11-27/summary.md](../../docs/research/stage-1-exit-2026-11-27/summary.md).
- [x] Client RAM row moves from Hypothesis to Measured.
- [ ] Stage 1 exit decision recorded by the owner.

## Out of scope

Network, LOD, mods.
