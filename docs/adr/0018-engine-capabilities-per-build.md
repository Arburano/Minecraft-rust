# 0018. Engine capabilities activated by builds

Status: Accepted (owner, 2026-11-27)

## Context

- ADR-0017 made the base game modular: a build lists mods, and data, assets and WASM code of mods outside the build are never installed or loaded.
- The planned builds need engine features that mods cannot implement themselves, because they need engine speed or reach into the world, physics, network and renderer:
  - the **industrial build**: several worlds and planets, space, giant moving structures with physics and collision, rocket thrust;
  - the **RPG build**: a giant world with zones, instanced and evolving dungeons, player progression stored by the server.
- These features are native engine code. If every build starts all of them, a build pays for features it never uses: threads, memory, per-tick systems, bigger downloads. The owner's requirement: code a build does not use must not run, and if possible not ship.

## Decision

1. **Capabilities.** An optional engine feature is a *capability* with a stable name. A mod declares the capabilities it needs in `mod.toml`:

   ```toml
   [requires]
   capabilities = ["dimensions", "moving_grids"]
   ```

   The build's capability set is the union over its mods (`world::ModSet`). The server and the client start only the subsystems of that set. A capability nobody asked for has no thread, no allocation, no ECS system in the tick schedule, no protocol messages and no render pass.
2. **Initial list** (each gets its own design document before implementation):

   | Capability | What it starts | Needed by | Spec |
   | --- | --- | --- | --- |
   | `dimensions` | more than one world per server; travel between them; physics per dimension | industrial, RPG | [dimensions.md](../architecture/dimensions.md) |
   | `moving_grids` | moving block structures with rigid-body physics and collision | industrial | [ADR-0019](0019-moving-block-structures.md), [moving-structures.md](../architecture/moving-structures.md) |
   | `space` | large-distance coordinates (per-dimension origin shifting), vacuum, orbit-scale travel | industrial | dimensions.md, "Space" |
   | `zones` | named world areas with rules (PvP, build rights, spawn tables) | RPG, role-play | PRD §14 No. 11 |
   | `instances` | private copies of an area per party, created and dropped on demand | RPG | builds.md, "RPG build" |
   | `progression` | server-stored per-player stats, levels, skills with a mod API | RPG | builds.md, "RPG build" |

   Always on, not capabilities: the world, terrain generation, lighting, the player, entities and pathfinding, the network, storage, the mod host.
3. **Two levels of "not used":**
   - *Not running* (the default): the code is in the binary but never started; it costs only disk space.
   - *Not shipped:* a capability with a heavy dependency (the physics engine for `moving_grids`) sits behind a cargo feature of the same name. A build's release binary is compiled with exactly its capabilities (the build tool reads the build file). Development builds have all features on.
4. **Errors are early and clear.** A mod that requires a capability the binary was compiled without fails at load with "build X needs capability Y; this binary was built without it". A world saved with a capability (e.g. it contains moving grids) refuses to open in a build without it instead of silently dropping data.
5. **The mod API follows.** WIT interfaces of a capability (e.g. `engine:api/grids`) are linked only when the capability is on; a mod importing them without requiring the capability gets a link error that names the capability (the existing permission rule of ADR-0006 applies the same way).

## Consequences

- **Simpler:** a lean build (RPG) carries no rigid-body physics; a server shows exactly which features are active; capabilities can be developed and benchmarked one by one.
- **Harder:**
  - subsystems must be truly optional: no capability may be assumed by core code, and every cross-cutting hook (network, save, render) checks for a started subsystem;
  - several release binaries (one per build) or one generic binary per platform; the updater (ADR-0010) must know which;
  - tests must run with all features and with none (CI matrix).
- **Not decided here:** the design of each capability (separate ADRs), the build tool that compiles a build's binary.

## Alternatives considered

- **Always compile and start everything.** Simplest, but every build pays for every feature, against the owner's requirement and the RAM budget.
- **Only cargo features, no runtime check.** Every build needs its own binary even in development; one development binary could not run two builds.
- **Implement these features as WASM mods.** Physics and world-scale features need engine speed and access to the region tick, the renderer and the network; WASM keeps the gameplay on top (ships, machines, skills), not the mechanisms.
