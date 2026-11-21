# T1-11. Terrain renderer: upload, culling, texture arrays, profiles

- **Status:** todo
- **Depends on:** T1-05, T1-07, T0-05
- **Owner:** agent (cloud) + owner's laptop for measurements

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

- [ ] GPU frame ≤ 8 ms for geometry at 12 chunks on the Vega 10 (owner's laptop run, 3 runs).
- [ ] CPU recording ≤ 1 ms per the statistic chosen in No. 14.
- [ ] Cave culling ≥ 50% in the cave test scene.
- [ ] Open questions 13 and 14 closed.

## Out of scope

Path A (stage 2+), entities/mob models, LOD (stage 5).
