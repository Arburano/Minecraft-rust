# 0003. Own renderer on wgpu with three paths

Status: Accepted (2026-11-15)

## Context

Target — 60 FPS on iGPUs and the GTX 1050, but also on older GPUs without compute/indirect. `MULTI_DRAW_INDIRECT_COUNT` is available only on DX12 and Vulkan 1.2+; on GL ES 3.0/WebGL2 there are no storage buffers in the vertex shader and no `INDIRECT_EXECUTION`.

## Decision

We write the renderer ourselves on `wgpu`. A capability probe at startup selects the path: A — GPU-driven (compute culling + multi_draw_indirect_count), B — vertex pulling of 8-byte quads from a storage buffer with CPU culling, C — the same 8 bytes as a per-instance attribute. The data format is the same for all paths. Stages 0–1 build B and C; A comes after measurements.

## Consequences

- One data format, three drawing paths — the selection logic can be tested without a GPU.
- The main risk is missing vertices on iGPUs; it is measured first (T0-06).

## Alternatives considered

- Bevy's renderer — not built for voxel scale.
- GPU-driven only — excludes older GPUs.
- Raw OpenGL — worse cross-platform support and no Metal.

## References

docs/architecture/meshing-and-rendering.md.
