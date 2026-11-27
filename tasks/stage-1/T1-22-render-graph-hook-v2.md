# T1-22. Programmable render graph and shader-pack hook version 2

- **Status:** todo ([ADR-0015](../../docs/adr/0015-programmable-render-graph.md) accepted 2026-11-27)
- **Depends on:** T1-19, T1-21
- **Owner:** agent (cloud) + owner's laptop

## Goal

Implement the render graph from [`render-graph.md`](../../docs/architecture/render-graph.md): the engine's stages (setup, shadow, opaque G-buffer, deferred, translucent, composite, final), declared render targets, vertex hooks with bounded displacement, and shader packs with `hook_version = 2`. Hook version 1 packs keep working.

## Deliverables

- `render::graph`: pass and target declarations, validation (pure, tested without a GPU), target allocation with aliasing, barriers.
- The built-in Low (forward) and Default graphs; Default must reproduce the hook version 1 look on the T1-21 scene within tolerance.
- Vertex hooks in the engine's vertex shaders for paths B/Bi/C; culling boxes grown by `max_displacement`.
- Shadow pass with cascades, drawing shadow casters from the same quad buffers.
- The `PackFrame` uniform block, previous-frame matrices, TAA jitter.
- GPU timestamps per stage shown in the shader-pack menu; the over-budget notice.
- A sample version 2 pack in `docs/examples/shaderpacks/` (waving leaves, sun shadows, simple water tint) with golden images.
- `docs/modding/packs.md` updated with hook version 2.

## Acceptance criteria

- [ ] Hook version 1 packs and the built-in look are unchanged on the T1-21 golden images.
- [ ] With the built-in Default graph, the stage 1 exit numbers still hold on Vega 10 (Low profile) and the frame budget holds on the GTX 1660 Ti (Default); before/after numbers in the report.
- [ ] CPU command recording stays ≤ 1 ms p95 with the sample version 2 pack.
- [ ] A pack with an invalid graph or shader falls back to the built-in look with a message naming the pass.

## Out of scope

Iris-format packs (open question 17), mesh shaders, ray queries, `SmoothMesh` and `WaterSurface` buckets (ADR-0016).
