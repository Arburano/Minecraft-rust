# Moving structures (grids)

Decision: [ADR-0019](../adr/0019-moving-block-structures.md) (Proposed). Capability: `moving_grids` ([ADR-0018](../adr/0018-engine-capabilities-per-build.md)). Background: [review of Create: Aeronautics and Sable](../research/moving-structures-review-2026-11-27.md). Status: design; nothing implemented (stage 4).

Goal: players build ships, aircraft, rockets and vehicles out of blocks; once assembled they fly, float, drive and crash with real physics, the machines on board keep working, and players walk on deck while they move.

## Data model

```text
Grid
  id: GridId (u64, unique per world)
  sections: sparse map SectionPos(local) -> world::Section     # same storage as the world
  pose: { position: DVec3 (world, f64), rotation: Quat (unit) }
  velocity: { linear: Vec3, angular: Vec3 }                      # m/s, rad/s
  mass: { total: f32, center: Vec3 (local), inertia: Mat3 (local, about center) }
  bounds: local AABB of all blocks
  state: Active | Sleeping | Frozen (over budget)
  attached: [EntityId]                                           # entities in the grid frame
```

- **Local coordinates:** block `(0,0,0)` of the grid is the assembly seed. The world transform maps a local point `p` to `position + rotation × (p − center) + center_world_offset`; the stored pose is that of the center of mass so that Rapier and the grid agree.
- **Units:** one block = 1 m; mass in arbitrary "kg"; 20 TPS tick.
- **No column writes:** while a grid exists, its blocks are not in any column. World queries (raycasts, explosions, mob pathfinding) check grids near the query explicitly through the grid index of the region.

## Block physics data

Each block file may set a `[physics]` table; tags set presets.

```toml
[physics]
mass = 1.0            # default from the collision shape volume × 1.0
inertia = [0.1667, 0.1667, 0.1667]   # multiplier per axis (uniform cube)
volume = 1.0          # displaced volume for buoyancy, m³
friction = 0.8
restitution = 0.0
fragile = false       # breaks when an impact exceeds `break_energy`
break_energy = 400.0
lift = 0.0            # floating force per block at pressure 1 (floating blocks)
```

- Defaults are derived from the block's collision shape, so most blocks need nothing.
- The engine reads `lift` only as data: a lifting block applies `lift × pressure(altitude)` upward. Richer lift rules (balloons that need heat, propellers that need rotation speed) are mods.
- Physics tags: `physics:light`, `physics:heavy`, `physics:slippery`, `physics:bouncy`, `physics:structural` (counts as connected for assembly).

## Physics step

Per region, inside the region tick (ADR-0008), only when the `moving_grids` capability is on and the region has grids:

1. Collect forces: gravity (dimension), drag (dimension `drag` × velocity², per grid), buoyancy (submerged block volume × fluid density × gravity, applied at the submerged centroid), lift blocks, and the batched mod forces of this tick (`apply-force`, `apply-torque`).
2. Terrain colliders: for each grid, the sections its swept bounds touch (current → predicted pose, plus 1 block) are inserted as static colliders if not cached; cache by `(section, version)`; dropped when no grid is near.
3. Step Rapier at 20 Hz with 3 substeps (CCD on for fast grids).
4. Write back poses and velocities; update attached entities (next section).
5. Contacts above the break energy break `fragile` blocks (queued as edits, applied in the region's edit phase); report impacts to mods as events.
6. Grids at rest sleep (Rapier islands); sleeping grids cost nothing.

### Colliders from blocks

- One compound collider per grid section: the solid mask (collision shapes that are full cubes) is merged into boxes greedily (same idea as greedy meshing); non-cube shapes (slabs, stairs) add their boxes.
- On an edit, only the touched section's compound is rebuilt (target ≤ 50 µs), and mass properties are updated by adding or removing that block's contribution.
- If a Rapier voxel shape type is available in the version we pin, it replaces the box merge (compare both with a benchmark).

## Entities on grids

- **Attaching:** an entity is attached to a grid when it stands on a grid block, is inside the grid's bounds and touching its blocks, or is seated (a mod's seat block). Detached when it has not touched the grid for 10 ticks.
- **In the grid frame:** an attached entity's position is local. Its movement runs the T1-12 sweep against the grid's blocks in local coordinates, plus a frame force from the grid's acceleration (so a sudden stop throws players forward, softened by a datapack factor for comfort). Collisions with the world are checked in world space after the transform.
- **Network:** attached entities are replicated relative to the grid (`EntityUpdate` gains a `frame: GridId` field); the client interpolates them in the grid frame, so a player on a turning ship does not jitter.
- **Prediction:** the local player predicts its own movement in the grid frame and the grid's pose by extrapolation; corrections reconcile in the grid frame (T2-03 machinery, with the frame as part of the state).
- **Saving:** a player who logs out on a ship is saved with its grid id and local position, and logs in there.

## Network

| Data | How |
| --- | --- |
| grid blocks | like columns: a grid record (sections in the T1-02 format, zstd) when a player comes in range; ForgetGrid when it leaves |
| pose, velocity | one replicated object per grid in `EntityDatagram`, 20 Hz, interpolated; extrapolated at most one interval |
| block edits in a grid | T2-04 edits with a `GridId`; `BlockChanges` with a grid id |
| assembly, disassembly, split | reliable events: GridCreated (with its record), GridRemoved, GridSplit |

Budget: a 1,000-block grid record ≈ 2–6 KB compressed (like a column, measured later); pose updates ≈ 30 bytes per grid per tick when moving, nothing when sleeping.

## Rendering

- Grid sections are meshed by the normal mesher in local space (ADR-0005), uploaded to their own batch, and drawn with a per-object model matrix (a render-graph feature, ADR-0015).
- Light: grid sections have their own block light; sky light is taken from the world column at the grid's position (an approximation; shadows from render-graph passes).
- Far grids are drawn as one LOD mesh (a later task).

## Regions and limits

- A grid belongs to the region that holds its center of mass. Its swept bounds plus 2 sections count as an occupied area in the partition (T2-06), so regions merge before a grid reaches another region's terrain.
- Handoff between regions moves the grid, its attached entities and its Rapier body in the Exchange phase.
- Limits per region from `data/grids/limits.toml` (merged over the build, like entity limits): `max_grids`, `max_blocks_per_grid` (start 32,768), `physics_ms_per_tick` (start 2.0). Over budget: the grids farthest from players are frozen (kept walkable), then woken when there is time.

## Assembly and disassembly

- **Assembly** (requested by a mod from a seed block): flood fill over face-connected blocks that are not air or fluid, through blocks the mod accepts (its glue rules); stop at `max_blocks_per_grid` with an error. In one region transaction: create the grid, copy blocks, sections and block entities, replace them with air in the world, send GridCreated.
- **Disassembly:** snap the rotation to the nearest of the 24 axis-aligned orientations and the position to the block grid; if every target cell is air or replaceable, write the blocks (rotated) into the world and remove the grid; otherwise refuse with the reason.
- **Splitting:** after removing blocks, a connectivity check from the removed cells' neighbors; separate parts become grids with the parent's velocity at their centers.

## Mod API (`engine:api/grids`, linked only with the capability)

- `assemble(seed: block-pos) -> result<grid-id>`, `disassemble(grid) -> result`
- `apply-force(grid, local-point, force)`, `apply-torque(grid, torque)` — batched per tick
- `grid-state(grid) -> { pose, velocity, mass, altitude, pressure, submerged }`
- `grid-blocks(grid)`: the same block access interface as the world, in local coordinates
- events: `grid-impact { grid, other, energy, point }`, `grid-split`, `grid-sleep`

Gameplay built on it (mods, not the engine): propellers and bearings (`contraptions`), balloons and burners, floating blocks with conditions, jet and rocket engines with fuel (`rocketry`), gyroscopes and autopilots, seats and controls.

## Tests (for the implementation tasks)

- Determinism: same forces → same poses on the same build (golden after N ticks; Rapier's `enhanced-determinism` feature).
- Mass properties against a reference sum (a reference implementation, rule 4).
- Assembly → disassembly round trip leaves the world identical.
- Walking on a rotating grid: the player stays on deck at 1 rad/s and 30 m/s without falling through or sliding (feel test, owner review).
- Collider rebuild and physics step benchmarks with the targets above.
