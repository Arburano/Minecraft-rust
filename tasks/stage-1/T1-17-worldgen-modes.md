# T1-17. World generation modes: normal and extended

- **Status:** todo (later: after the main stage 1 tasks; owner decision 2026-11-22)
- **Depends on:** T1-03, T1-13 (world creation screen)
- **Owner:** agent (cloud) + owner review of the look

## Goal

Two generation modes chosen when a world is created:

- **Normal** — the T1-03 pipeline (`mods/core/data/worldgen/overworld.toml`). This is the mode all current work targets.
- **Extended** — a richer generator of our own: multi-noise biome placement (temperature, humidity, continentalness, erosion), height splines for mountains, plateaus and canyons, smooth biome blending, more cave types, more biomes and surface variety.

## Reference: Terralith (look and feel only)

The owner chose **Terralith** (Stardust Labs) as the reference for what "extended" should feel like. Its license (Stardust Labs license, last altered 2023-02-05) forbids copying parts into another project, using portions in other projects without permission, and using anything created by Stardust Labs in AI projects. Therefore:

- Agents must **not** download, read, parse or port Terralith files or code, and nothing from it goes into the repository.
- The reference is **human observation**: the owner plays Terralith worlds and describes, with screenshots in `docs/research/worldgen-reference/`, the landscapes, biome variety and transitions wanted. Agents implement those goals with our own design from public, general techniques.
- Using Terralith's actual data (e.g. a datapack-interpreter mode) requires written permission from Stardust Labs and the lawyer's answer (T0-10); it is out of scope here.

## Deliverables

- A mode registry in `worldgen` (`Normal`, `Extended`), stored in the world metadata (`world.redb`) so a world keeps its mode.
- The extended pipeline as stages configured by `mods/core/data/worldgen/extended.toml`.
- The mode choice on the world creation screen (T1-13).

## Acceptance criteria

- [ ] Both modes deterministic (the T1-03 determinism and cross-border tests run for both).
- [ ] Extended ≤ 1 ms per column per core including packing on the laptop, or an owner-approved deviation.
- [ ] The owner approves the look against the reference screenshots.
