# T1-24: modular base game — split `core` into modules and add builds

## What was done

Owner request (2026-11-27): split `core` so each world and the creatures live separately, keep later features (moving structures with collision, space, role-play roles and permissions) as documented plans, and make builds from different sets of modules. Decision recorded as [ADR-0017](../docs/adr/0017-modular-base-game-and-builds.md).

**Loader** (`crates/world/src/modset.rs`, exported as `world::ModSet`):

- reads `builds/<id>.toml` (`[build] id, name, description, mods`) and the `mod.toml` of every folder under the mods root;
- adds dependencies, checks version requirements (`^a.b.c`, `=a.b.c`, `*`; optional dependencies only when present), orders topologically with ties by id;
- reports missing mods, version mismatches, cycles, duplicate ids and files no mod provides (`ModSetError`);
- `registry_builder()` merges every mod's `data/blocks` under its own id; `read(file)` takes a file from the last mod in load order that has it;
- `locate(build, mods_dir)` for binaries (next to the executable, else the source tree); `source_build(id)` for tests and examples.

**Modules** (`mods/`):

| Module | Content now |
| --- | --- |
| `core` (0.1.0) | the 15 shared blocks, default entity limits |
| `movement` | `player/physics.toml` (moved from `core`) |
| `overworld` | `worldgen/overworld.toml` (moved from `core`) |
| `nether`, `end`, `mobs` | manifests only, with what each will own |

`builds/classic.toml` = `core`, `movement`, `overworld`.

**Engine:**

- `ServerConfig::mods`, `SessionConfig::mods` and `BotConfig::mods` replace the data-folder paths;
- `server`, `client` and `bots` take `--build` (default `classic`) and `--mods-dir`;
- `EntityLimits::from_mods` merges `entities/limits.toml` over the build (a mobs module will add its kinds);
- `client::textures::read_mod_faces` reads block faces from every mod.

All tests, examples and benchmarks that read `mods/core/data` now use the build.

**Docs:** `docs/modding/builds.md` (build format, merge rules, module catalogue with responsibility, size and tick budget, rules for cutting modules, planned modules and example builds); ADR-0017; ADR-0013 task (a) note; T1-18 (the split part is done here); `mods/README.md`, `AGENTS.md`, `architecture/overview.md`, `worldgen.md`, `simulation-and-ai.md`, `server-regions.md`, `mod-manifest.md`, `coding-standards.md`, T1-17.

**Tests:** unit tests for version requirements, dependency order and optional dependencies, every error case, data provided by the last mod, and that every shipped build resolves and builds a registry. The determinism golden hash and the stage 1 save migration test are unchanged, so existing worlds open as before. `cargo fmt --all`, `cargo clippy --workspace --all-targets` with `-D warnings`, `cargo test --workspace` (cloud VM, 4 vCPU).

## What went wrong

- A scripted rewrite of ~30 call sites left some `mods` / `data` variables out of scope (light, worldgen tests, `map_preview`); fixed by hand after `cargo check`.

## Issues encountered

- **Block ids stay `core:*`.** Grass, sand, ores and wood are used by the overworld, but every planned build needs them too, and renaming would break saved ID tables. Blocks of a single world go to that world's module (`nether:*`, `end:*`).
- **ADR-0013 names.** T1-18 planned `core-movement` and `core-worldgen`. Mod ids allow only `[a-z0-9_]`, and generation is per dimension, so the modules are `movement` and `overworld`. ADR-0017 states this; `provides` and the slot checks stay in T1-18.
- **Not done (listed in builds.md):**
  - the build id is not stored in the world;
  - the server does not send its mod list (`mod_manifest_hash` is zero), so the client and the server must run the same build;
  - the updater does not download per build.
- **Test walkers' limit** (`test:walker`) stays in `core` because the engine's test entity is not gameplay; it moves out with the test entities.
- **Shipped binaries** look for `mods/` and `builds/` next to the executable; packaging (T1-13/updater) must copy both folders.
