# T1-18. Gameplay slots: replaceable movement profile and generator

- **Status:** done (owner: feel review of the example `movement_agile` profile)
- **Depends on:** T1-12, T1-13
- **Owner:** agent (cloud)

## Goal

The data side of [ADR-0013](../../docs/adr/0013-replaceable-movement-and-worldgen.md) (accepted by the owner 2026-11-22). A pack replaces the movement profile or the world generator by shipping a different provider mod; unused providers are neither installed nor loaded.

## Deliverables

- ~~Split `mods/core`~~ — done by T1-24 ([ADR-0017](../../docs/adr/0017-modular-base-game-and-builds.md)) under the names `core` (shared blocks), `movement` (the movement profile) and one mod per dimension (`overworld`: the overworld generator config). What remains here is `provides`, the slot checks and the profile format.
- `mod.toml` gains `provides`. Loading checks that each slot (`movement`, `worldgen:<dimension>`) has exactly one provider, with clear errors.
- Movement profile format `movement/profile.toml`: the base numbers (today's `player/physics.toml`) plus a list of abilities built from engine primitives. The first primitives are impulse, set velocity, hold against a surface, change box size and gravity scale; each ability has a trigger, conditions, a cooldown and an animation state name. `sim::physics` runs a profile. The default profile reproduces today's movement exactly: the T1-12 determinism and feel tests stay green.
- At least one example ability in a test-only profile, for example a dash or a wall climb, with tests.
- Cargo feature `builtin-overworld` on `worldgen` (on by default) around the built-in stages.
- Slot choices are stored per world (`world.toml`) and checked when a world is opened with a different pack. Done in the server's `server.toml` (the world file of the embedded and the dedicated server): a different generator is refused, a different movement provider is reported.

## Acceptance criteria

- [x] A test build without `movement` / `overworld` loads only its own providers, and the loader reports a missing or duplicate provider clearly.
- [x] The default profile gives bit-identical T1-12 physics traces.
- [x] Docs: `docs/modding/` (slots, profile format), `simulation-and-ai.md`, `worldgen.md`.

## Out of scope

WASM abilities and generator stages (with the mod API, stage 3); the entity animation system.
