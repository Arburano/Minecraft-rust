# T0-09. Server regions prototype

- **Status:** in-progress (2026-11-20)
- **Depends on:** T0-02
- **Owner:** agent

## Goal

In the `sim` crate: a region = `bevy_ecs::World`, `NetId` for entities, merge and split with a buffer of ≥ 1 section, tick phases, inter-region messages with sorting. Benchmark: N simple bots (movement + collisions with a flat world) on 1 and on 8 regions.

## Specs

- [docs/architecture/server-regions.md](../../docs/architecture/server-regions.md)
- [docs/architecture/engine-core.md](../../docs/architecture/engine-core.md)
- [docs/adr/0008-region-threading.md](../../docs/adr/0008-region-threading.md)

## Deliverables

- `sim::region` with tests for merge/split and message delivery (N → N+1).
- Benchmark of a tick for 100/500/2 000 bots, 1 and 8 regions; results in docs/research/.
- First data for open question No. 6 (region capacity).

## Acceptance criteria

- [ ] Proptest: random bot movement, merging and splitting — no entity is lost or duplicated.
- [ ] Repeatability: same input and partitioning → same state hash.

## Out of scope

No networking and no mods.
