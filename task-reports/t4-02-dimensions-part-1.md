# T4-02 (part 1): dimension definitions

## What was done

- **`world::dimension`**: `DimensionDef`, the `data/dimensions/<id>.toml` format of `dimensions.md`:
  - id, generator file, height, sky (`day_night` | `none` | `fixed`);
  - physics: gravity, `player_gravity_scale`, pressure, `pressure_by_altitude`, drag, fluid density.

  `from_toml` checks:
  - the id is `[a-z0-9_]`;
  - the height is multiples of 16, low below high, at most 4096 blocks;
  - physics values are in range and pressure heights increase;
  - the generator is a `.toml` inside the module.

  Helpers: `pressure_at(y)`, `bounds()` and `slot()` (`worldgen:<id>`).
- **`dimension::from_mods(&ModSet)`** collects the build's definitions. An id defined by two mods is an error naming both. Each dimension needs a mod filling its `worldgen:<id>` slot, and that mod must have the generator file.
- **Data:** `mods/overworld` and `mods/overworld_plus` define the `overworld` dimension, with Earth-like physics and the default height.
- **Server and client:**
  - read the dimensions at start (an invalid definition stops the start with its message);
  - apply the overworld's `player_gravity_scale` to the movement profile (`PhysicsConfig::with_gravity_scale`; the same on both sides, so predictions agree; ×1 is a no-op).
- **Docs:** `dimensions.md` ("Implementation (T4-02 part 1)").

**Tests:**

- `world::dimension` (3): the documented format and the pressure curve; refusals of bad height, sky, pressure, pressure order, id and path; the shipped builds define exactly the overworld.
- `sim`: a 0.38 scale falls more slowly, and ×1 leaves the profile equal.
- The classic movement golden trace and the server determinism golden are unchanged.
- `cargo fmt`, clippy with `-D warnings`, `cargo test --workspace`.

## What went wrong

None.

## Issues encountered

- **Acceptance is not met yet:** two dimensions running at once with travel between them is part 2. It touches the server's world thread (one region set per dimension), storage under `dimensions/<id>/`, the Exchange phase for travel, and the protocol (a dimension id on `Spawn`, columns, entities and edits, so a protocol version bump), plus per-dimension maps on the client.
- **Storage choice:** the overworld stays at the world root, so existing worlds need no migration. New dimensions go under `dimensions/<id>/`.
- **The `dimensions` capability** is not required by any shipped mod yet: one dimension needs no capability. Part 2 starts the multi-dimension machinery only when a mod requires it (ADR-0018).
