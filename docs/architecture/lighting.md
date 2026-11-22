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
- **Direct sky is not limited to 15 blocks:** a block placed over a deep shaft darkens it all the way down. An edit that changes whether a cell passes direct sky therefore recomputes the 3×3 sections below (from the snapshot's bottom ring downwards, exact full snapshot BFS) until a whole level is unchanged.
- Tests: incremental world edits (3 seeds × 60 random edits, processed one section per "tick") equal a fresh initial lighting of the final world; breaking the roof of a 110-block shaft lights its bottom.
- **Cost (VM):** the BFS itself stays within budget, but each processed section rebuilds a 48³ snapshot from the world (≈ 0.5 ms build + ≈ 0.3 ms import) — see performance-budgets "Lighting through the world path". Faster designs (a smaller window around the edit, persistent per-region light grids instead of snapshot copies, incremental vertical sky propagation) are listed in the T1-09 report for the owner's decision.
