# World generation

Crate: `worldgen`.

## Pipeline

```text
heightmap+biomes (cheap, 2D) ─► terrain ─► caves ─► ores ─► structures ─► decorations ─► initial light
```

- Each stage is a function `fn(&GenContext, &mut ProtoColumn)`; stages are registered by mods (`core` registers vanilla-like ones). A mod can add a stage or replace a stage by id.
- Stages that touch neighbors (structures, trees) require neighboring columns to have passed the previous stage — like chunk "statuses" in Minecraft. The generation scheduler holds a dependency graph.
- Generation is deterministic from seed and coordinates; task execution order does not affect the result.
- Noise: `noise` for the start; hot functions get a SIMD implementation after profiling.

## Game zones (PRD §14, No. 11)

- Static map layout: zone = (biomes, difficulty, spawn tables, dungeon and structure tables).
- Zone tables are compiled into fast lists when the world loads. The spawn system takes the list of the zone under a position and does not check others.
- Zone assets (mob models, sounds) are loaded by the client on approaching the zone.
- Zone ≠ threading region. A mob can cross a zone border, so its behavior works everywhere; only spawning is restricted.

## Generation range

- Only what is needed is generated: players' simulation distance + the LOD request. A priority queue (movement direction, height, speed — PRD §14, No. 4).
- World pre-generation is a standard server command; recommended in production, because on-the-fly generation is the most expensive part of the load.
- A cheap heightmap+biomes pass provides data for far LOD before full chunks are generated.

## Tests

- Determinism: one column generated in different orders and with different thread counts gives the same hash.
- Speed: a column ≤ N ms per core (the number comes after the stage 1 benchmark).

## Test corpus (T0-12)

`worldgen::test_corpus` is the fixed synthetic terrain for benchmarks and tests
(`docs/process/testing-and-benchmarks.md`). It is **not** the game's final
generator; the pipeline above replaces it in later stages.

- API: `test_corpus(seed, &CorpusArea) -> Vec<TestColumn>` and
  `test_column(seed, x, z, y_range) -> TestColumn`. `CorpusArea` is in section
  coordinates (end-exclusive ranges); a `TestColumn` is a stack of
  `world::Section` over the requested y range. `TestColumn::hash()` is an
  FNV-1a 64 over the coordinates and every block ID in a fixed order.
- Terrain: 4-octave Perlin heightmap (base 36, amplitude 16, base frequency
  1/128), sea level 44, 3D-noise caves, deterministic 2×2×2 ore clusters in
  stone (coal everywhere, iron below y 24), trees (24 candidates per 32×32
  cell). Transparent blocks (water + leaves) are ~8% of non-air blocks, matching
  the `research/bench/meshing` starting point.
- Corpus-local test IDs (`test_corpus::ids`): 0 air, 1 stone, 2 dirt, 3 grass,
  4 sand, 5 water (T), 6 leaves (T), 7 glass (T, edit benchmarks only), 8 coal
  ore, 9 iron ore. Opacity for meshing is `test_corpus::is_transparent`; these
  IDs are not registry semantics.
- Determinism: every column is a pure function of `(seed, x, z, y_range)`.
  Trees never leave their 32×32 placement cell (trunks at cell offset 3..=28,
  leaves reach 2 blocks), so columns need no neighbor data; generation order
  and thread count cannot change the result. Covered by
  `crates/worldgen/tests/determinism.rs` and `corpus_proptest.rs`.
- Users: `world` bench `section_corpus` (T0-03), `mesh` benches `mesh_chunk`
  and `render_batch` (T0-04/05), and the future `light` benchmark (T0-08) —
  all with fixed seed 0.
