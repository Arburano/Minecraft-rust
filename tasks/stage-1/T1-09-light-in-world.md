# T1-09. Lighting integrated with the world

- **Status:** done (2026-11-22). Owner decision: option 1 for stage 1 — the world-path cost is accepted (updates are deferred and capped per tick; the ≤ 100 / 200 µs budget applies to the BFS); a faster algorithm that keeps the same look follows in [T1-16](T1-16-light-fast-world-path.md).
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

- [x] Proptests pass; no visible seams at group borders (test).
- [x] Initial lighting time per column measured (done: 5.4–5.6 ms p50 on the VM); per-edit budgets still met through the world path (≤ 100/200 µs p95) — not met; accepted by the owner for stage 1, follow-up T1-16 (torch ≈ 0.7–1.2 ms p95, sky-changing ≈ 13–15 ms p95).

## Out of scope

Server-side lazy gameplay light (stage 2), smooth lighting/AO in the shader (T1-11 may use it).
