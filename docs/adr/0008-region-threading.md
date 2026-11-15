# 0008. Regional multithreading

Status: Accepted (2026-11-15)

## Context

The main bottleneck of Minecraft servers is the single tick thread. Folia showed that regions with their own tick cycles scale, but it broke compatibility with all plugins. We have no legacy API.

## Decision

The server splits the world into dynamic regions (a separate `bevy_ecs::World` per region) with a buffer of ≥ 1 section. Regions tick in parallel during the `Simulate` phase; between regions there are only messages, delivered in `Exchange` with sorting. Merging/splitting happens in `PostTick`. Mods have access only to their own region.

## Consequences

- Scales across cores if players are spread out (≥ 8 regions for 500 players).
- A crowd in one place — one thread; a "region capacity" target and queues/instances are needed.
- There is no bit-level determinism between runs.

## Alternatives considered

- One tick thread + parallel systems — limited by a single world and synchronization.
- A static grid of threads — poor with uneven player distribution.

## References

docs/architecture/server-regions.md.
