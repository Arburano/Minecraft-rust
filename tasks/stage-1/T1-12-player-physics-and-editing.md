# T1-12. Player physics, digging and placing blocks

- **Status:** todo
- **Depends on:** T1-01, T1-09, T1-10
- **Owner:** agent (cloud)

## Goal

A player can walk, jump, fall, collide with blocks, dig and place blocks; each edit updates masks, light and meshes within one frame budget.

## Specs

- [simulation-and-ai.md](../../docs/architecture/simulation-and-ai.md) — Player physics
- [meshing-and-rendering.md](../../docs/architecture/meshing-and-rendering.md)
- [lighting.md](../../docs/architecture/lighting.md)

## Deliverables

- Player AABB physics in `sim` (fixed 20 TPS tick + client interpolation), collision against the world map, step-up, swimming placeholder.
- Block raycast, dig/place with the edit pipeline: `set_block` → masks → `relight` → remesh of affected mesh chunks → batch re-copy.
- Determinism tests for physics (same inputs → same positions); edit pipeline tests.

## Acceptance criteria

- [ ] Edit-to-visible latency ≤ 1 frame at 60 FPS on the owner's laptop (measured).
- [ ] Physics determinism tests pass.
- [ ] Movement feel reviewed by the owner (PRD §14: feel wins).

## Out of scope

Server prediction/reconciliation (stage 2), items/inventory (stage 4).
