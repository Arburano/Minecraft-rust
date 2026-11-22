# T1-12. Player physics, digging and placing blocks

- **Status:** in progress (cloud part done; owner: laptop latency run, movement feel after T1-13)
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

- [ ] Edit-to-visible latency ≤ 1 frame at 60 FPS on the owner's laptop (measured). VM: the block itself is ready for upload in 0.5–0.9 ms p50 (≤ 1.7 ms p95). Light: underground edits relight in ≈ 1 ms; edits that change direct sky take 9–17 ms p50 on the VM (T1-09 world path) — one frame late at best and a frame spike if processed in one go; see the report.
- [x] Physics determinism tests pass (`sim::physics` tests, 600 scripted ticks bit-identical).
- [ ] Movement feel reviewed by the owner (PRD §14: feel wins) — needs the playable client (T1-13).

## Laptop run (owner)

```bash
cargo run --release -p client --example edit_latency -- 100   # 3 runs; paste the output into the report
```

## Out of scope

Server prediction/reconciliation (stage 2), items/inventory (stage 4).
