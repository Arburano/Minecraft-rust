# T1-17. World generation modes: normal and extended

- **Status:** in-progress (part 1 done 2026-11-27: landscape engines, the `climate` engine and the `overworld_plus` module; owner: review of the look; then part 2)
- **Depends on:** T1-03, T1-13 (world creation screen)
- **Owner:** agent (cloud) + owner review of the look

## Goal

Two generation modes chosen when a world is created:

- **Normal** — the T1-03 pipeline (`mods/overworld/data/worldgen/overworld.toml`). This is the mode all current work targets.
- **Extended** — a richer generator of our own, modeled on Tectonic: multi-noise biome placement (temperature, humidity, continentalness, erosion), height splines for mountains, plateaus and canyons, smooth biome blending, more cave types, more biomes and surface variety.

## Reference: Tectonic (owner decision 2026-11-22)

The owner chose **Tectonic** (by Apollo, `Apollounknowndev/tectonic`, Modrinth project `tectonic`) as the reference for the extended generator, replacing Terralith (whose Stardust Labs license forbids copying and any AI use, so it is not used at all).

License status (checked 2026-11-22):

- Modrinth declares **MIT**. The GitHub repository has **no LICENSE file**, so the copyright line MIT requires us to reproduce is not published.
- Until that is clarified, agents may read Tectonic and port its **ideas and parameters** (terrain shaping, continent/erosion/ridge splines, biome placement), crediting it in `THIRD_PARTY.md`. Copying Tectonic files verbatim into the repository waits for a published LICENSE with a copyright line (the owner asks the author to add one, or confirms the holder and year in writing).
- Tectonic builds on Minecraft's density-function world generation and may contain or override **vanilla data** (`minecraft:` worldgen files). Anything derived from vanilla data is Mojang's and never enters the repository (`docs/legal.md` rule 2). Our implementation of density-function-style generation is written from public documentation, never from decompiled Minecraft code (rule 1).
- The owner's observations and screenshots of Tectonic worlds go to `docs/research/worldgen-reference/`.

## Deliverables

Part 1 (done, owner request 2026-11-27: "different generation engines: a basic landscape and advanced ones that replace only the generation, maybe adding blocks such as new trees"):

- Landscape engines in `worldgen` (`engine = "classic" | "climate"` in the dimension's generation file), `worldgen.md` "Engines".
- The `climate` engine: continentalness, erosion and ridges through height splines, temperature cooling with height, humidity, biomes by climate ranges; tree kinds with crown shapes (`round`, `cone`) per biome.
- The `overworld_plus` module (ADR-0017) with its blocks (spruce, birch, snow) and the build `classic_plus`. The mode is chosen by the build instead of a world-creation setting; the world records its generator in `server.toml` and refuses another one.

Part 2 (todo):

- Biome blending of surface blocks, rivers, more cave types (cheese and spaghetti caves), more biomes and surface variety, closer to the Tectonic reference after the owner's review.
- A build choice on the world creation screen (T1-13 menu), now that the mode is a build.

## Acceptance criteria

- [x] Both modes deterministic (the T1-03 determinism and cross-border tests run for both; `worldgen/tests/engines.rs`).
- [ ] Extended ≤ 1 ms per column per core including packing on the laptop, or an owner-approved deviation (VM: 0.37 ms without packing, 0.26–0.32 ms packing).
- [ ] The owner approves the look against the reference screenshots.
