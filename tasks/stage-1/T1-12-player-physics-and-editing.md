# T1-12. Player physics, digging and placing blocks

- **Status:** done (owner reviewed movement and flight, 2026-11-23)
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

- [x] Edit-to-visible latency ≤ 1 frame at 60 FPS on the owner's laptop (measured: block geometry ready in 0.53–0.61 ms p50, underground total 1.24–1.50 ms; sky changes 11.8–13.3 ms deferred).
- [x] Physics determinism tests pass (`sim::physics` tests, 600 scripted ticks bit-identical).
- [x] Movement feel reviewed by the owner (PRD §14: feel wins): "movement is fine", 2026-11-23, including flight with mouse-wheel speed.

## Laptop run (owner)

```bash
cargo run --release -p client --example edit_latency -- 100   # 3 runs; paste the output into the report
```

## Out of scope

Server prediction/reconciliation (stage 2), items/inventory (stage 4).
