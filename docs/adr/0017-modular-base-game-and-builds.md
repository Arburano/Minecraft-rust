# 0017. Modular base game and builds

Status: Accepted (owner, 2026-11-27)

## Context

- Until now all gameplay data lived in one mod, `mods/core`: blocks, the overworld generator config, the player movement numbers and the entity limits. The engine read `mods/core/data` by a fixed path.
- The owner plans completely different games on the same engine: the classic survival game, a high-fantasy world, an engineering build with machines, aircraft, rockets and space, role-play servers with roles and permissions. A build must carry only what it uses: no unused worlds, mobs, code or assets in memory, on disk or in the download (PRD §14: optimization serves the feel, and RAM is a budget).
- ADR-0013 already decided that movement and generation are replaceable slots and planned to split `core` into `core`, `core-movement` and `core-worldgen` (task T1-18). It did not cover dimensions, mobs or whole builds.

## Decision

1. **A build is a list of mods.** A build file `builds/<id>.toml` names the mods a game runs with on top of the engine (`[build] id, name, description, mods`). Dependencies from each `mod.toml` are added automatically; the load order is topological, ties broken by id (`mod-manifest.md`). The server, the client and the bot harness take `--build <id|file>` (default `classic`) and `--mods-dir`. The engine never reads a fixed mod path.
2. **Mods are cut by responsibility, one per concern**, so that a build includes or leaves out a whole concern:
   - `core`: only what every build shares: the common block palette (stone, dirt, wood, water, ores, glass, light blocks) and default entity limits. It stays small.
   - `movement`: the player movement profile (the `movement` slot of ADR-0013).
   - **one mod per world (dimension):** `overworld` now; `nether` and `end` are planned modules without content. A dimension mod owns its generation, its own blocks (under its namespace) and, later, its biomes, structures and portals.
   - `mobs`: creatures, their AI, spawning, limits and drops (planned, stage 4). Creatures that belong to one dimension may be split out (`mobs_nether`, …) so that a build without that dimension does not carry them.
   - Later features are separate mods as well: moving structures with collision, space, roles and permissions (see `docs/modding/builds.md`, "Planned builds" and "Other planned modules"; the engine features they need are capabilities, ADR-0018). They are documented now and built later.
3. **How data from several mods is combined.**
   - *Merged:* blocks — every mod's `data/blocks` adds blocks under its own id; entity limits — every mod's `data/entities/limits.toml` adds kinds, a later mod may replace an entry.
   - *Provided:* a single file such as `worldgen/overworld.toml` or `player/physics.toml` comes from the last mod in load order that has it. A build without the owning mod gets a clear error ("no mod of this build provides …").
4. **Block ids keep their namespace.** Blocks that every build shares stay `core:*`, so existing worlds keep their ID tables. A block that belongs to one world goes to that world's mod and namespace (`nether:netherrack`), never to `core`.
5. **Relation to ADR-0013 / T1-18.** This ADR performs the split T1-18 planned, with these names: `core-worldgen` → one mod per dimension (`overworld`, …), `core-movement` → `movement` (mod ids allow only `[a-z0-9_]`). T1-18 still adds `provides`, slot checks, the movement profile format and the `builtin-overworld` feature.
6. **Size and responsibility are recorded per module** in `docs/modding/builds.md` (what it owns, what it depends on, its tick budget and data size), and a new module states them when it is added. A module that grows a second, separable concern is split before it ships.

## Consequences

- **Simpler:** a new game is a new build file plus its own mods; leaving out a dimension or all mobs is removing a line. The server and client load exactly the build's mods (`world::ModSet`).
- **Harder:** more manifests to keep in sync; cross-module features (portals between worlds, mobs per dimension) need optional dependencies or small bridge mods instead of direct references.
- **Not done here:** mod downloads per build (the updater, ADR-0010, and the server's mod list in the handshake, `mod_manifest_hash`), storing the build id in the world and refusing a world opened with another build (with T1-18's slot checks), and the WASM side of modules (stage 3).
- **Tasks:** T1-24 (this split and the loader, done); content of `nether`, `end` and `mobs` comes with stage 4 gameplay.

## Alternatives considered

- **Keep one `core` and switch features off with flags.** Everything is still installed and loaded; flags spread through the data and code; third-party builds cannot remove what they do not want.
- **Rename all blocks into per-world namespaces now** (`overworld:grass_block`). Cleaner on paper, but breaks every saved world's ID table for no gain: the classic blocks are shared by every planned build.
- **One mod per feature at a fine grain** (a mod per block family, per mob). Too many manifests and load-order edges for little saving; the unit of choice for a build is a concern (a world, mobs, movement), not a block.
