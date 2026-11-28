# T4-02. Dimensions

- **Status:** done (2026-11-28; part 1: definitions, checks, gravity scale; part 2: several dimensions at once, player travel, protocol v6)
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
- **Part 2 (done, cloud):**
  - `server::node::Node`: one `World` per dimension on the world thread, routing by player;
  - storage under `dimensions/<id>/` (the overworld stays at the root);
  - player travel at the tick boundary (`Server::travel`, `/dimension <id>` on offline servers);
  - player files record the dimension;
  - protocol version 6 with the dimension index;
  - the client resets its world on a travel and drops stale messages by tag;
  - tests in `crates/client/tests/dimensions.rs`.

  Report: [t4-02 part 2](../../task-reports/t4-02-dimensions-part-2.md).
- **Left for later:** travel of entities other than players (needs the mod API); keeping the last dimension's map on the client.

## Acceptance criteria

- [x] Two dimensions with different generators run at once; a player moves between them and back without losing position (there are no items yet; the player file keeps the dimension).
- [ ] Tick and memory unchanged within noise for a build with one dimension: not measured. A one-dimension build runs its world without the node, as before; the only change on its path is one byte on a few messages.
