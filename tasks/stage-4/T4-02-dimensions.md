# T4-02. Dimensions

- **Status:** todo
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

## Acceptance criteria

- [ ] Two dimensions with different generators run at once; a player moves between them and back without losing items or position.
- [ ] Tick and memory unchanged within noise for a build with one dimension.
