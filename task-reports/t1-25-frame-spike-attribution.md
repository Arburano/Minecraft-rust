# T1-25: frame spike attribution and fix — part 1 (attribution and walking route)

## What was done

- **Per-frame attribution** in the client (`crates/client`):
  - `session::UpdateCost` (library): `Session::update` now times the loader tick, the saved-column reads it makes on the main thread (time and count), relighting, edited-column bookkeeping, unloaded columns and the autosave. `Session::last_update_cost()` reads it. Always on; it costs a few `Instant::now()` calls per frame.
  - `view::SyncCost`: `WorldView::sync` returns the time for batch collection + upload and for light atlas writes, plus batches, quads, light sections and removed sections.
  - `bench::FrameCost`: `App::frame` times update, fixed ticks (and their count), UI, sync, prepare, surface acquire, encode, submit and present. Each frame interval is attributed to the work of the frame that started it; the rest is reported as `outside` (event handling, OS, driver outside our calls).
  - Every frame over 33 ms goes into a preallocated buffer (8,192 spikes; later ones are only counted, no allocation while measuring). `finish` writes `spikes-<time>.csv` (each phase in ms, the dominant phase, the counters), and the summary gains "spikes attributed", "spike causes (longest phase)" and "mean phase over spikes".
- **Walking route:** `--bench-mode walk` sprints on the ground (≈ 5.6 blocks/s), jumps when blocked or in water, turns 90° when it makes less than 2 blocks of progress in 3 s, and turns at the end of each 1000-block side. It runs at the Default profile (12 chunks), so it matches the exit criterion literally. `--bench-mode fly` (the default) is the T1-15 route; it now sets the High profile (16 chunks) itself instead of using the saved settings. The laptop runs already used High, so the numbers stay comparable. Neither override is saved.
- `scripts/stage1_exit.ps1` gains `-Mode walk|fly` (default `walk`) and mentions the spikes CSVs.
- Task renumbered from T1-21 to **T1-25**: T1-21 … T1-24 are already used on the unmerged branch `ccr-1493f683-03fpg7`. References in the exit review, `performance-budgets.md`, PRD, roadmap and the T1-20 files were updated.
- Tests: 6 new unit tests in `bench.rs` (phase split and `outside`, `outside` never negative, cause counting, spike recording and CSV shape, walking jump rule, route profiles) and `update_reports_where_its_time_went` in `tests/session.rs`. `cargo test -p client` passes (lib 7, bin 8, integration 4 + 3 + 3 + 4). `cargo clippy -p client --all-targets` with `-D warnings` is clean. `cargo fmt --all` was applied.
- No benchmark numbers yet. The route needs a window and a GPU. This container has neither a GPU nor the X11 keyboard library winit needs (the run under `xvfb-run` panicked in `xkbcommon` at event-loop creation), so the route was not run here.

## What went wrong

- The smoke run of `--bench-route 20 --bench-mode walk` under Xvfb failed before opening a window (no `libxkbcommon-x11`, no GPU). Not fixable in the cloud container; the laptop runs are the real check.

## Issues encountered

- **Duplicate and unmerged work in the repository.** `main` still lists T2-01 … T2-08, T1-17 and T1-18 as `todo`, but the branch `ccr-1493f683-03fpg7` (no PR) has implementations of all of them plus T1-21 … T1-24 and T2-10. Open PRs #38 (T2-01) and #39 (T2-01 + T2-02) are separate implementations of the same tasks, and `ccr-b8153911-awpqgm` has a third T2-01. Scheduled runs that start from `main` keep picking the same tasks again. The owner should decide which lineage to merge. This branch is based on `ccr-b8153911-v42apu` (the T1-20 exit review), which is based on the current `main`.
- The walking route has not been checked for getting stuck in real terrain (cliffs, lakes). The 90° turn when stuck should keep it moving. The summary's "blocks walked" shows whether it covered the expected ≈ 10,000 blocks per 30 min.
- **Fixes are not done.** The task asks for fixes of the causes the attribution finds, each with a before/after run, so they wait for the laptop CSVs. The candidate causes are still those in the [exit review](../docs/research/stage-1-exit-2026-11-28.md).
- The `performance-budgets.md` Frame spike row is unchanged until there are new runs.

## Follow-up: laptop results and fix 1 (2026-11-28)

**Laptop quick check** (300 s × 1 per row, `adfbfc0`, i.e. before the embedded server of T2-08; [summary](../docs/research/t1-25-2026-11-28/summary.md)):

| Run | Avg / 1% low FPS | Frames > 33 ms | Main cause |
| --- | --- | --- | --- |
| Vega 10, walk, 12 chunks | 311 / 116 | 3 | isolated (ui, encode, outside) |
| Vega 10, fly, 16 chunks | 97 / 30 | 143 | `upload_light` 73, `acquire` 46 |
| GTX 1660 Ti, walk | 44 / 24 | 6,973 | `present` ≈ 33 ms almost every frame |
| GTX 1660 Ti, fly | 31 / 17 | 7,863 | `present`, plus `upload_light` 103 |

**What the CSVs show:**

- **`upload_light` spikes are removals.** In every such frame `light_sections` is 0 and `removed_sections` is 744–936: a row of 31–39 columns × 24 sections leaving the view. `remove_section_light` wrote one 4-byte texel with its own `queue.write_texture` per section. On DX12 that costs 30–66 ms for ~900 calls.
- **`present` on the GTX 1660 Ti** stays at ≈ 33 ms at p50, with only 1–3 ms of engine work per frame. This is display pacing with vsync on a switchable-graphics laptop (the dGPU's frames go out through the iGPU), not scene cost.
- **`acquire` on the Vega 10 at 16 chunks** is the GPU being busy: the frame waits for a free swapchain image.

**Fix 1** (`render::terrain::renderer`):

- `TerrainRenderer` keeps a CPU copy of the light indirection (64 × 32 × 64 entries, 512 KiB) and a dirty bit per z slice. `set_section_light` and `remove_section_light` only change the copy.
- `flush_light_index` (called by `prepare`) uploads each changed z slice once: 8 KiB, one `write_texture`. A row of columns along x is one write; along z, at most one per column. 64 writes is the hard maximum.
- Light changes still show in the next frame drawn, as before.

**Measured on the cloud VM** (llvmpipe, `cargo run --release -p render --example light_churn`, 936 sections removed + `prepare`, 20 rounds × 3 runs):

| | Median | Max |
| --- | --- | --- |
| Before | 1.21–1.64 ms | 2.0–2.2 ms |
| After | 0.022–0.030 ms | 0.033–0.061 ms |

These numbers show the call overhead only. The real effect is on DX12 on the laptop, and its re-run decides.

**Tests:**

- `render/tests/light_index.rs`: a row of 936 removals is 1 write, an unchanged flush is 0 writes, two rows give 11 writes.
- The T1-21 golden images are unchanged on all paths.

**Vsync:**

- `client --no-vsync` (`AutoNoVsync`) and `scripts\stage1_exit.ps1 -NoVsync`.
- The bench summary's GPU line now says `vsync` or `no vsync`.
- The default stays vsync: that is what players get. The no-vsync run shows the engine's own cost on the GTX 1660 Ti.

**Still open:**

- the laptop re-run on the current `main` (the embedded server changes the update phase);
- the Vega 10 `acquire` waits at 16 chunks (a GPU-load question for the High profile, not a stall);
- the 7 `generation` spikes in the fly run (main-thread generation work around new columns), to look at after the re-run.
