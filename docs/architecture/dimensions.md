# Dimensions and space

Capabilities: `dimensions`, `space` ([ADR-0018](../adr/0018-engine-capabilities-per-build.md), accepted 2026-11-27). Status: implemented (T4-02): dimension definitions in module data, several dimensions on one server, player travel, the dimension id in the protocol (version 6). Entities other than players do not travel yet (that needs the mod API, stage 3).

## Dimensions

A **dimension** is a world of its own: its own columns, its own generator (a world module's `worldgen/<dimension>.toml`, ADR-0017), its own region set, its own physics data. Builds decide which dimensions exist: the classic build has the overworld (later the nether and the end); the industrial build has planets and space; the RPG build may have one giant world plus instances.

- A world module declares its dimensions in `data/dimensions/<name>.toml`:

  ```toml
  id = "overworld"
  generator = "worldgen/overworld.toml"
  height = [-64, 320]        # world bounds
  sky = "day_night"          # day_night | none | fixed (render graph)
  [physics]
  gravity = [0.0, -9.8, 0.0] # m/s², for grids and other rigid bodies
  player_gravity_scale = 1.0 # multiplies the movement profile's gravity (0.38 for a Mars-like planet)
  pressure = 1.0             # at the sea level; 0 = vacuum
  pressure_by_altitude = [[62, 1.0], [200, 0.6], [320, 0.3]]   # spline
  drag = 0.09
  fluid_density = 1.0        # for buoyancy of grids
  ```

- Storage: `<world>/dimensions/<id>/` with the T1-02 column store each; `server.toml` lists the dimensions and their generators.
- Regions are per dimension (a region never spans dimensions); the node schedules all dimensions' regions on one pool.
- Travel: a mod moves an entity (or a grid, with `moving_grids`) to another dimension at a position; the engine saves it in the source dimension's Exchange phase and inserts it in the target's.
- Protocol: `Spawn` and every column, entity and grid message carry a dimension id (one byte); the client keeps one map per dimension it has seen recently and drops others.
- Players: the movement profile (the `movement` module) is tuned for feel, not in m/s²; a dimension scales its gravity with `player_gravity_scale`. Breathing and pressure effects are gameplay.

## Implementation (T4-02 part 1)

- `world::dimension`: `DimensionDef` (the format above, `deny_unknown_fields`) with checks:
  - the id is `[a-z0-9_]`;
  - the height is multiples of 16, low below high, at most 4096 blocks;
  - physics values are in range and pressure heights increase;
  - the generator is a `.toml` inside the module.

  It also provides `pressure_at(y)` (piecewise linear), `bounds()` and `slot()`.
- `dimension::from_mods(&ModSet)` collects every `data/dimensions/*.toml` of the build. An id defined by two mods is an error naming both. Each dimension needs a provider for its `worldgen:<id>` slot that has the generator file.
- `mods/overworld` and `mods/overworld_plus` define `overworld`.
- **Server and client** read the build's dimensions at start (an invalid one stops the start with its message). Players use the overworld's `player_gravity_scale` (`PhysicsConfig::with_gravity_scale`; ×1 changes nothing, so the classic movement stays bit-identical).
- **Storage:** the overworld stays at the world root (no migration). Other dimensions will live under `<world>/dimensions/<id>/`. `server.toml` already records the provider of every `worldgen:<id>` slot.

## Implementation (T4-02 part 2)

- **Server:** one `game::World` per dimension, each with its own regions, column store, generator (its `worldgen:<id>` module's file, a seed derived from the world seed and the id), bounds, cache and player physics (the movement profile × `player_gravity_scale`). Region threads are shared out between the worlds. A build with one dimension runs its world directly, as before; with more, `server::node::Node` owns the worlds on the world thread: it routes every message from the connections to the world the player is in and steps each world in turn at 20 TPS.
- **Capability:** a build with more than one dimension must have a mod requiring `dimensions`, or the server does not start (the message names the build and its dimensions).
- **Storage:** the overworld stays at the world root; every other dimension has its store in `<world>/dimensions/<id>/`.
- **Travel:** between two ticks the node takes the player out of its world (its queued intents applied, as when leaving) and inserts it into the target world, at the given position or flying 32 blocks under the top at the same x and z. That is the tick boundary the Exchange phase stands for: no region of either world runs meanwhile. The target world gives the player a new network id and sends a new `Spawn`. Ways to travel: `Server::travel(player, dimension, pos)` (what a portal or a mod will call) and, on offline servers only (development and tests), the chat command `/dimension <id>` (without an id it lists the dimensions).
- **Players:** the player file records the dimension (`dimension = "<id>"`, left out for the overworld, so older files still load); a returning player joins the dimension it left in.
- **Protocol (version 6):** a dimension's index is its position in the build's dimensions sorted by id (both sides read the same build). `Spawn`, column payloads, entity datagrams, `BlockChanges`, `ForgetColumn` and the client's `Dig` and `Place` carry it. The server refuses an edit tagged with another dimension than the player's. A new `Spawn` with another dimension means the player travelled.
- **Client:** on a travel the session drops the whole old world: columns, light, meshes, pending columns and edits, other entities, prediction history. It then starts a map with the new dimension's bounds and the new gravity scale. Anything still in flight from the old dimension is dropped by its tag (and a column with other bounds than the map's is dropped too). The client keeps one map, not one per recently seen dimension: coming back streams the columns again. Keeping the last dimension's map is a possible later optimisation, if going back and forth through a portal shows a wait.
- **Tests:** `crates/client/tests/dimensions.rs`. A test module adds a `moon` with another height and half the gravity. A player starts in the overworld and goes to the moon with the chat command: there it holds only moon columns and has half the gravity. It leaves, the file says `moon`, and it rejoins on the moon. The server API brings it back to the overworld. A second test checks that a build with two dimensions and no capability is refused.

## Space

Space is a dimension with `pressure = 0` and a very large horizontal extent. Two engine needs come with it:

- **Large coordinates.** Block positions are `i32` (±2 billion blocks is enough); entity and grid positions are `f64` in the simulation; the renderer works relative to the camera (camera-relative coordinates, already needed for far LOD) so precision does not degrade far from the origin.
- **Orbit-scale travel.** Planets are separate dimensions; "flying to a planet" is a transition (an approach zone at the edge of a space dimension leads into the planet's dimension at its top), not a continuous 10⁶-block flight. True orbital mechanics, if wanted, are gameplay in a mod using the grid force API; the engine gives gravity per dimension and per zone (a gravity source list in the space dimension: point masses with a radius, so a ship near a planet is pulled towards it).

## Open

- Whether dimensions of one world may run on different nodes (shards, ADR-0009): likely yes, a dimension is a natural shard boundary.
- Seamless planet approach (no loading transition): out of scope until the transition version is played.
