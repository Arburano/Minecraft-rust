# Lighting

Crate: `light`. ADR: 0011.

## Data

- Two 4-bit channels: block light and sky light. `Nibbles` = `Single(u8)` for uniform sections (all sky 15 / all dark 0) or 2,048 bytes.
- Light is stored separately from blocks, under the same `SectionPos` key.

## Who computes it

- The **client** computes light for rendering (the server does not send light — less traffic).
- The **server** computes light only where it affects gameplay (mob spawn, plant growth), lazily, with the same code.
- Same code + same inputs → same result. Test: client light hash = server light hash (T1).

## Algorithm

- Add: a BFS queue from the source, level −1 per step, taking block transparency into account.
- Remove: reverse BFS — clear everything lit by the removed source, collect the boundary with other sources, then refill from the boundary.
- Sky: start from the column's height map, then the same BFS under overhangs and into caves.
- Initial lighting of generated chunks is done in 2×2 column groups so that borders are computed once.
- Light tasks work on a section snapshot (3×3×3 around), and the region applies the result on the next tick. Light reaches ≤ 15 blocks, so with a region buffer of ≥ 1 section it does not leave the region.

## Limits

- A per-tick limit on light updates. The excess (TNT, mass building) is deferred to later ticks; the client temporarily shows the old light.
- Targets: incremental add ≤ 100 µs p95 per edit, removal ≤ 200 µs p95 (T1).

Measured (full recompute, VM): one section 25–45 µs; 3×3×3 sections with 8 torches — 1.4–2.2 ms; sky 3×3×3 — 0.55 ms; 10–35 ns per queue step.

Measured in T0-08 (VM, 4 vCPU Xeon 2.1 GHz, 3 runs; T0-12 corpus with 8 torches per section): incremental add ≤ 3.4 µs p95 (torch), ≤ 0.4 µs p95 (broken block); removal ≤ 16.8 µs p95 (torch), ≤ 5.2 µs p95 (placed block); synthetic worst cases (torch in a dark empty snapshot, cave roof under open sky) ≤ 24.5 µs p95; full recompute of 3×3×3, both channels, ≈ 0.9–1.2 ms p50. Details: [T0-08 report](../../task-reports/t0-08-light-incremental-bfs.md).

## Implementation (T0-08)

- `light::LightNeighborhood` holds a 3×3×3 snapshot (48³ cells) as a padded 50³ grid with a sentinel border of opacity 15, so the BFS steps by constant index offsets. Queues are reused; an edit does not allocate after warm-up.
- Block properties come from the caller as `LightProps { emission, opacity }` (registry data from mods); the engine knows no blocks. Opacity 0 = air/glass, 15 = fully opaque; light entering a block loses `1 + opacity`.
- Both channels solve the same equation: `L(c) = max(src(c), max over neighbors L(n) − 1 − opacity(c))`, clamped at 0, as the least fixed point inside the snapshot (cells outside count as dark). `src` is the emission for block light and 15 for the column's exposed sky run for sky light.
- **Sky run:** a column is open if the heightmap says it is open above the snapshot; inside the snapshot the run is the cells above the topmost block with opacity > 0 (leaves and water stop *direct* sky; light still reaches below them sideways and through them with attenuation). Edits move the run: placing a block in it shortens it (reverse BFS from the lost cells), removing the block that bounded it extends it (BFS from the gained cells).
- **Exactness:** light travels at most 15 blocks, so the **center section** of a snapshot is exact; the outer ring is a boundary approximation recomputed by the neighboring snapshot.
- API: `full_relight`, `relight_add` (changes that cannot darken: emission not lower, opacity not higher — one BFS from the cell; darkening changes are rejected), `relight_remove` / `set_block` (any change: reverse BFS clears what may have depended on the cell, collects the boundary, re-seeds sources, refills), `section_light` (compact `Nibbles` export).
- `light::reference::naive_light` is an independent relaxation (sweeps until stable). A proptest checks incremental = full recompute after every edit and = naive at the start and end of each case.

## Tests

- Torch on a section boundary: the incremental result is bit-for-bit equal to a full recompute.
- Torch removal and a broken block under the sky: bit-for-bit equality with a full recompute.
- Property test: random edits, compared with a naive full BFS.

## World integration (T1-09)

- `light::WorldLight` keeps light per `SectionPos`, apart from the blocks; `encode_column`/`decode_column` give the blob stored in `world::storage::ColumnRecord::light`.
- **Initial lighting** (`light_column`): every section is the exact center of its own snapshot built from the world's blocks (neighbors' blocks must be loaded, their light is not needed), so the result does not depend on the order columns are lit in — no seams by construction (tested against a full relight). Shortcuts: an all-opaque non-emitting section is dark; a section with all 27 neighbors empty and every heightmap value of the 3×3 columns at or below its bottom is open sky. This replaces the "2×2 column groups" idea: per-section snapshots make borders exact without grouping.
- **Edits** (`LightUpdates`): edits are recorded with the block's light properties before the edit and processed a limited number of sections per call (`max_sections`, the per-tick limit); the rest waits and the client shows the old light. Per section: a snapshot with the stored light imported and the surrounding one-cell layer **frozen** at the world's light; every still-pending edit inside it reverted to its old properties (so the stored light matches the blocks); this section's edits applied incrementally; only touched sections exported. Highest sections are processed first, so no pending edit lies above a snapshot.
- **Direct sky is not limited to 15 blocks:** a block placed over a deep shaft darkens it all the way down, so a pass checks how far down a column's direct-sky run moved (see T1-16 below).
- Tests: incremental world edits (3 seeds × 60 random edits, processed one section per "tick") equal a fresh initial lighting of the final world; breaking the roof of a 110-block shaft lights its bottom.
- **Owner decision (2026-11-22):** the world-path cost below is accepted for stage 1; [T1-16](../../tasks/stage-1/T1-16-light-fast-world-path.md) replaces it with a faster algorithm later, on the condition that the look and feel do not change.
- **Cost:** see T1-16 below and performance-budgets "Lighting through the world path".

## Faster world path (T1-16)

Same results, bit for bit (every T1-09 equality test unchanged and green, plus new tests below); less work per edit.

- **Box instead of the whole snapshot.** Light changes from an edit reach at most 14 cells. A pass loads blocks, imports stored light and freezes the surrounding layer only for the box of ±15 cells around its section's edits (31³ for one edit instead of 48³). Sections the box covers only partly are read cell by cell; the frozen layer is decoded once per section; the next load clears only the box written before. Exported sections merge the box with the stored light outside it.
- **When direct sky can move.** An edit whose opacity flips (0 ↔ > 0) at or above its column's sky height − 1 extends the box down to the snapshot's bottom. The heightmap counts exactly the blocks with opacity > 0, so no other edit can move a run. If a run nevertheless moves while the box does not reach the bottom, the pass is redone with the whole snapshot (a safety net).
- **What happens below.** The pass records the lowest local y whose direct-sky source changed (`sky_low`).
  - `sky_low` ≥ 15: nothing reaches the snapshot's bottom; done. The exact bound is 13 (a changed source at row 13 lights the border layer with 15 − 13 − 1 − 1 = 0 at best), and the rule keeps a margin of 2.
  - 1 ≤ `sky_low` < 15: the run ends inside the snapshot. Only a slab ±15 cells around the snapshot's bottom `B` can be wrong:
    - rows from `B + 14` up are exact, because the stale bottom border reaches 14 rows at most;
    - nothing below `B − 14` changes.
    
    The slab is recomputed from scratch for sky only (block light is unchanged below the pass's section), with its top face frozen at the exact new light and its other faces at the unchanged world light.
  - `sky_low` = 0: the run reaches the bottom and may continue to any depth. The 9 sections of each level below are recomputed as exact centers of their own snapshots, sky only, in parallel (rayon, one reusable snapshot per thread), until a whole level is unchanged.
- **Full relights queue only useful sources:** a source is queued only if it can raise a neighbor, which skips most open-sky cells.
- **Bug fixed on the way:** a decrease that reached the snapshot's bottom border made the BFS step outside the grid and panic. The new sky tests found it; neighbors outside the grid are now skipped.
- **Tests:**
  - `sky_edits_at_section_borders_equal_fresh_lighting`: 5 seeds, shafts of random depth, rooms and tunnels below them, roofs on section border rows, random batch sizes.
  - `roof_over_a_shallow_run_darkens_below_the_snapshot`: runs ending 1, 5 and 12 rows above the bottom.
  
  Both fail if the `sky_low` threshold drops to 12, if the box margin drops to 12, or if the slab is thinned (bottom at +6 or top at 21).
- **Not done yet:** persistent per-region light grids, which are the way to ≤ 100 µs and need an ADR, and a cheaper deep-run pass, which would chain slabs instead of 9 snapshots per level.
