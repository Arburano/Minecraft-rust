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
| other files (`worldgen/<dimension>.toml`, `movement/profile.toml`) | provided: the last mod in load order that has the file | `overworld` provides `worldgen/overworld.toml` |

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
| `classic_agile` | `core`, `movement_agile`, `overworld` | the classic world with dash, double jump, wall slide, crawl and glide (example movement module) |

A world remembers the generator that made it (`server.toml`), so it opens only with a build that has the same overworld module and engine.

`nether`, `end` and `mobs` join `classic` when they have content.

## World-generation modules

A world module provides its dimension's `worldgen/<dimension>.toml`, which picks a landscape engine (`worldgen.md`, "Engines") and names every block it places. A module that replaces only the generation (like `overworld_plus`) brings just its engine settings, biomes, tree kinds and the blocks they need; everything else in the build stays. A world module declares `provides = ["worldgen:<dimension>"]`; a build with two providers for one slot fails at load, and one without a provider for the overworld fails at start (slots, ADR-0013, T1-18). The movement profile is a slot too (`movement`): [movement-profiles.md](movement-profiles.md).

## Rules for cutting modules

1. **One concern per module.** A concern is something a build may want without the rest: a world, creatures, a movement feel, a feature (machines, roles). If a module grows a second concern that a build could leave out, split it before it ships.
2. **A world is a module.** Everything that only exists in one dimension (its generator, blocks, creatures, structures) lives in that dimension's module or in a module that depends on it, never in `core`.
3. **`core` stays small:** only what every planned build needs. Adding to `core` needs a reason in the PR ("every build uses this").
4. **Cross-module features go through optional dependencies or bridge mods.** A portal from the overworld to the nether belongs to `nether` with an optional dependency on `overworld`; a build without the overworld simply has no portal there.
5. **Ids follow the owner.** A block or entity is named in the namespace of the module that owns it. Moving content between modules after release needs an ID-table migration, so choose the owner first.
6. **State size and responsibility when adding a module:** a row in the catalogue above (owns, depends on, status, size, tick budget). Big assets (textures, sounds, models) are counted too: they decide the download of a build.
7. **No module reads another module's files.** Modules see each other only through registry ids, the mod API and the merge rules above.

## Engine capabilities

Some features cannot be mods: they need engine speed or reach into physics, network and rendering. They are **capabilities** ([ADR-0018](../adr/0018-engine-capabilities-per-build.md), accepted 2026-11-27): a mod requires them in `mod.toml` (`[requires] capabilities = [...]`), and the engine starts only those that some mod of the build requires. A capability no mod asked for does not run (no threads, memory, tick systems, messages or render passes); heavy ones are also left out of the build's release binary by a cargo feature.

| Capability | What it gives | Spec | Stage |
| --- | --- | --- | --- |
| `dimensions` | several worlds per server, travel between them, physics per dimension | [dimensions.md](../architecture/dimensions.md) | 4 |
| `moving_grids` | moving block structures with rigid-body physics and collision | [ADR-0019](../adr/0019-moving-block-structures.md), [moving-structures.md](../architecture/moving-structures.md) | 4 |
| `space` | vacuum, large coordinates, planets as dimensions, gravity sources | [dimensions.md](../architecture/dimensions.md), "Space" | 4–5 |
| `zones` | named areas with rules (PvP, build rights, spawn tables, level ranges) | PRD §14 No. 11 | 4 |
| `instances` | private copies of an area per party, created and dropped on demand | "RPG build" below | 4+ |
| `progression` | server-stored per-player stats, levels and skills with a mod API | "RPG build" below | 4+ |

How it works (T4-01):

- **The set.** `world::ModSet::capabilities()` is the union of the mods' `[requires] capabilities`. `world::Capabilities` is a small bit set, so checking it costs one bit test.
- **Compiled in.** A capability with a heavy dependency has a cargo feature of the same name on `server` and `client` (first: `moving_grids`, on by default for development). `server::capabilities::compiled()` and `client::capabilities()` give the binary's set. The server and the client check the build at start: "build X needs capability Y (required by mod Z); this server binary was built without it (cargo feature "Y")".
- **Started.** `server::capabilities::SubsystemRegistry` maps a capability to a factory; `Server::start` starts one `Subsystem` per capability of the build. A subsystem is ticked on the world thread after the regions simulate and stopped with the world. A capability the build does not require has no factory call, thread, tick call or message; with no capability the world tick only checks an empty list. A required capability without a subsystem is an error ("… which this engine version does not implement yet"); none is implemented yet, the first is `moving_grids` (T4-03).
- **Announced.** `Welcome.capabilities` (protocol version 5) lists the capabilities the server runs, by name. The client refuses a server whose capability it does not know or was built without.
- **Recorded in the world.** `server.toml` has `capabilities = [...]`: every capability of every build that opened the world, since data written once stays. A build without one of them refuses the world, naming the build and the capability.
- **CI** runs the workspace tests with every feature (the default) and `cargo test -p server --no-default-features` without them.

## Planned builds

Two large builds are planned besides `classic`. They share only the engine, `core` and `movement` (and may replace `movement` too). Everything else is their own modules, so neither carries the other's logic. Working titles.

### Industrial mod pack

Giant constructions that fly, sail and drive with real physics, machines, other planets and space.

| Module | Owns | Capabilities |
| --- | --- | --- |
| `industrial_world` | its own overworld generation (its own engine settings or engine, biomes, resources for industry) | — |
| `planets` | other planets as dimensions: generation, gravity, atmosphere, resources | `dimensions` |
| `space` | the space dimension, gravity sources, travel to planets | `dimensions`, `space` |
| `contraptions` | assembly blocks, bearings, propellers, balloons and burners, floating blocks, gyroscopes, seats and controls — like Create: Aeronautics ([review](../research/moving-structures-review-2026-11-27.md)) | `moving_grids` |
| `machines` | power (rotation or energy), machines, pipes, belts, item transport | — |
| `rocketry` | rocket and jet engines, fuel, thrust, heat; depends on `contraptions` | `moving_grids`, `space` |
| `mobs_industrial` | its own creatures | — |

### RPG build

A giant world with zones, bosses, dungeons that appear and grow stronger, progression and skills.

| Module | Owns | Capabilities |
| --- | --- | --- |
| `rpg_world` | its own world generation, for example the `map` engine (a painted map of regions) or a climate engine with its own biomes; zones by region | `zones` |
| `dungeons` | dungeon sites, layouts, the life cycle below | `zones`, `instances` |
| `progression` | levels, experience, attributes | `progression` |
| `skills` | skills and classes, built on `progression` | `progression` |
| `bosses` | bosses with phases and arenas | — |
| `mobs_rpg` | creatures by zone and level | — |
| `quests`, `npcs` | quests, traders, dialogue (later) | — |

**Dungeons that appear and grow** (design notes for the `dungeons` module and the engine parts it needs):

- **Reserved sites.** The world generator reserves dungeon sites when the terrain is generated: a footprint and an entrance, stored with the column, never built over by later generation. Players build around them, not into them (a zone rule).
- **Content separate from terrain.** A dungeon's interior is a template instantiated into its site (or into an instance), with its own state record: level, layout id, cleared rooms, loot table, last reset. Upgrading or replacing a dungeon rewrites only its site from a new template, so nothing players built nearby is touched.
- **Life cycle**, driven by the mod on a schedule or by player activity: a site wakes up (a new dungeon appears), levels up (harder layout, stronger mobs, better loot), resets after being cleared, or goes dormant.
- **Instances** (capability `instances`): a party can enter a private copy, a temporary dimension holding only the dungeon, created from the template and dropped when the party leaves.

## Other planned modules

| Module | What it adds | Engine work it needs |
| --- | --- | --- |
| `roles` | role-play servers: roles, permissions, zones, chat channels | a server-side permission API for mods (who may break, place, enter, use commands), stored per world; capability `zones` |

Example builds once these exist:

| Build | Modules |
| --- | --- |
| classic | `core`, `movement`, `overworld`, `nether`, `end`, `mobs` |
| industrial mod pack | `core`, `movement`, `industrial_world`, `planets`, `space`, `contraptions`, `machines`, `rocketry`, `mobs_industrial` |
| RPG | `core`, `movement`, `rpg_world`, `dungeons`, `progression`, `skills`, `bosses`, `mobs_rpg` |
| role-play server | `core`, `movement`, `overworld`, `mobs`, `roles` |

## Not done yet

- The build id is not stored in the world, and opening a world with a different build is not checked (with T1-18's slot checks).
- The server does not send its mod list to the client (`mod_manifest_hash` in `Welcome` is zero); both sides must run the same build.
- The updater (ADR-0010) does not download per build.
- A per-build release binary with only its capabilities' features (the build tool, ADR-0018 "Not decided here").
- WIT interfaces linked only with their capability (ADR-0018 rule 5): with the stage 3 mod API.
- Client-side subsystems (render passes of a capability): with the first capability that has one (T4-06).
