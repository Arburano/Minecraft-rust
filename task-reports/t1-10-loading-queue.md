# T1-10: chunk loading queue with priorities

Status: implemented and measured on a VM. Open: the owner confirms the proposed "nearest 3×3 ≤ 0.5 s" number, and the run on the owner's laptop is pending.

## What was done

- **`client::loading`** (the `client` crate gains a library target, `src/lib.rs`):
  - **`Viewer`:** priority = 3D distance discounted ahead of the horizontal movement direction. The discount grows with speed (none when standing, at most 50% from 10 blocks/s). Columns use horizontal distance, mesh chunks include height.
  - **`Loader::tick`:** cancels jobs and unloads columns, light and meshes beyond the kept area; applies finished results within `apply_budget`; builds candidates (generate within radius + 2, light within radius + 1 once the 3×3 neighbors exist, mesh chunks whose 2×2 core is lit and 4×4 blocks exist); sorts by priority and submits until `max_in_flight` or `tick_budget` is reached.
  - **Jobs** get copies, never the main thread's map:
    - generation: `Pipeline::generate` + `into_column`;
    - light: `light_column` on a private 3×3 `WorldMap`;
    - mesh: `MeshChunkBuilder::build` on a cloned `SectionNeighborhood`.
  - All-air mesh chunks get an empty mesh immediately, without a job. The pool class (`Visible` / `Prefetch`) follows distance, not job kind.
  - `is_ready(column)`: lit, and all its mesh chunks exist. `LoadStats` counts submitted, cancelled and applied jobs and records the maxima of apply, tick, scan, single submit and unload.
- **`engine_core::jobs::JobPool::with_low_priority_workers`:** workers below normal OS priority (best effort, `thread-priority` already in the workspace).
- **`world::WorldMap::column_positions`:** iterator over loaded columns, used for unloading.
- **`examples/flythrough`** (spawn, then 10 s of flight at a given speed in real time, 60 Hz ticks): it measures and prints the readiness times, main-thread maxima, cancellations and lag.
- **Docs:** `world-storage.md` (loading), `engine-core.md` (low-priority workers), two new `performance-budgets.md` rows.

**Tests:**
- Unit: nearer first, ahead before behind, faster movement prefers the front more, height for mesh chunks.
- Integration (`tests/loading.rs`, worldgen terrain, y 0..128, radius 3):
  - the area becomes ready, and the light computed by the jobs equals `light_column` on the whole map for the 5×5 inner columns; meshes exist with geometry;
  - a teleport of 100 columns while jobs are in flight cancels jobs and unloads every old column, its light and its meshes;
  - a 1 ns apply budget still makes progress.

**Measurements** (`cargo run --release -p client --example flythrough -- 12 3 20`): cloud **VM**, 4 vCPU Xeon 2.1 GHz, 3 worker threads at low priority, radius 12, bounds y −64..320, seed 7. Final configuration, 3 runs:
- spawn: nearest 3×3 ready **0.47–0.49 s**; full radius 12 (625 columns) ready 2.0–2.2 s; 2,410 jobs;
- apply batch max 0.91–1.46 ms (target ≤ 2 ms); whole tick max 2.2 ms at spawn;
- flying at 20 blocks/s for 10 s: tick max 2.4–3.5 ms; never a non-ready column within 2 columns of the viewer; 0 cancellations (nothing in flight leaves the kept area at that speed; the cancellation path is covered by the teleport test).

## What went wrong

1. **Unreachable readiness:** a mesh chunk spans 2×2 columns, so border columns of the radius needed light and blocks beyond it. The loaded rings are now radius + 1 (light) and radius + 2 (blocks).
2. **5.5–9 ms main-thread ticks:**
   - First suspected the copies of job inputs, but measuring them separately showed they are cheap: `WorldMap::new` ≈ 9 µs, 9 column clones ≈ 16 µs, the mesh neighborhood ≈ 7 µs.
   - The stalls were rare preemptions: 3–4 busy workers plus the main thread on 4 vCPUs.
   - Low-priority workers brought the tick max from 5.5–9 ms down to 2.2–2.7 ms.
3. **3×3 readiness took ~1.6 s, about as long as the whole radius:**
   - Meshes were submitted as `Prefetch` and waited behind all `Visible` generation.
   - After switching the class to follow distance, the delay remained: the mesh chunks high in the sky (far by height) still queued behind far generation.
   - Skipping all-air mesh chunks without a job brought it to 0.47–0.49 s and cut the jobs from 3,598 to 2,410.
4. **Clippy:** too many arguments, an unused variable, `map_or` → `is_some_and`. All fixed.

## Issues encountered

- **Owner decision:** confirm "nearest 3×3 ready ≤ 0.5 s". It is met on the VM, with little margin. The laptop run is pending (`cargo run --release -p client --example flythrough -- 12 7 20` with 7 workers on the 8-thread laptop).
- **The tick budget is soft:** the last submission can overshoot it (the tick max was 3.5 ms once while flying). Making it hard would need splitting input copies; the apply budget itself holds.
- **Not wired yet:**
  - loading from and saving to `WorldStore` (columns are always generated; T1-13 decides the save directory);
  - edit relighting through `LightUpdates` (T1-12);
  - mesh upload to the GPU (T1-11).
- **The mesh job does not use light yet:** the quads carry no light; T1-11 adds it in the shader.
- VM numbers only.
