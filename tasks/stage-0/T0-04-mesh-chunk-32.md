# T0-04. Gather, incremental masks and fast_mesh for the 32³ mesh chunk

- **Status:** done
- **PR:** [#6](https://github.com/Arburano/Minecraft-rust/pull/6)
- **Depends on:** T0-03
- **Owner:** agent

## Goal

In the `mesh` crate: gather 34³ from sections (word-by-word unpacking), opaque/transparent masks with incremental update on `set_block`, call `binary-greedy-meshing` 0.6.4 `Mesher<32>::fast_mesh`, skip empty and solid mesh chunks. Starting point — `research/bench/meshing` (there the unit is 62³).

## Specs

- [docs/architecture/meshing-and-rendering.md](../../docs/architecture/meshing-and-rendering.md)
- [docs/adr/0005-mesh-chunk-32-with-batches.md](../../docs/adr/0005-mesh-chunk-32-with-batches.md)

## Deliverables

- API: `MeshChunkBuilder::build(&SectionNeighborhood) -> MeshChunkQuads` and `MaskCache::on_block_set`.
- Naive reference mesher for tests (count of visible faces must match).
- Benchmark on a fixed corpus (synthetic terrain with seed, 8% transparent).

## Acceptance criteria

- [x] Visible geometry of `fast_mesh` equals the reference (proptest on random mesh chunks, 512 cases + 512 edit-sequence cases; covered faces compared, not the quad count).
- [x] Numbers for gather+masks (≈ 60 µs), `fast_mesh` (≈ 42 µs), and mask update (≈ 2.6 ns) recorded in performance-budgets.md — measured on a VM, marked as such; see [task report](../../task-reports/t0-04-mesh-chunk-32.md).
- [x] An edit on a mesh chunk boundary marks the correct neighbors as dirty (`affected_mesh_chunks`, unit tests for interior/face/edge/corner and negative coordinates).

## Out of scope

No GPU and no render batches (T0-05).
