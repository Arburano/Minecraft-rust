# Dimensions and space

Capabilities: `dimensions`, `space` ([ADR-0018](../adr/0018-engine-capabilities-per-build.md), accepted 2026-11-27). Status: design; today a server has one world, the overworld.

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

## Space

Space is a dimension with `pressure = 0` and a very large horizontal extent. Two engine needs come with it:

- **Large coordinates.** Block positions are `i32` (±2 billion blocks is enough); entity and grid positions are `f64` in the simulation; the renderer works relative to the camera (camera-relative coordinates, already needed for far LOD) so precision does not degrade far from the origin.
- **Orbit-scale travel.** Planets are separate dimensions; "flying to a planet" is a transition (an approach zone at the edge of a space dimension leads into the planet's dimension at its top), not a continuous 10⁶-block flight. True orbital mechanics, if wanted, are gameplay in a mod using the grid force API; the engine gives gravity per dimension and per zone (a gravity source list in the space dimension: point masses with a radius, so a ship near a planet is pulled towards it).

## Open

- Whether dimensions of one world may run on different nodes (shards, ADR-0009): likely yes, a dimension is a natural shard boundary.
- Seamless planet approach (no loading transition): out of scope until the transition version is played.
