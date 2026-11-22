# T1-11. Terrain renderer: upload, culling, texture arrays, profiles

- **Status:** done (bare-metal laptop measurements and OQ 13/14 resolved 2026-11-22)
- **Depends on:** T1-05, T1-07, T0-05
- **Owner:** agent (cloud) + laptop confirmation

## Goal

Draw the real world: upload T0-05 render batches to GPU buffers, frustum + face-direction + cave culling, texture arrays with mipmaps, block light/sky light in the shader, quality profiles (Low/Default/High = 8/12/16 chunks). Decide the iGPU default path and the recording statistic.

## Specs

- [meshing-and-rendering.md](../../docs/architecture/meshing-and-rendering.md)
- [open questions](../../docs/open-questions.md) No. 13 and 14
- [GPU summary](../../docs/research/gpu/summary-2026-11-19.md)

## Deliverables

- Buffer management for batches (allocation without per-frame allocations), paths B and C as in T0-06.
- Culling: frustum, ≤ 3 face directions per batch, cave visibility graph (target ≥ 50% of units discarded in a cave).
- **Open question 13:** indexed/4-vertex B variant, B vs C re-measured on real terrain (T0-12 corpus) on the Vega 10; owner decides the default.
- **Open question 14:** CPU recording statistic decided by the owner with real-scene data.
- Quality profiles and their settings UI hooks.

## Acceptance criteria

- [x] GPU frame ≤ 8 ms for geometry at 12 chunks on the Vega 10 (measured: **1.52 ms** on Path Bi, **1.96 ms** on Path B, 3 runs on ASUS FX505DU).
- [x] CPU recording ≤ 1 ms per the statistic chosen in No. 14 — measured: **0.06 ms** median, **0.12 ms** p95 on laptop (≤ 0.11 ms on VM).
- [x] Cave culling ≥ 50% in the cave test scene — 76–77% of mesh chunks with geometry in the frustum discarded (camera in a generated cave at y −49.5, radius 8 and 12).
- [x] Open questions 13 and 14 closed (Path Bi selected for iGPU; CPU recording confirmed at 0.06 ms median / 0.12 ms p95).

## Laptop run (owner)

Three runs of each line on the ASUS FX505DU, plugged in, at 12 chunks. Paste the `first frame`, `next … frames` and `GPU pass` lines into the task report (or send them to the agent):

```bash
# Vega 10 — the decision for open question 13: B vs indexed B (Bi) vs C
ADAPTER_NAME=vega WGPU_BACKEND=dx12 FRAMES=300 cargo run --release -p client --example screenshot -- 12 vega-b.bmp B
ADAPTER_NAME=vega WGPU_BACKEND=dx12 FRAMES=300 cargo run --release -p client --example screenshot -- 12 vega-bi.bmp Bi
ADAPTER_NAME=vega WGPU_BACKEND=dx12 FRAMES=300 cargo run --release -p client --example screenshot -- 12 vega-c.bmp C
# the same three with WGPU_BACKEND=vulkan (needs VK_LOADER_LAYERS_DISABLE=VK_LAYER_AMD_switchable_graphics, T0-14) and WGPU_BACKEND=gl
# GTX 1660 Ti, for comparison: ADAPTER_NAME=1660 with the same lines
```

Also open one `.bmp` to check the picture: terrain, water, trees, light and fog as in the [cloud screenshot](../../docs/research/t1-11-terrain-screenshot.png).

## Out of scope

Path A (stage 2+), entities/mob models, LOD (stage 5).
