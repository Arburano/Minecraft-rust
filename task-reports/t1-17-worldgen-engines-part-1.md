# T1-17 (part 1): world-generation engines — `climate` engine and the `overworld_plus` module

## What was done

The owner asked (2026-11-27) for different generation engines: a basic landscape engine and advanced ones that replace only the generation, possibly with their own blocks (new trees, and later animals). This is part 1 of T1-17, done the ADR-0017 way: the engine is chosen by a world module, and a build picks the module.

**Engines** (`crates/worldgen`):

- `engine = "classic" | "climate"` in the dimension's generation file selects `config::Engine`. `GenContext::surface` dispatches on it; every stage after the shape is shared.
- `classic`: unchanged; its output is bit-identical to before (the server determinism golden hash did not change).
- `climate`:
  - fractal Perlin noises for continentalness, erosion, ridges, temperature and humidity;
  - height = continent spline + ridge spline × erosion factor × inland fade + detail;
  - temperature drops with height above the sea;
  - biomes are the first in file order whose climate ranges hold, with a catch-all last.
- Tree kinds: `[[tree_kinds]]` with their own log, leaves, trunk range and crown shape (`round`, `cone`). A biome lists the kinds that grow in it. When a biome has a single kind no extra random draw is made, so single-kind data keeps its trees.
- Config checks:
  - unknown engine;
  - `climate` without a `[climate]` table, or climate ranges with `classic`;
  - unknown or duplicate tree kinds;
  - trunk ranges; biome count.

**Modules and builds:**

- `mods/overworld_plus`: the climate engine, 9 biomes (snowy peaks, rocky mountains, ocean, snowy taiga, taiga, desert, birch forest, forest, plains) and 5 blocks (spruce and birch log and leaves, snow block).
- `builds/classic_plus.toml` = `core`, `movement`, `overworld_plus`.
- `mods/overworld` now states `engine = "classic"`.
- `world::ModSet::provider` names the mod that provides a file.
- The server records `generator = "<module>:<engine>"` in `server.toml`. It refuses a world made by another generator and fills the field into older worlds.
- Fallback textures for the 5 new blocks (`assets::fallback`). `CONVERTER_VERSION` was bumped to 2 and the golden file regenerated. The fallback test now covers the blocks of every mod.
- Tools: `map_preview` and `stats` take a build; `stats` prints biome names, height range and trees of every kind. The `pipeline` bench runs both engines.

**Tests:**

- `crates/worldgen/tests/engines.rs`:
  - builds pick their engine; the classic build does not carry the new blocks;
  - climate determinism (serial = parallel, seeds differ);
  - variety and continuity over 12 × 12 km: oceans > 5%, mountains > 1%, ≥ 7 biomes, neighbor steps ≤ 6 blocks;
  - every tree kind's crown is complete across column borders, using the shape;
  - config errors.
- Unit tests for splines and climate ranges.
- Server: `a_world_keeps_its_generator` (an old world gets the record, another build is refused, the same build opens it).
- `cargo fmt --all`, `cargo clippy --workspace --all-targets` with `-D warnings`, `cargo test --workspace`.

**Numbers** (cloud VM 4 vCPU Xeon 2.1 GHz, one criterion run, 6 s measurement):

| | classic | climate |
| --- | --- | --- |
| column, all stages | 0.32–0.38 ms | 0.35–0.39 ms |
| surface pass, 256 block columns | 15–17 µs | 56–60 µs |
| 64 columns, 4 threads | 6.2–6.9 ms | 7.1–8.0 ms |

Landscape shares (`stats`, 128 × 128 columns): seed 7 — ocean 20%, forest 18%, plains 17%, taiga 15%, snowy taiga 12%, birch forest 7%, desert 6%, rocky mountains 4%, snowy peaks 1%, heights 23..193. Seed 2026 — sea 46%, heights 17..181.

## What went wrong

- The first climate tuning was too cold: snowy taiga and taiga covered over half the land, because height cooling also applied to ordinary lowlands. Height cooling was lowered from 0.006 to 0.0035 per block and the cold ranges narrowed.
- The first cross-border tree test stopped searching once every kind had one forest, and found no birch trees in it. It now collects three forests per kind and checks crowns by shape.

## Issues encountered

- **The mode is a build, not a world-creation option.** T1-17 planned a mode choice on the world-creation screen. With ADR-0017 the generator comes with the module, so the choice is the build (`--build classic_plus`). A build picker in the menu is part 2.
- **The look needs the owner's review** (T1-17 acceptance): the preview is `cargo run --release -p worldgen --example map_preview -- out.bmp 7 48 classic_plus`, or play with `client --build classic_plus`.
- **Not yet:**
  - blending of biome surface blocks (borders between biome tops are sharp; heights are smooth);
  - rivers, more cave types;
  - ground cover (needs block models);
  - animals per biome (the `mobs` module, stage 4).
- **Tectonic** was not ported. The climate engine is our own design from the general multi-noise idea; `THIRD_PARTY.md` still says "not ported yet".
- **Two modules providing the same generation file** in one build: the later one in load order wins silently. A slot check (T1-18) should make this an error.
- **Laptop numbers** for the 1 ms budget are for the owner (`cargo bench -p worldgen --bench pipeline`).
