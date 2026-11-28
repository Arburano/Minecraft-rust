# T4-02. Dimensions

- **Status:** in-progress (part 1 done 2026-11-28: definitions, checks, gravity scale; part 2: several dimensions at once, travel, protocol)
- **Depends on:** T4-01, T2-06, T2-08
- **Owner:** agent (cloud)

## Goal

Several worlds per server ([dimensions.md](../../docs/architecture/dimensions.md)), the `dimensions` capability: the base for the nether and end, planets, space and instances.

## Deliverables

- `data/dimensions/<id>.toml` (generator, bounds, sky, physics: gravity, `player_gravity_scale`, pressure by altitude, drag, fluid density).
- Storage per dimension under `<world>/dimensions/<id>/`; migration of existing worlds (the overworld moves there or stays at the root with a pointer).
- Regions per dimension on one pool; travel between dimensions for entities in the Exchange phase.
- Protocol: a dimension id on `Spawn`, columns, entities, edits; the client keeps maps per dimension.
- Players' gravity scaled by the dimension.

## Progress

- **Part 1 (done, cloud):**
  - `world::dimension` (format, checks, pressure curve, bounds);
  - `from_mods` with one owner per dimension and a generator check;
  - overworld definitions in `overworld` and `overworld_plus`;
  - server and client apply the overworld's player gravity scale.

  Report: [t4-02](../../task-reports/t4-02-dimensions-part-1.md).
- **Part 2 (todo):**
  - a dimension set in the server's world thread (one `World` per dimension, or regions keyed by dimension);
  - storage under `dimensions/<id>/`;
  - travel in the Exchange phase;
  - protocol version bump with the dimension id;
  - client maps per dimension;
  - the acceptance tests.

## Acceptance criteria

- [ ] Two dimensions with different generators run at once; a player moves between them and back without losing items or position.
- [ ] Tick and memory unchanged within noise for a build with one dimension.
