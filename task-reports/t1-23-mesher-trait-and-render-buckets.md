# T1-23. Mesher trait and render buckets — report

Cloud part done 2026-11-28. Branch `ccr-b8153911-k3g0kc`, built on top of the not-yet-merged branch `ccr-1493f683-03fpg7` (ADR-0016 and this task file exist only there). Still open: the owner's laptop frame check.

## What was done

- **Registry property** (`world`): `world::MesherKind` (only value `Cubic`, the default), a new `BlockProps::mesher` field, and the datapack key `mesher = "cubic"`. Unknown values such as `"smooth"` fail to parse. Documented in `docs/modding/datapacks.md`. No world format change: the ID table and save format stay the same (ADR-0016, decision 4).
- **Mesher boundary** (`mesh::mesher`):
  - `ChunkMesher` trait with `KIND`, `BUCKETS` and `mesh(MeshInput, &mut MeshChunkQuads)`. `MeshInput` is the gathered padded voxels plus both masks.
  - `CubicGreedyMesher` wraps `Mesher<32>::fast_mesh` and is the only implementation.
  - `RenderBucket` declares four buckets: `PackedVoxelQuads` and `ModelInstances` are implemented; `SmoothMesh` and `WaterSurface` are declared only.
  - `ModelInstance` is a u64 with the same 6-bit x/y/z fields as `Quad`, so the batch 0/32 offset is the same XOR.
- **Builder dispatch**: `MeshChunkBuilder::remesh_into` calls the concrete cubic mesher, once per mesh chunk. `remesh_with<M: ChunkMesher>` takes any other mesher with static dispatch, and `input()` exposes the gathered `MeshInput`. The per-cell loop has no virtual calls.
- **Buckets in meshes and batches**:
  - `MeshChunkQuads` gets a `models` bucket. `is_empty()` covers both buckets, `len()` still counts quads, and `bucket_len()` is new.
  - `RenderBatch::set_chunk` copies models with the slot offset, and `pack_into` concatenates them into `PackedBatch::models`.
- **Renderer** (`render::terrain`):
  - `GpuBatch` holds per-bucket ranges: `quads` with the six direction sub-ranges, and `models` from a second `RangeArena`.
  - New `TerrainError::ModelBufferFull` and `FrameStats::models_resident`.
  - Model instances are bookkeeping only: there is no block-model pipeline yet, and no mesher produces them.
- **Tests**:
  - `mesh`: model-instance pack round trip, the list of implemented buckets, a test mesher through `remesh_with` (models only, no quads), and slot offsets plus pack order for models in batches.
  - `world`: the `mesher` key parses, defaults to cubic, and rejects unknown values.
  - `sim/tests/collision_source.rs`: `sim` and `server` do not depend on `mesh` or `render`. Changing a block's visual properties (opaque/transparent masks, light) leaves `MapBlocks::collides` and a fan of 64 raycasts unchanged. Changing only the collision shape changes them.
- **Checks run**:
  - `cargo fmt --all` and `RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets`: clean.
  - `cargo test -p mesh -p world -p sim -p render`: green.
  - T1-21 golden images on software Vulkan (llvmpipe, Mesa 25.2.8, installed in the container): 0.000 % of pixels differ, max channel difference 0, for every camera on B, Bi and C and for the warm-glow pack.
  - `cargo test --workspace`: green (with software Vulkan installed).

### Benchmarks (VM, cloud container, 4 vCPU, criterion; not bare metal)

Interleaved A/B runs: the old and new `mesh_chunk` bench binaries alternated, 3 rounds, pinned to one CPU (`taskset -c 2`), `--measurement-time 5`. The numbers are the criterion point estimate per round.

| Bench | Before (3 rounds) | After (3 rounds) |
| --- | --- | --- |
| `mesh_chunk/gather_masks` | 108.0 / 102.0 / 101.6 µs | 100.8 / 96.1 / 101.0 µs |
| `mesh_chunk/fast_mesh` | 49.4 / 51.0 / 51.6 µs | 49.3 / 49.0 / 53.0 µs |
| `mesh_chunk/build` | 146.7 / 151.9 / 145.8 µs | 143.6 / 150.3 / 155.3 µs |
| `mesh_chunk/remesh` | 49.5 / 48.8 / 47.9 µs | 51.2 / 53.9 / 53.3 µs |
| `mesh_chunk/mask_update` | 3.08 / 3.28 / 3.18 ns | 3.26 / 3.42 / 3.73 ns |
| `mesh_chunk/mask_rebuild` | 40.9 / 40.9 / 46.2 µs | 42.5 / 43.7 / 46.4 µs |
| `render_batch/edit_subrange` | 173 / 165 / 171 ns | 166 / 179 / 173 ns |
| `render_batch/pack_all` | 1.44 / 1.39 / 1.46 µs | 1.43 / 1.42 / 1.41 µs |
| `render_batch/edit_recopy_all` | 1.61 / 1.63 / 1.61 µs | 1.64 / 1.58 / 1.57 µs |

`remesh` looked ≈ 9 % slower in this run. In 4 earlier rounds of `remesh` alone (pinned, `--measurement-time 8`) both binaries were equal: 50.7 / 52.4 / 48.0 / 49.9 µs before, 51.3 / 49.0 / 50.4 / 49.2 µs after. To separate real cost from VM noise, I counted instructions with callgrind (`--toggle-collect=*remesh_into*`) over 200 remeshes of a fixed synthetic chunk with 3 954 quads: 170.81 M before, 171.02 M after. That is +0.12 %, about 1 k instructions per remesh (building `MeshInput` and clearing the empty `models` list). There is no regression beyond noise; the wall-clock spread on this VM is ±5–10 %. Bare-metal numbers come with the owner's laptop check.

## What went wrong

- **First version had a generic builder.** `MeshChunkBuilder<M: ChunkMesher = CubicGreedyMesher>` measured a real ≈ 10 % slower `remesh` in pinned A/B runs (48–50 µs → 55–73 µs). `fast_mesh` alone did not change. `#[inline(always)]` on `CubicGreedyMesher::mesh` made it worse (up to 73 µs). The likely cause is that generic builder methods are compiled in the calling crate with different inlining. I reverted to a non-generic builder, with the cubic mesher as a field and `remesh_with<M>` for other meshers. Pinned runs and callgrind then matched the baseline.
- **Golden-image test skipped at first.** The first `cargo test -p render` passed only because `reference_scene` printed "skipped: no GPU adapter". I installed `mesa-vulkan-drivers` (lavapipe, as CI does) and re-ran it; it then really compared.
- **The new sim test had wrong assertions twice.** With collision `None`, rays still "hit": first the solid floor below the loaded column, then unloaded cells next to it (both count as solid for physics). The assertion now checks that no hit is one of the placed blocks.

## Issues encountered

- **Base branch.** `main` does not have ADR-0016 or this task file yet. They are on `ccr-1493f683-03fpg7`, which also holds T2-01…T2-08, T2-10, T1-17, T1-18, T1-21 and T1-24 and has no PR. This branch fast-forwarded onto it, so it contains all of that work plus the T1-23 commit. Merge `ccr-1493f683-03fpg7` first, or review only the last commit. Earlier runs of this schedule also left duplicate T2-01/T2-02 branches (`ccr-b8153911-ak534e`, `ccr-b8153911-57bvwh`) that the work on `ccr-1493f683-03fpg7` replaces.
- **Model instances are not drawn.** The renderer accounts for their ranges, but no GPU buffer or pipeline exists. This is intentional: no mesher produces them until block models land (planned gameplay polish). That work adds the instance buffer, a pipeline per render path, and the vertex hook in the render graph (ADR-0015, T1-22).
- **Mesher selection is not read during gather.** Every block is `Cubic` today, so `remesh_into` always calls the cubic mesher. When a second `MesherKind` exists, the gather must record which kinds a chunk contains (per section, not per cell) and dispatch once per kind.
- **`MeshChunkQuads` keeps its name** even though it now holds more than quads, to avoid churn in `client`. Rename it when a third bucket is implemented.
- **Open acceptance criterion:** frame numbers on the laptop, unchanged within noise. The terrain draw path is the same except for one extra arena per renderer and one extra range per batch. Owner: run `scripts/stage1_exit.ps1` (the T1-15 bench route) once and compare with the recorded T1-15 numbers.
