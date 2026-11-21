# T0-09. Server regions prototype

- **Status:** done (2026-11-20)
- **PR:** [#18](https://github.com/Arburano/Minecraft-rust/pull/18)
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

- [x] Proptest: random bot movement, merging and splitting — no entity is lost or duplicated (`tests/conservation.rs`, 64 cases; resident + in-flight ids equal the initial set after every operation).
- [x] Repeatability: same input and partitioning → same state hash (`tests/determinism.rs`: 200 bots, 200 ticks with a merge and a split; identical hashes across pool sizes 1/2/4 and a repeat run).

## Results

- Implementation: `crates/sim` — region = `bevy_ecs::World`, stable `NetId`, PreTick/Simulate/Exchange/PostTick phases, sorted transfer messages (sent tick N → processed N+1), merge/split between ticks with the buffer invariant enforced at new borders.
- Benchmark: 100/500/2 000 bots × 1/8 regions, 3 runs × 1 000 ticks + Criterion; all configs far under 25 ms (worst p99 ≈ 0.27 ms); entity transfer ≈ 168.5 ns vs the ≤ 5 µs hypothesis. [Summary](../../docs/research/sim-regions-2026-11-20.md), raw outputs in `docs/research/sim/`.
- First data for open question No. 6 recorded; the capacity answer stays with the stage 2 benchmark (the load is light kinematic bots, not gameplay).
- Report: [task-reports/t0-09-sim-region-prototype.md](../../task-reports/t0-09-sim-region-prototype.md).

## Out of scope

No networking and no mods.
