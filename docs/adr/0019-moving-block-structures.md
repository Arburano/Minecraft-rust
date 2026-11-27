# 0019. Moving block structures: grids with rigid-body physics

Status: Proposed (2026-11-27; owner decision needed). Capability `moving_grids` (ADR-0018).

## Context

- The industrial build needs what Create: Aeronautics gives in its game: players build ships, aircraft, rockets and vehicles out of ordinary blocks, assemble them, and they move with real physics: weight, lift, thrust, inertia, collisions with terrain and with each other. Players walk on deck while the ship flies, and the machines on board keep working. Review: [moving-structures-review-2026-11-27.md](../research/moving-structures-review-2026-11-27.md).
- Aeronautics' physics library (Sable) does this with **sub-levels**: ordinary chunks with their own position and orientation, simulated by the Rapier physics engine (Rust), with per-block mass/inertia/volume/friction data and per-dimension gravity/pressure/drag. Its license (PolyForm Shield) allows ideas only; we do not use its code.
- Our engine already has what a sub-level needs in local coordinates: palette sections (`world::Section`), the mesher, light, the block registry with collision shapes, the T1-12 collision sweep, regions with their own ECS worlds (ADR-0008), entity replication (T2-07).

## Decision

1. **A grid is a small world with a pose.** A moving structure is a `Grid`:
   - its own sparse set of 16³ sections in **local block coordinates**, stored exactly like world sections;
   - a pose: position as `f64` (world space) and orientation as a unit quaternion;
   - linear and angular velocity;
   - mass properties summed from its blocks.

   Blocks inside behave as anywhere else: block entities tick, machines run, light spreads, mods see an ordinary block access API in the grid's frame. Meshes are built by the normal mesher in local space and drawn with the grid's transform. A grid is never written into the world's columns while it exists.
2. **Rigid-body physics with Rapier** (`rapier3d`, Apache-2.0, pure Rust; rule 5: the only maintained Rust physics engine with continuous collision detection, compound and voxel colliders and joints; behind the `moving_grids` cargo feature, so builds without grids do not ship it).
   - One Rapier world per **region** (ADR-0008), stepped inside the region's tick at a fixed 20 Hz with substeps. No cross-region physics state.
   - The terrain is **not** loaded into Rapier: near each grid, the region inserts static colliders only for the terrain sections its swept bounds touch, built from block collision shapes and cached per section version.
   - A grid's collider is built from its blocks' collision shapes, merged into boxes per section (greedy box merge on a solid mask), and updated incrementally when blocks change.
3. **Physics properties are block data.** Each block may set, in its block file or by tag, `mass`, `inertia` (multiplier per axis), `volume` (buoyancy), `friction`, `restitution`, `fragile` (breaks above an impact energy), and `lift` (floating blocks). Defaults come from the block's collision shape. Mass, center of mass and the inertia tensor of a grid are sums over its blocks (parallel-axis theorem), kept incrementally.
4. **Forces come from the dimension and from mods.**
   - Dimension physics (`dimensions.md`): gravity vector, air pressure by altitude (zero is vacuum), drag, fluid density. The engine applies gravity, drag and buoyancy (submerged block volume × fluid density × gravity).
   - Everything that *makes* things fly is gameplay in mods, through the `grids` mod API: `apply-force(grid, world-point, force)` and `apply-torque`, batched per tick. Propellers, balloons, jet and rocket engines, floating blocks and gyroscopes are mods (`contraptions`, `rocketry`), not engine code. The engine gives them the inputs: altitude, pressure, the grid's velocity, the block's local position.
5. **Players and entities ride grids in the grid's frame.** An entity standing on, inside or holding on to a grid is *attached*: its position is stored relative to the grid, its movement physics runs in grid-local coordinates (the T1-12 sweep against the grid's blocks, with the grid's acceleration as a frame force), and it is networked, interpolated and saved relative to the grid. Detaching converts to world space and adds the grid's point velocity. Player prediction works in the grid frame, so walking on a turning ship feels like walking on the ground.
6. **Network.** A grid's sections stream like columns when a player is in range. Its pose and velocity are replicated at 20 Hz through the entity channel (one grid = one replicated object) and interpolated on the client. Block changes inside a grid use the T2-04 edit path with a grid id. Clients do not simulate grid physics; they extrapolate the pose for at most one interval.
7. **Lifecycle.**
   - *Assembly*: a mod (with a block like a "physics bearing" or "assembler") gives a seed block. The engine collects the connected blocks (face-connected, through blocks a tag marks as structural; glue rules are the mod's), moves them into a new grid in one transaction and replaces them with air.
   - *Disassembly*: rotate the pose to the nearest axis-aligned orientation and snap to the block grid. If every target cell is free, write the blocks back; otherwise refuse and report.
   - *Splitting*: after a block is removed, the grid checks connectivity, and disconnected parts become separate grids.
   - *Destruction*: impacts above a threshold break `fragile` blocks; the dropped parts are items (stage 4).
8. **Limits (overload protection, T2-10 style).** Per region, from the datapack: maximum grids, maximum blocks per grid (start: 32,768), maximum physics time per tick. A grid over the time budget is put to sleep (frozen in place, still walkable) until there is time. Sleeping grids at rest cost nothing; Rapier's sleeping is used.
9. **Saving.** Grids are saved per region in a separate grid store next to the column store: sections in the T1-02 record format, pose, velocity, attached entities. The column format does not change.

## Consequences

- **Simpler:** everything blocks already do works on ships for free (machines, light, storage, mod logic); the physics is a mature library in Rust; gameplay (what lifts, what pushes) stays in mods and data.
- **Harder:**
  - **Crossing region borders**: a grid belongs to the region that holds its center, and its swept bounds (plus a margin) count as an occupied area in the partition (T2-06), so regions merge around it before it reaches another region's terrain. A grid larger than a region is out of scope (the limit above keeps grids smaller).
  - **Players on fast or rotating grids** need careful prediction in the grid frame; this is where the feel is won or lost (PRD §14: feel first).
  - **Rendering** needs per-object transforms in the render graph (ADR-0015), and light at grid borders mixes the grid's own light with the world's sky light (approximation: sky light from the world at the grid's position).
  - **Shards** (ADR-0009): a grid is migrated with its attached entities as one unit; that comes with stage 6.
- **Performance targets (proposed, not measured):** 50 active grids of 1,000 blocks each in one region within 2 ms of physics per tick; collider rebuild of one edited section ≤ 50 µs; 0 allocations per tick at steady state. A benchmark comes with the first task.
- **Tasks:** the capability is stage 4 work (roadmap: "contraptions"). Its prerequisites are `dimensions` (dimension physics) and per-object transforms in the renderer.

## Alternatives considered

- **Move the blocks through the world every tick** (classic piston-style contraptions). Rewrites columns at 20 Hz, breaks light and meshes constantly, no rotation, no real collision response.
- **Entities made of blocks without their own sections** (a list of blocks with one entity per block). Block entities and mod logic would need a second code path; meshing would be per block.
- **Our own physics engine.** Months of work to reach what Rapier already does (contacts, CCD, sleeping, joints); no reason to.
- **Bullet or PhysX through FFI.** Needs `unsafe` and a C++ toolchain (rule 4), for no gain over Rapier.
