# T1-25. Frame spike attribution and fix

Numbered T1-25 because T1-21 … T1-24 are taken on the unmerged branch `ccr-1493f683-03fpg7` (render reference scene, render graph, mesher trait, modular base game).

- **Status:** in-progress (cloud part 1 done: attribution and walking route; owner: laptop runs; then fixes)
- **Depends on:** T1-15
- **Owner:** agent (cloud) writes the instrumentation and fixes; owner's laptop runs the route

## Goal

The stage 1 exit runs (T1-15, [review](../../docs/research/stage-1-exit-2026-11-28.md)) met FPS, 1% low, RAM and start time, but showed 36–189 frames over 33 ms per 30 minutes (max ≈ 95–120 ms on both GPUs). Find out what each of these frames was doing, remove the main-thread causes, and show a 30-minute run without frames over 33 ms. The budget row "Frame spike: none > 33 ms over 30 min of walking" is unchanged.

## Specs

- [performance-budgets.md](../../docs/process/performance-budgets.md) — Frame spike
- [stage 1 exit review](../../docs/research/stage-1-exit-2026-11-28.md) — candidate causes
- [T1-15](T1-15-stage-1-exit-harness.md) — the bench route and script
- [T1-06](T1-06-profiling-and-ci-benchmarks.md) — profiler zones and Tracy

## Deliverables

- **Per-frame attribution in the bench harness:** for every frame, the time spent in `Session::update` (loader tick, saved-column source calls, light processing, autosave), fixed ticks, UI, `render_updates` (batches and light sections uploaded, bytes written), surface acquire and present. Frames over 33 ms are written with these counters to `spikes-<time>.csv`; the summary lists the top causes. No allocation per frame (preallocated buffers, as the frame log is today).
- **Walking route:** `--bench-route` gets a walking mode at the default 12 chunks (ground movement at sprint speed, 5.6 blocks/s), so the run matches the criterion literally; the fly route at 16 chunks stays as the stress case.
- **Fixes** for the main-thread causes the attribution finds (for example moving saved-column reads to a worker, spreading uploads over frames, an upload byte budget per frame). Each fix with a before/after run.
- A Tracy capture of one spike on the laptop, if the attribution is not conclusive.
- `scripts/stage1_exit.ps1` gains the walking mode.

## Progress (2026-11-28)

- Done (cloud): per-frame attribution (`FrameCost`, `UpdateCost`, `SyncCost`), `spikes-<time>.csv`, the "spike causes" lines in the summary, `--bench-mode walk|fly`, `scripts/stage1_exit.ps1 -Mode`. Report: [t1-25](../../task-reports/t1-25-frame-spike-attribution.md).
- Done (laptop quick check, 300 s × 1): `walk` + `fly` attribution recorded in [t1-25-2026-11-28/summary.md](../../docs/research/t1-25-2026-11-28/summary.md) (`upload_light` identified as main internal spike cause; GTX 1660 Ti hit ~33 ms `present` pacing).
- Next (owner's laptop, optional full run): `powershell -File scripts\stage1_exit.ps1` (walk, 3 × 30 min Vega 10 + 1 × 1660 Ti) and `-Mode fly -Runs 1`.
- Done (cloud, fix 1): the `upload_light` spikes are **removals** (744–936 sections of a column row leaving the view, 0 uploads in every such frame), each a separate 1-texel `write_texture`. The renderer now keeps a CPU copy of the light indirection and uploads changed z slices once per frame (≤ 64 writes, a row along x is 1). VM before/after: 1.2 ms → 0.026 ms for 936 removals + prepare; golden images unchanged.
- Done (cloud): `--no-vsync` and `scripts\stage1_exit.ps1 -NoVsync`; the summary names the present mode. The GTX 1660 Ti's ≈ 33 ms `present` is display pacing (switchable graphics), not engine work.
- Next (owner's laptop): `scripts\stage1_exit.ps1 -Seconds 300 -Runs 1 -Mode fly` and `-Mode walk` on the current `main` (now with the embedded server, T2-08), with and without `-NoVsync`.

## Acceptance criteria

- [ ] Every frame over 33 ms in a laptop run is attributed to a named phase, or shown to be outside the engine (time between frames with no engine phase over budget).
- [ ] 3 × 30 min walking runs at 12 chunks on the Vega 10 with 0 frames over 33 ms, or the remaining spikes are explained and the decision goes to the owner.
- [ ] The fly route at 16 chunks reported before and after.
- [ ] `performance-budgets.md` Frame spike row updated.

## Out of scope

Network (stage 2), LOD, new gameplay.
