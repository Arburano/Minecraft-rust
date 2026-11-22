# T1-03: terrain generator pipeline

## What was done

- **`worldgen::pipeline`:**
  - `GenContext`: seed, resolved config, bounds, Perlin noises; `surface(wx, wz)` gives the 2D height and biome; `rng(key)` gives seeded random streams.
  - `ProtoColumn`: a flat section-major block array plus surface data; `into_column`; `hash` for determinism checks.
  - The `Stage` trait and `Pipeline` with `overworld()`, `replace`, `insert_after`, `generate`, and `generate_parallel` (`Visible` jobs on the T0-02 `JobPool`, results sorted by position).
- **`worldgen::stages`:** `core:surface`, `core:terrain` (stone, biome top and filler, beaches, sea water), `core:caves` (3D Perlin on a 4-block grid with trilinear interpolation, kept 6 blocks under the surface), `core:ores` (random-walk veins inside the column), `core:trees`. Trees with origins in the column and its 8 neighbors are each written only into the column's own cells, so trees across borders match from both sides.
- **`worldgen::config` and `mods/core/data/worldgen/overworld.toml`:** all block names and parameters come from mod data and are resolved through the registry; an unknown block is a load error. Three biomes (plains, forest, desert) by a temperature noise. The engine code names no blocks.
- **`world::section::build_from_array`** (used by `Section::from_blocks`) was rewritten: one pass with a last-value cache, a linear scan for palettes of ≤ 16 entries and a hash map only above that; words are packed in bulk. The palette order (first seen) and the output are unchanged; the golden tests still pass.
- **Examples:** `map_preview` (top-down BMP map) and `stats` (biome shares, sea share, tree count).
- **Docs:** `worldgen.md` (implementation, speed, proposed target) and a new `performance-budgets.md` row.

**Tests** (`crates/worldgen/tests/pipeline.rs`):
- **Determinism:** 16 columns generated in forward and reverse order and on pools of 1, 2 and 4 threads give identical hashes; a different seed gives a different hash.
- **Trees across borders:** for every trunk in a 6×6-column area, every crown cell that was air before the tree stage (the same pipeline with `core:trees` replaced by a no-op) is leaves or log, including cells in neighboring columns. The test also requires that at least one tree actually crosses a border.
- **Stage editing:** replace and insert, and the order of stage ids.
- **Config:** an unknown block name gives `UnknownBlock`.
- **Plausibility:** stone, water, air, leaves, both ores, grass and sand occur; surface heights are in bounds; more than 1,000 cave air blocks are underground.
- **Mutation check:** placing trees only from the column's own origins is caught ("crown hole at …").

**Benchmark** (`cargo bench -p worldgen --bench pipeline`): cloud **VM**, 4 vCPU Xeon 2.1 GHz, 20 samples. Seed 7.

| Measure | Before the `from_blocks` rewrite (3 runs) | After (2 runs) |
| --- | --- | --- |
| column, all stages (1 thread) | 0.28–0.33 ms | 0.33–0.37 ms |
| surface pass, 256 block columns | 12–15 µs | 12–15 µs |
| pack into palette sections | 1.46–1.66 ms | 0.26–0.30 ms (Criterion baseline: −79%) |
| 64 columns, 4 threads (generation only) | 6.0–6.8 ms | 6.7–7.8 ms |

Generation alone barely changed between the runs; the "after" generation numbers sit at the noisy end.

A preview of seed 7 (512×512 blocks) shows lakes, beaches, forests with trees and a desert. It is not committed: it is reproducible with the example.

## What went wrong

- **Biome thresholds:** the first ones (0.15 / 0.45 / 1.0) assumed temperature spread over 0..1. Perlin·0.5+0.5 mostly stays within 0.25..0.75, so plains never appeared and desert covered about 70%. They were retuned to 0.47 / 0.6 / 1.0 (about 38% plains, 43% forest, 19% desert over 64 columns × 4 seeds), and forest density was raised to 3 trees per column.
- **Tree origins** were first limited to 2..14 inside a column, so no crown ever crossed a border and the cross-column logic was never exercised. The test's "at least one border tree" guard caught it; origins now cover 0..16.
- **Packing was 5× slower than generating:** `from_blocks` did a SipHash `HashMap` lookup per block. It was rewritten (see above).
- Small compile and edit slips: an untyped integer in `abs`, and a bench file written before its directory existed.

## Issues encountered

- **Speed target for the owner:** worldgen.md wanted "≤ N ms per column per core" from this benchmark. Proposed: **≤ 1 ms including packing**, about 1.5× margin over the VM numbers. It should be confirmed on the laptop.
- **Deviation from worldgen.md** (described in the doc, no ADR involved): no chunk-status dependency graph. Cross-column features are placed by every column they touch. A graph is needed only when a stage must read a neighbor's generated blocks.
- **Biome borders are sharp** (no blending), there is only one cave shape, there is no bedrock block (`bedrock_depth` is reserved), and ore veins stay inside one column. Fine for stage 1; game zones and richer biomes are stage 4.
- **Initial light** is not a stage yet (T1-09 adds lighting in 2×2 column groups), and generated columns are not written to disk yet (T1-10/T1-13 connect `WorldStore`).
- VM numbers only.
