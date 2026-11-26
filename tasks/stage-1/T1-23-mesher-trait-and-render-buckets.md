# T1-23. Mesher trait and render buckets

- **Status:** blocked (waits for the owner to accept [ADR-0016](../../docs/adr/0016-geometry-agnostic-meshing.md))
- **Depends on:** ADR-0016 accepted, T1-11
- **Owner:** agent (cloud)

## Goal

Remove the "quads only" assumption from `mesh` and `render` without changing what is drawn or how fast: a `ChunkMesher` trait with `CubicGreedyMesher` as the only implementation, and render batches that hold one range per render bucket (`PackedVoxelQuads`, `ModelInstances`; the other buckets are declared but not implemented).

## Deliverables

- `mesh::ChunkMesher` (dispatch per mesh chunk, monomorphic inner loop) and the block-registry property that selects the mesher (default cubic).
- Per-bucket ranges in `RenderBatch` / `RangeArena` bookkeeping and in `render::terrain`.
- A test that physics and raycasts never read mesh output (they use registry collision shapes only).
- No world format change (ADR-0016, decision 4).

## Acceptance criteria

- [ ] `mesh_chunk` benchmarks before/after: no regression beyond noise (state the machine and runs).
- [ ] T1-21 golden images unchanged.
- [ ] Frame numbers on the laptop unchanged within noise.

## Out of scope

Smooth meshers, water surface, density storage.
