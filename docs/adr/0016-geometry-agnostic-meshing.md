# 0016. Geometry-agnostic meshing and render buckets (readiness for non-cubic terrain)

Status: Accepted (owner, 2026-11-27)

## Context

- Today one mesher exists: binary greedy meshing of 32³ mesh chunks into 8-byte quads (ADR-0005, `mesh`), drawn by paths B/Bi/C (ADR-0003). Non-cubic block models (stairs, plants) are planned as a separate instanced path (`meshing-and-rendering.md`).
- The owner wants to keep the option of a "less cubic" world later: smooth terrain, procedural trees with individual leaves, real water surfaces. Experience from another voxel game (see [research](../research/external-projects-review-2026-11-26.md), section 4): micro-voxels for smooth terrain looked bad; what worked is Surface Nets / Dual Contouring over a density field, separate meshes for flora, and a layered water surface.
- If the 8-byte quad format and the cubic mesher are assumed everywhere (renderer, batch layout, physics, save format), adding any of this later means rewriting several crates.
- No product decision to make the world non-cubic exists; this ADR only keeps the door open at low cost.

## Decision

1. **A mesher boundary.** `mesh` exposes a `ChunkMesher` trait: input is the gathered, padded mesh chunk (block states, masks, and — when present — the density layer); output is written into **render buckets**. The current greedy mesher becomes `CubicGreedyMesher`, the only implementation for now. Which mesher a block uses is a block property in the registry (default: cubic).
2. **Render buckets by geometry type.** A render batch holds one range per bucket instead of assuming quads only:
   - `PackedVoxelQuads` — the current 8-byte quads (paths B/Bi/C), unchanged;
   - `ModelInstances` — instanced block-model templates (stairs, fences, plants), the already planned non-cubic path;
   - `SmoothMesh` — indexed vertices with position, normal, tangent and UV (smooth terrain, procedural trees, impostors) — **not implemented** until a decision to use it;
   - `WaterSurface` — a displaceable water grid for waves — **not implemented** until a decision to use it.

   Each bucket has its own pipeline per render path and its own vertex hook in the render graph (ADR-0015). Empty buckets cost nothing.
3. **Physics never reads visual meshes.** Collisions, raycasts and server simulation use block collision shapes from the registry (as T1-12 does now). Any future smooth or detailed geometry gets a simplified collision representation (voxel shapes or a coarse BVH) built separately. A million leaves or five layers of waves never reach the server.
4. **A reserved place for density, no format change now.** World format v1 stays. When smooth terrain is decided, format v2 adds one length-prefixed blob of optional per-section layers after the entity blob (first layer: `density: i8` per cell, absent for most sections), with a `v1 → v2` migration that writes an empty blob. In memory the layer is `Option<Box<[i8; 4096]>>`, allocated only for sections that use it. Nothing is allocated or stored until then.

## Consequences

- **Simpler:** a new mesher or a new geometry type is a new trait implementation and a new bucket, not a rewrite of `render`, `sim` and `world`.
- **Harder:** the batch layout gets per-bucket ranges (small change in `RenderBatch` and `RangeArena` bookkeeping); the trait must not add a virtual call per cell — dispatch is per mesh chunk, and the hot loop stays monomorphic (AGENTS.md rule 3; a before/after benchmark of `mesh_chunk` is required).
- **Not decided here:** whether the world becomes non-cubic, which smooth mesher, mesh shaders. Each is a later ADR (open question 18).
- **Tasks:** T1-23 (trait and buckets, after acceptance). It must show no regression in the `mesh` benchmarks.

## Alternatives considered

- **Do nothing until non-cubic is decided.** Cheapest now, but the quad-only assumption spreads with every renderer task (T1-19, T1-22) and costs more to undo later.
- **Micro-voxels (subdivide blocks) for smooth terrain.** Already shown to look bad on slopes and diagonals; it also multiplies storage.
- **Store density for every section now.** 4 KB per section for a feature nobody uses yet; against the RAM budget.
