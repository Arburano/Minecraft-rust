# Core: tick, phases, scheduler, budgets

Crate: `engine-core`. ADR: 0002 (bevy_ecs), 0008 (regions).

## Tick

- Fixed step, 20 TPS by default (`DEFAULT_TPS`), configurable per world.
- The client renders with interpolation between the two latest states. Camera rotation is taken from the latest input at frame rate rather than interpolated (otherwise +50 ms of perceptible latency).
- If a tick overruns: the server does not "catch up" with a burst of ticks but skips and logs (the Minecraft "server is lagging" behavior), because catching up makes the situation worse like an avalanche.

## Phases

```text
PreTick ─► Simulate ─► Exchange ─► PostTick
```

| Phase | What happens | Parallelism |
| --- | --- | --- |
| `PreTick` | receiving intents from the network, commands, scheduled tasks | sequential |
| `Simulate` | tick of each region: engine and mod systems | regions in parallel; systems within — by declared access |
| `Exchange` | delivering messages between regions and to the global region | sequential; messages are sorted |
| `PostTick` | saving, deltas to clients, region merge/split, lighting and LOD queues | IO asynchronous |

Invariants:

1. A message sent in `Simulate` of tick N is processed in `Simulate` of tick N+1. The reply comes no earlier than N+2.
2. In `Exchange` messages are sorted by `(tick, sender_region_stable_id, sender_pos, counter)`, not in queue arrival order.
3. Within a region, systems iterate entities in stable ID order.
4. Reproducibility is guaranteed for the same input and the same region partitioning. There is no bit-exact determinism between server runs (see open-questions.md).

## Task scheduler

A pool separate from the ECS for heavy work: meshing, lighting, generation, LOD, pathfinding, decoding.

- Priorities (highest first): `Interactive` (re-meshing after a player edit, lighting near the player) → `Visible` (new chunks in frame) → `Prefetch` (along the movement direction) → `Background` (LOD, saving).
- Chunk queue priority is computed from distance, angle to the view direction, and player speed (PRD §14, No. 4).
- A task works on a data snapshot (an `Arc` snapshot of sections); the result returns to the main world through a channel and is applied in the next appropriate phase. Tasks never mutate the live world.
- Cancellation: a task has a token; if the chunk has been unloaded, the result is discarded without being applied.
- Implementation: `rayon::ThreadPool` + own priority queues (not `rayon::spawn` directly, as it has no priorities).

## Budgets

`Budget { soft, hard }`:

- `soft` — exceeding is recorded by the profiler (metric + a rate-limited warning in the log).
- `hard` — exceeding interrupts the work: for mods, an epoch interruption and discarding the batch of changes; for engine systems, only an alert (we do not interrupt our own code).

Budget values are in [`../process/performance-budgets.md`](../process/performance-budgets.md).

## ECS

- `bevy_ecs` without the rest of Bevy, version pinned in the workspace (ADR-0002).
- Each server region is a separate `World` (ADR-0008). An entity has a stable `NetId` (u64) that survives transfer between `World`s, region merges, and handoff between nodes.
- Components are data without logic; systems are functions. No trait hierarchies like `Mob: Entity`.
- Components replicated to clients are marked (`Replicate`) and have a versioned schema.

## Profiling

- `tracing` zones for each phase, each region, each system, and each mod.
- Tracy (`tracy-client`) behind the `profiling` feature flag.
- Implementation (T1-06): `engine_core::zone!("crate::what")` opens a Tracy zone until the end of the block. The macro is chosen when `engine-core` is compiled: without `profiling` it expands to nothing (measured: no change beyond VM noise on the `light` and `mesh` benches); with it, a zone costs one atomic check while no Tracy client runs. Binaries start the client with `engine_core::profile::start()` and forward the feature: `cargo run --release -p client --features profiling`, then connect the Tracy profiler GUI (version matching `tracy-client` 0.19). Zones so far: `jobs::run`, `sim::tick` with its phases, `mesh::build`, `mesh::remesh`, `light::full_relight`, `light::relight`, `modhost::call`. Zones are for coarse work units — never per block or per cell.
- Server commands `/profile` (tick, regions) and `/profile mods`.
