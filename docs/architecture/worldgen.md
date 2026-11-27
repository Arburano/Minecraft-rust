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
- Speed: a column ≤ N ms per core (the number comes after the stage 1 benchmark). Measured in T1-03 (VM, 4 vCPU Xeon 2.1 GHz): all stages of a 16×384×16 column ≈ 0.28–0.37 ms per core, packing into palette sections ≈ 0.26–0.32 ms, 64 columns on 4 threads ≈ 6–7.8 ms. **Target: ≤ 1 ms per column per core including packing** (confirmed by the owner 2026-11-22: do what is reasonable to reach it, but not at the cost of rewriting the project).

## Generation modes (owner decision 2026-11-22)

- **Normal** — the T1-03 pipeline below with the `classic` engine.
- **Extended** — a richer generator of our own (multi-noise biomes, height splines, smooth blending, more caves and biomes), task [T1-17](../../tasks/stage-1/T1-17-worldgen-modes.md). The first part is done: the `climate` engine and the `overworld_plus` module (see "Engines"). The mode is chosen by the build (ADR-0017), and the world remembers its generator.
- **Reference for "extended": Tectonic** (Apollo, MIT on Modrinth; replaces Terralith, which is not used because its license forbids copying and AI use). Agents may study it and port ideas and parameters with credit in `THIRD_PARTY.md`; verbatim copies wait for a published LICENSE with a copyright line (the GitHub repository has none yet). Vanilla-derived data in it stays out of the repository, and our density-function-style generation is written from public documentation. Details: [T1-17](../../tasks/stage-1/T1-17-worldgen-modes.md).

## Engines (T1-17)

A **landscape engine** decides the shape of the land: height and biome of every block column (`GenContext::surface`, used by every stage, by trees in neighboring columns and by far LOD). Everything after the shape is shared by all engines: surface layers, caves, ores, trees. A world-generation module picks its engine with `engine = "..."` in its `worldgen/<dimension>.toml`, so an advanced module replaces **only the generation** (and adds the blocks it needs), and a build picks the module.

| Engine | Shape | Biomes | Used by |
| --- | --- | --- | --- |
| `classic` | one hills noise + detail around `base_height` | by temperature noise (`max_temperature`) | `overworld` (build `classic`) |
| `climate` | continentalness (oceans → coasts → inland), erosion (flat vs mountainous), ridges (peaks and valleys) through height splines; mountains fade in from the coast (`inland`) | first biome in file order whose climate ranges hold: temperature (cooling with height above the sea, so peaks are snowy), humidity, continentalness, surface height; a biome without ranges catches the rest | `overworld_plus` (build `classic_plus`) |

- Height depends only on the climate noises, never on the biome, so biome borders never make cliffs (a test checks that neighbors differ by ≤ 6 blocks over 12 × 12 km).
- **Tree kinds:** `[trees]` is the default kind (`"default"`); `[[tree_kinds]]` add named kinds with their own log, leaves, trunk heights and crown `shape` (`round` like oak and birch, `cone` like spruce); a biome lists the kinds that grow in it (`trees = ["spruce"]`), one picked per tree. Crowns stay within a radius of 2, so the cross-border rule below still holds (tested per kind).
- **Determinism:** both engines are pure functions of `(seed, position)`; the classic engine's output is bit-identical to before engines existed (the server determinism golden hash is unchanged).
- **A world keeps its generator:** `server.toml` records `<module>:<engine>` (e.g. `overworld:classic`); a build with a different overworld generator refuses to open the world, because new terrain would not fit the saved terrain at the borders.
- **Speed** (cloud VM, 4 vCPU Xeon 2.1 GHz, one criterion run): a full column with all stages 0.35 ms (`classic`) / 0.37 ms (`climate`); the 2D surface pass for 256 block columns 16 µs / 58 µs; 64 columns on 4 threads 6.5 / 7.5 ms. Both stay under the 1 ms per column budget.
- **Adding an engine** (until mods can register stages through the WIT API in stage 3): a new `Engine` variant in `worldgen::config` with its data table, a branch in `GenContext::surface`, determinism, continuity and cross-border tests, and a module that selects it. Planned ideas: `flat` for test builds, `islands` (archipelagos), a fantasy engine with tall mountain ranges, an `amplified`-style one. Caves, rivers and structures per engine come as stages.
- Not yet: biome blending of surface blocks (borders between biome tops are sharp), rivers, more cave types, ground cover (grass, flowers: needs block models), animals per biome (the `mobs` module, stage 4: biomes will get spawn lists).

## Implementation (T1-03)

- `worldgen::Pipeline` holds ordered `Stage`s (`fn run(&GenContext, &mut ProtoColumn)`); `replace(id, …)` and `insert_after(id, …)` let mods change it (through the mod API from stage 3). Built-in overworld: `core:surface` (2D heightmap + biome pass, also `GenContext::surface` for LOD and neighbors) → `core:terrain` → `core:caves` (3D noise on a 4-block grid, trilinear interpolation) → `core:ores` (random-walk veins) → `core:trees`.
- All blocks and parameters come from mod data: `mods/overworld/data/worldgen/overworld.toml` (the `overworld` module, ADR-0017) names blocks by id and is resolved through the registry; the engine code contains no block names.
- **Cross-column features without a dependency graph:** every column places the trees whose origins lie in itself and its 8 neighbors (origins and shapes are pure functions of the seed and the origin), writing only its own cells. Columns are therefore pure functions of `(seed, position)`; a test checks that every crown is complete across borders. The chunk-status dependency graph described above is needed only once a stage must read a neighbor's *generated blocks* (not its pure surface data); none does yet.
- `Pipeline::generate_parallel` runs columns as `Visible` jobs on the T0-02 pool.
- Previews: `cargo run --release -p worldgen --example map_preview -- out.bmp [seed] [radius] [build]` (top-down map), `--example stats -- [seed] [radius] [build]` (biome shares, sea share, height range, tree count).

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
