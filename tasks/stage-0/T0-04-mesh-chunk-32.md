# T0-04. Gather, incremental masks and fast_mesh for the 32³ mesh chunk

- **Status:** todo
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

- [ ] Visible geometry of `fast_mesh` equals the reference (proptest on random mesh chunks; compare covered faces, not the quad count).
- [ ] Numbers for gather+masks, `fast_mesh`, and mask update recorded in performance-budgets.md.
- [ ] An edit on a mesh chunk boundary marks the correct neighbors as dirty (test).

## Out of scope

No GPU and no render batches (T0-05).
