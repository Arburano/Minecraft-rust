# T1-09. Lighting integrated with the world

- **Status:** todo
- **Depends on:** T1-01, T1-03, T0-08
- **Owner:** agent (cloud)

## Goal

Connect the T0-08 `light` crate to the world: initial lighting of generated columns in 2×2 column groups, importing stored light into snapshots (instead of a full relight per edit), applying results on the next tick, and a per-tick limit with deferral.

## Specs

- [lighting.md](../../docs/architecture/lighting.md)
- [T0-08 report](../../task-reports/t0-08-light-incremental-bfs.md) — open items

## Deliverables

- `light::LightNeighborhood::from_world` / import of stored `SectionLight`; export back into the world map.
- Initial lighting job for 2×2 column groups on the scheduler; borders computed once.
- Per-tick light update limit with a deferred queue; the client shows old light until then.
- Tests: world-level edits equal a full relight of the affected area (proptest); seam test across column groups.

## Acceptance criteria

- [ ] Proptests pass; no visible seams at group borders (test).
- [ ] Initial lighting time per column measured; per-edit budgets still met through the world path (≤ 100/200 µs p95).

## Out of scope

Server-side lazy gameplay light (stage 2), smooth lighting/AO in the shader (T1-11 may use it).
