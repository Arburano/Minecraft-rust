# T1-09: lighting integrated with the world

Status: implemented and tested. The per-edit budget through the world path is **not met**; the owner decides how to proceed (options below). The task stays `in-progress` until then.

## What was done

- **`light::world_light`:**
  - **`WorldLight`:** light per `SectionPos`, kept apart from the blocks. `encode_column` / `decode_column` produce the blob stored in `ColumnRecord::light` (T1-02), with a presence byte per section and 2-byte single or 2,049-byte packed channels.
  - **`snapshot_from_world` / `reload_from_world`:** a 3×3×3 snapshot from the `WorldMap`.
    - Uniform sections are loaded without unpacking; sections above the world count as air, below the world as opaque.
    - "Open to the sky above" comes from the world heightmap.
    - Optionally the stored light is imported and the surrounding one-cell layer is frozen at the world's light: border cells get light and source equal to the stored value, so a decrease pass can never erase light coming from outside.
    - A neighbor without light is an explicit `NotLit` error.
  - **`light_column`:** initial light per section; each section is the exact center of its own snapshot, so the result does not depend on lighting order and has no seams. Shortcuts: an opaque non-emitting section is dark; an open-sky section is skipped. It returns statistics.
  - **`LightUpdates`:** a queue of edits with each block's light properties *before* the edit. `process(.., max_sections)` is the per-tick limit:
    - the highest section is processed first;
    - every still-pending edit inside the snapshot is reverted to its old properties, so the stored light matches the blocks;
    - this section's edits are applied incrementally and only the touched sections are exported;
    - an edit that changes direct-sky passage recomputes the 3×3 sections below, level by level, until a level is unchanged.
  - `props_from_registry`: a precomputed table of light properties.
- **`LightNeighborhood`** (crate-internal additions): in-place reload (`begin_load`, `load_uniform`, `load_ids`, `set_open_top`), `import_section`, `freeze_border`, `set_props_raw`, `rebuild_sky_runs`, and touched-section tracking (marked from the BFS queues, not inside the hot loop). `full_relight` now seeds every cell with a source, including frozen border cells.
- **`examples/edit_profile`:** a phase timing of one world-path edit (diagnostics).
- **Docs:** `lighting.md` ("World integration"), and a new `performance-budgets.md` row marked as a deviation.

**Tests** (`crates/light/tests/world_light.rs`, on worldgen terrain with bounds y 0..128):
- The initial-lighting shortcuts equal a full relight for every section of 5×5 columns; both the shortcut and the BFS paths are exercised.
- 3 seeds × 60 random edits (air, stone, torch, glowstone, glass, water near the surface and in caves), with `process` at one section per call interleaved with editing. The final light of all sections in the 5×5 columns equals a fresh initial lighting of the final world.
- Opening the roof of a 110-block shaft lights its bottom (sky 0 → 15), and the column equals a fresh lighting.
- The storage blob round-trips, a truncated blob is rejected, and an unlit neighbor gives `NotLit`.
- The T0-08 unit tests and proptests still pass.

**Mutation checks:**
- Removing the recomputation below the snapshot is caught (shaft test).
- Removing the revert of pending edits is caught (random edits and shaft).
- Removing the frozen-border sources is **not** caught. An edit in the center section darkens at most 15 blocks around it, and the snapshot border is at least 17 blocks away. Only a direct-sky change reaches the bottom border, and that level is recomputed exactly right after. The freeze therefore makes the exported ring sections exact before that recomputation, but no test currently depends on it.

**Benchmark** (`cargo bench -p light --bench world_light`): cloud **VM**, 4 vCPU Xeon 2.1 GHz, 3 runs, 192 edits per kind per run. Real generator (seed 7), bounds y −64..320, 9×9 columns loaded and 7×7 lit.

| Measure | First version | After reuse/fast paths |
| --- | --- | --- |
| initial light per column (p50) | 7.2–8.0 ms | 5.4–5.6 ms |
| torch place/remove (p50 / p95) | 1.08–1.24 / 1.20–1.90 ms | 0.66–0.94 / 0.71–1.16 ms |
| stone on the surface, sky change (p50 / p95) | 13.1–13.8 / 15.3–20.6 ms | 9.1–9.2 / 12.8–15.5 ms |

Out of 1,176 sections (49 columns × 24): 9 opaque shortcuts, 617 open-sky shortcuts, 550 by BFS.

## What went wrong

- **First design flaw, found before testing:** "an edit changes at most 15 blocks" is false for sky light, because direct sky runs down a column without limit. Added the recomputation below the snapshot and top-down processing; the shaft test covers it.
- **The world path is about 7× (torch) to 70× (sky change) over the per-edit budget.**
  - Profiling one edit (allocating path): snapshot build ≈ 520 µs, import and freeze ≈ 330 µs, exporting 27 sections ≈ 190 µs, while the BFS itself is ≈ 10–20 µs.
  - Optimizations so far: reused snapshot (no allocation per section), face-only border freeze, fast paths for uniform sections, a precomputed props table, exporting only touched sections. They gave about −35–45%, not the needed 10×.
- **Clippy:** `double_must_use` and a complex type alias (fixed).

## Issues encountered

- **Decision for the owner** (AGENTS.md: a target unreachable with the current architecture). Options:
  1. **Accept** the world-path cost for stage 1. Light updates are deferred and capped per tick (the client shows old light briefly), so they do not block frames if processed in a background job. The ≤ 100/200 µs budget would then apply to the BFS only.
  2. **A smaller edit window:** a 32³ box around the edit instead of the 48³ section-aligned snapshot, about 3.5× fewer cells to build and import.
  3. **Persistent per-region light and opacity grids** instead of snapshot copies, with the BFS working in place. This is the likely way to reach ≤ 100 µs, but it is a storage and architecture change (lighting.md / ADR-0011 territory).
  4. **Incremental vertical sky propagation** instead of exact per-level recomputation, for sky-changing edits.
- **Spec deviation:** "initial lighting in 2×2 column groups" is replaced by exact per-section snapshots (described in lighting.md). The result has no seams and does not depend on order.
- **Light is not saved through `WorldStore` yet**, and processing does not run as a job yet. The blob format and the API are ready; T1-10/T1-13 connect them.
- VM numbers only.
