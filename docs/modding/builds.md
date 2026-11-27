# Builds and base-game modules

Decision: [ADR-0017](../adr/0017-modular-base-game-and-builds.md). Code: `world::modset` (`ModSet`, `locate`). Files: [`../../builds/`](../../builds/), [`../../mods/`](../../mods/).

## The idea

The engine is one; games are many. A **build** is a list of mods on top of the engine. The plan is to make completely different builds from different sets of modules, adding new modules to the core and removing the ones a game does not need:

- the classic survival game;
- a high-fantasy world (its own generator, races, no nether or end);
- an engineering build: machines, moving contraptions, aircraft, rockets, space;
- role-play servers with roles and permissions;
- small test builds (a flat world, a benchmark world).

A build carries only its modules: no unused worlds, creatures, code, textures or sounds are installed, downloaded or loaded. This is why modules are cut by responsibility and their size is tracked.

## Build file

`builds/<id>.toml`:

```toml
[build]
id = "classic"                     # [a-z0-9_]
name = "Classic"                   # shown to players
description = "The overworld with the shared base"
mods = ["core", "movement", "overworld"]   # dependencies are added automatically
```

Running a build:

```bash
cargo run -p server -- --offline --build classic
cargo run -p client -- --offline-name Dev --build classic
cargo run -p client --bin bots -- --embedded /tmp/w --build classic
```

`--build` takes an id (looked up in `builds/` next to the mods folder, then in the source tree) or a path to a build file; `--mods-dir` points to the folder of mod folders (default: `mods/` next to the executable, else the source tree).

## How mods combine

| Data | Rule | Example |
| --- | --- | --- |
| `data/blocks/*.toml` | merged: each mod adds blocks under its own id | `core:stone`, later `nether:netherrack` |
| `data/entities/limits.toml` | merged: each mod adds kinds; a later mod replaces an entry or `[default]` | `mobs` adds its creatures to the core defaults |
| other files (`worldgen/<dimension>.toml`, `player/physics.toml`) | provided: the last mod in load order that has the file | `overworld` provides `worldgen/overworld.toml` |

Load order: dependencies first, ties by id. A missing mod, a version mismatch, a cycle or a file no mod provides is a clear error at start (`world::ModSetError`).

## Module catalogue

Size is the data on disk today; the tick budget is the soft limit from `mod.toml`.

| Module | Owns | Depends on | Status | Size today | Tick budget |
| --- | --- | --- | --- | --- | --- |
| `core` | the shared block palette (stone, dirt, grass, sand, gravel, water, wood, leaves, ores, glass, glowstone, torch), default entity limits | — | done | 15 block files, 1 limits file (< 10 KB) | 0.5 ms |
| `movement` | the player movement profile (walk, sprint, jump, swim, fly); the abilities of T1-18 later | `core` (swim blocks) | done | 1 file (< 2 KB) | 0.2 ms |
| `overworld` | overworld generation: terrain, caves, biomes, beaches, ores, trees; later its biomes' tints, weather, structures | `core` | done | 1 file (< 3 KB) | 0.5 ms |
| `overworld_plus` | an advanced overworld: the `climate` engine (continents, oceans, mountains, snowy peaks, 9 biomes) and its blocks: spruce and birch logs and leaves, snow (`overworld_plus:*`); replaces `overworld` in a build | `core` | done (look: owner review) | 5 block files + 1 file (< 8 KB) | 0.5 ms |
| `nether` | nether generation, nether blocks (`nether:*`), portals to the overworld (optional dependency) | `core` | planned (stage 4+), no content | manifest only | 0.5 ms |
| `end` | end generation (islands), end blocks (`end:*`), the boss fight (depends on `mobs`), the exit portal | `core` | planned (stage 4+), no content | manifest only | 0.5 ms |
| `mobs` | creatures: entity kinds, AI goals, spawning rules, per-kind limits, drops | `core` | planned (stage 4), no content | manifest only | 1.5 ms |

The engine's test entity (`test:walker`, T2-07) is not gameplay; its limit sits in `core` until test entities move out of the server.

### Builds today

| Build | Mods | What it is |
| --- | --- | --- |
| `classic` | `core`, `movement`, `overworld` | the current game: the overworld, no mobs yet |
| `classic_plus` | `core`, `movement`, `overworld_plus` | the same game with the advanced overworld engine |

A world remembers the generator that made it (`server.toml`), so it opens only with a build that has the same overworld module and engine.

`nether`, `end` and `mobs` join `classic` when they have content.

## World-generation modules

A world module provides its dimension's `worldgen/<dimension>.toml`, which picks a landscape engine (`worldgen.md`, "Engines") and names every block it places. A module that replaces only the generation (like `overworld_plus`) brings just its engine settings, biomes, tree kinds and the blocks they need; everything else in the build stays. A build lists one module per dimension; if it lists two that provide the same file, the later one in load order wins (slot checks that make this an error come with T1-18).

## Rules for cutting modules

1. **One concern per module.** A concern is something a build may want without the rest: a world, creatures, a movement feel, a feature (machines, roles). If a module grows a second concern that a build could leave out, split it before it ships.
2. **A world is a module.** Everything that only exists in one dimension (its generator, blocks, creatures, structures) lives in that dimension's module or in a module that depends on it, never in `core`.
3. **`core` stays small:** only what every planned build needs. Adding to `core` needs a reason in the PR ("every build uses this").
4. **Cross-module features go through optional dependencies or bridge mods.** A portal from the overworld to the nether belongs to `nether` with an optional dependency on `overworld`; a build without the overworld simply has no portal there.
5. **Ids follow the owner.** A block or entity is named in the namespace of the module that owns it. Moving content between modules after release needs an ID-table migration, so choose the owner first.
6. **State size and responsibility when adding a module:** a row in the catalogue above (owns, depends on, status, size, tick budget). Big assets (textures, sounds, models) are counted too: they decide the download of a build.
7. **No module reads another module's files.** Modules see each other only through registry ids, the mod API and the merge rules above.

## Planned modules (later)

These are documented so that the engine work they need is not blocked by today's choices. None of them exists yet; each will get an ADR and tasks when its stage comes.

| Module | What it adds | Engine work it needs |
| --- | --- | --- |
| `contraptions` | large moving structures with collision: doors, lifts, trains, ships, aircraft built from blocks | moving block grids (a sub-world with its own transform), collision between the player and moving grids, replication of grid transforms, region hand-off of grids (ADR-0008) |
| `space` | space as a dimension: orbit, planets, rockets, low gravity, vacuum | dimensions with their own physics parameters (gravity per dimension, through the movement profile), large-distance coordinates, travel between dimensions; depends on `contraptions` for rockets |
| `roles` | role-play servers: roles, permissions, zones, chat channels | a server-side permission API for mods (who may break, place, enter, use commands), stored per world; ties to zones (stage 4) |
| `machines` | power, machines, pipes (an engineering build) | block entities with ticking, batched mod systems (ADR-0006), item transport |

Example builds once these exist:

| Build | Modules |
| --- | --- |
| classic | `core`, `movement`, `overworld`, `nether`, `end`, `mobs` |
| high fantasy | `core`, `movement`, its own world module (instead of `overworld`), its own creatures, `roles` |
| engineering / space | `core`, `movement`, `overworld`, `mobs`, `machines`, `contraptions`, `space` |
| role-play server | `core`, `movement`, `overworld`, `mobs`, `roles` |

## Not done yet

- The build id is not stored in the world, and opening a world with a different build is not checked (with T1-18's slot checks).
- The server does not send its mod list to the client (`mod_manifest_hash` in `Welcome` is zero); both sides must run the same build.
- The updater (ADR-0010) does not download per build.
- `provides` and slot checks in `mod.toml` (T1-18).
