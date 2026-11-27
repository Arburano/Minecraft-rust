# T1-17. World generation modes: normal and extended

- **Status:** todo (later: after the main stage 1 tasks; owner decision 2026-11-22)
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

- A mode registry in `worldgen` (`Normal`, `Extended`), stored in the world metadata (`world.redb`) so a world keeps its mode.
- The extended pipeline as stages configured by the `overworld` module (`mods/overworld/data/worldgen/extended.toml`, ADR-0017).
- The mode choice on the world creation screen (T1-13).

## Acceptance criteria

- [ ] Both modes deterministic (the T1-03 determinism and cross-border tests run for both).
- [ ] Extended ≤ 1 ms per column per core including packing on the laptop, or an owner-approved deviation.
- [ ] The owner approves the look against the reference screenshots.
