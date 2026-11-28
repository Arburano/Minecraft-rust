# T1-24. Modular base game: split `core` into modules and add builds

- **Status:** done
- **Depends on:** T1-13, T2-08
- **Owner:** agent (cloud)

## Goal

[ADR-0017](../../docs/adr/0017-modular-base-game-and-builds.md) (owner request, 2026-11-27): the base game is no longer one `core` mod. Each world, the movement and the creatures are separate modules, and a build lists the modules a game runs with, so that completely different builds can be assembled from different sets of mods and carry only what they use.

## Deliverables

- `world::modset`: build files, mod manifests with dependencies and version requirements, topological load order, merged blocks, provided data files, clear errors.
- Modules: `core` (shared blocks, default entity limits), `movement` (player physics), `overworld` (overworld generation), and manifests of the planned `nether`, `end`, `mobs`.
- `builds/classic.toml`; `--build` and `--mods-dir` in `server`, `client` and `bots`; `ServerConfig::mods`, `SessionConfig::mods`, `BotConfig::mods`.
- Entity limits merged over all mods of the build.
- Docs: `docs/modding/builds.md` (build format, module catalogue with responsibility and size, rules for cutting modules, planned modules: moving structures with collision, space, roles and permissions, machines; example builds).

## Acceptance criteria

- [x] The server and the client load blocks, generation, physics and limits from the build, not from a fixed path.
- [x] A build without the mod that owns a file fails with a clear error; missing mods, version mismatches, cycles and duplicate ids are reported.
- [x] Existing worlds open unchanged (block ids keep `core:*`); the determinism golden hash is unchanged.
- [x] `cargo test --workspace` green.

## Out of scope

Content of `nether`, `end`, `mobs` (stage 4); storing the build id in the world and the server sending its mod list (with T1-18 and the updater); `provides` and slot checks (T1-18).
