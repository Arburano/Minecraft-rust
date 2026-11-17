# T0-05. Render batches of 2×2×2 mesh chunks

- **Status:** done
- **Depends on:** T0-04
- **Owner:** agent

## Goal

Assemble quads of 8 mesh chunks of 32³ into a 64³ batch (offset 0/32 along each axis), with 6 ranges by face direction. Compare two update variants after an edit: re-copying the whole batch vs. 8 sub-ranges (open question No. 8).

## Specs

- [docs/architecture/meshing-and-rendering.md](../../docs/architecture/meshing-and-rendering.md)
- [docs/open-questions.md](../../docs/open-questions.md)

## Deliverables

- `mesh::RenderBatch` with packing tests (decoding a quad gives the correct world position).
- Benchmark of both variants; recommendation in the PR description.
- Update to open-questions.md (No. 8) with the result.

## Acceptance criteria

- [x] Decoded position of each quad in the batch = position in the mesh chunk + offset (proptest).
- [x] Re-copying the batch ≤ 50 µs, or the chosen variant is justified with numbers. (VM: full re-copy ≈ 1.6 µs, 4 976-quad batch; see [task report](../../task-reports/t0-05-mesh-render-batches.md).)

## Out of scope

No upload to the GPU.
