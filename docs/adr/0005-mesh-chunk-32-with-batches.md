# 0005. 32³ mesh chunk + 2×2×2 render batches

Status: Accepted (2026-11-15, owner decision)

## Context

`binary-greedy-meshing` 0.6.4 supports sizes up to 62. A 62³ unit does not divide into 16³ sections (mesh and data boundaries do not line up); a 32³ unit lines up with 2×2×2 sections, but yields ≈ 7× more units and draw commands (≈ 4 500 with 12 chunks without indirect, estimate). The crate's quad encodes coordinates in 6 bits (0..63).

## Decision

We mesh in 32³ mesh chunks (`Mesher<32>`, `fast_mesh`, incremental masks). The quads of eight neighboring mesh chunks are copied into a 64³ render batch with an offset of 0/32 along each axis, and drawn with one command per face direction. On path A (multi_draw_indirect) batches are not required.

## Consequences

- Remeshing after an edit — ≈ 7× fewer voxels than with 62³.
- Number of draw commands — same as with 62³ (≈ 600).
- A copy step into the batch is added (target ≤ 50 µs) — benchmark T0-05.
- Compile-time check: `MESH_CHUNK_SIZE * RENDER_BATCH_MESH_CHUNKS <= 64` in `crates/world`.

## Alternatives considered

- 62³ — mismatched boundaries with sections, more complex gather and edits.
- 48³ — also not a multiple of the 64 batch size, same boundary problems.
- 16³ — too many units and draws.

## References

PRD §4, §14; docs/architecture/meshing-and-rendering.md.
