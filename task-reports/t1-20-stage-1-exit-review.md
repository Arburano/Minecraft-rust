# T1-20: stage 1 exit review and stage 2 backlog

## What was done

- **Stage 2 backlog** (earlier commit `f33d8b3`): `tasks/stage-2/T2-01…T2-09` with waves in `tasks/README.md`.
- **Exit review** [`docs/research/stage-1-exit-2026-11-28.md`](../docs/research/stage-1-exit-2026-11-28.md), from the T1-15 laptop runs of 2026-11-27 (bare-metal Ryzen 7 3750H, Vega 10 3 × 30 min, GTX 1660 Ti 1 × 30 min, High 16 chunks, fly route over new terrain):
  - **Met:** average FPS 257.9–295.8 (target ≥ 60), 1% low 50.9–103.0 (≥ 45), RAM peak 324–346 MiB (≤ 1.5 GB; internal ≤ 1 GiB), start to menu 1.31–1.67 s (≤ 5 s).
  - **Deviation:** 36–189 frames > 33 ms per 30 min (target 0), max ≈ 115–120 ms on the Vega 10 and 94.6 ms on the GTX 1660 Ti.
  - Every other stage 1 budget row classified; open questions 1, 7, 12 and 15 carried over.
  - Recommendation: exit stage 1 with the spike target unchanged and a follow-up task.
- **New task** [T1-25](../tasks/stage-1/T1-25-frame-spike-attribution.md): per-frame attribution of spikes in the bench harness, a walking route at 12 chunks, and fixes.
- **Documents updated:** `performance-budgets.md` (FPS, frame spike, distance, Client RAM moved from Hypothesis to Measured, start-time T1-15 numbers), PRD §1 and §13.6 notes, roadmap, `tasks/README.md`, `docs/research/README.md`.
- No code changed and no new benchmark was run in this task; all numbers come from the owner's T1-15 runs.

## What went wrong

- The first draft listed autosave with fsync as a spike candidate. Checking the code showed that `world.toml` is written without fsync and the route edits no columns, so it was rewritten as a weak candidate.

## Issues encountered

- **The owner's exit decision is pending**: T1-20 and T1-15 stay `in-progress` until it is recorded in the roadmap and the review.
- **Spike causes are not attributed.** The harness writes only frame times, and the per-frame CSVs were not committed. The causes in the review are candidates from reading `App::frame` and `Session::update`, not measurements. T1-25 is there to measure them.
- **The runs used 16 chunks and a fly route**, not "12 chunks, walking" as the criterion says. That is a heavier load, so FPS and RAM pass at 12 as well. Spikes have not been measured on the literal criterion; T1-25 adds that route.
- **Vega run 3** (1% low 50.9, p99 16.6 ms vs 6–8 ms in runs 1–2) is probably thermal, as it ran after one hour of load, but that is not proven.
- **Open question 1** (web client) was due "before the end of stage 1" and needs the owner's confirmation.
