# Server: regions and threads

Crates: `sim` (regions), `server` (binary). ADR: 0008 (regional multithreading).

## Region

- A region = a separate `bevy_ecs::World` with a set of sections and entities, ticked by one thread at a time.
- Regions form around players and active entities, merge when they get close, and split when they move apart.
- Buffer invariant: around occupied sections there is a buffer of ≥ 1 section belonging to the same region. Two regions never border each other directly, so local interactions (a neighboring block, light ≤ 15 blocks, a nearby mob) do not leave the region.
- A merge with a region that is currently ticking is deferred until the end of its tick; splitting is checked in `PostTick`.
- Transferring entities between `World`s is copying components by stable `NetId` (target ≤ 5 µs per entity).

## Access

- Region code (engine and mods) reads and writes only its own region.
- Distant actions (teleport, a projectile beyond the buffer, a command to another player) are messages via `send_to`, delivered in `Exchange`, processed in the next tick.
- The global region holds time, weather, tables; reads use a snapshot of the previous tick.

## Node threads

| Pool | Size (16 cores) | Work |
| --- | --- | --- |
| Network | 2–4 | quinn, delta encoding |
| Chunk IO | 2–3 | reading/writing region files |
| Generation | 2+ (0 if the world is pre-generated) | worldgen |
| Region tick | ≈ 80% of remaining cores | region `Simulate` |
| Tasks | the rest | lighting, pathfinding, LOD |

Sizes are tuned automatically from metrics; starting values come from Folia experience (≈ 330 players on 16 cores in their test).

## What parallelizes poorly and how to live with it (PRD §14, No. 6)

- The tick of a single dense region (a crowd in one place, large contraptions, block update chains) is sequential by nature. CPU choice depends on this: high core clock matters more than core count beyond 16.
- The heaviest regions go to the fastest cores (pinning, P-cores on hybrid CPUs); enabled in stage 6.
- There is no dedicated core "for mods": mods run in the tick of their region with a budget, otherwise every hook would become cross-thread synchronization.
- The capacity of a single region is a separate target: the maximum number of players in one region at tick ≤ 25 ms. Beyond that — a queue or an instance. The number is determined by the stage 2 benchmark.
- **Two levels of parallelism.** Regions are the macro level (ADR-0008). Inside a crowded region the systems still run in parallel by declared access (engine-core.md), and heavy systems (physics, AI decisions, fluids) iterate entities in parallel batches (`par_iter`) when the entity count makes it worth it. Entities that are not ticked (simulation LOD, T2-07) cost no CPU: they are filtered out by component, not checked one by one. This is what keeps a single dense region from being one thread ([review](../research/external-projects-review-2026-11-26.md), section 1).

## Overload protection (T2-10)

- **Generation backpressure:** the authoritative player position never enters a column that is not loaded on the server. A player faster than generation is slowed or held at the edge (with a notice), and generation for that player gets the highest priority. Dropping generation work silently and letting players fly into empty space is never acceptable, even if it keeps the tick rate.
- **Cheap-entity limits:** falling blocks and item entities have per-region limits and merge rules (numbers in the `core` datapack), so lag machines cannot stall a region.
- **Activation range:** the simulation LOD (simulation-and-ai.md) is our equivalent of an entity activation range; block mechanics inside the simulation distance are never slowed.

## Memory

500 scattered players × 15,000 sections — ≈ 8.9 GB of voxels alone (estimate). Therefore: simulation distance 6–8 chunks, the rest of the view radius is an LRU cache of compressed columns for sending; light only where gameplay needs it.

## Prototype (T0-09)

`crates/sim` implements the core of this design: region = `bevy_ecs::World`, stable `NetId`, the four tick phases, sorted inter-region transfer messages, merge/split between ticks with the buffer invariant enforced at new borders (split deferred while entities occupy the border sections). Prototype relaxations, to be lifted in later stages: a flat single-layer world with rectangular territories on the x/z plane; entity ownership by territory containment, so a walking entity transiently occupies the buffer ring before its transfer is delivered; intents for in-flight entities are dropped deterministically; the load is scripted kinematic bots, not gameplay. Measurements: [T0-09 summary](../research/sim-regions-2026-11-20.md).

## Implementation on the real server (T2-06, `crates/server`)

- **Node and regions:** the world thread is the node (`game.rs`). A region (`region.rs`) is a separate `bevy_ecs::World` whose entities are the connected players, plus a `WorldMap` with the live columns of its territory. Regions tick in parallel on a rayon pool (`ServerConfig::region_threads`, default 80% of the logical CPUs) and touch only their own players and columns; they read the node's shared state (registry, physics, the encoded-column cache, last tick's player snapshot) immutably. No locks around the world.
- **Phases per tick:** `PreTick` routes connection messages to the owning region and makes arriving columns live; the **restructure** (between ticks) recomputes the partition; `Simulate` runs the regions in parallel; `Exchange` sends block changes to players of other regions that hold the column, re-encodes edited columns into the cache, requests missing columns, and takes the player snapshot.
- **Territories and partition** (`partition.rs`, pure): each player owns the square of columns within the **simulation distance** (default 8, `ServerConfig::sim_distance`) plus a 1-column buffer; the world spawn is an anchor with radius 2. Members whose areas touch share a region (union–find), so two regions never border directly. Members already together split only when their areas are 2 columns apart (hysteresis against flicker at a border). Groups keep the region ID most of their members had (ties: the lower ID); others get new IDs. When nobody crossed a column border nothing is recomputed; when only positions changed and every member keeps its region, the area edges are updated incrementally (coverage counts); full repartitioning (merge, split, join, leave) moves player entities between ECS worlds by stable ID and live columns between maps.
- **Columns:** live columns exist only inside territories. Columns between the simulation distance and the view distance live in the node's **cache of encoded columns** (the "LRU of compressed columns for sending"), which also feeds streaming. A column entering a territory is decoded from the cache on the job pool (off the tick) or loaded from storage/generation; a column leaving every territory is dropped from the live maps and saved if edited. The cache keeps every column some player may see (view + 2) and the spawn area; it is trimmed once a second.
- **Interactions across regions:** block changes reach players of other regions in `Exchange` (same tick); other regions' players are replicated from the previous tick's snapshot (one tick late), own region's from the current state. Entity transfers between regions happen only in the restructure, so `Simulate` needs no inter-region messages yet; distant actions (teleports, projectiles) will use the message machinery of the T0-09 prototype (`sim::message`).
- **Determinism:** the same inputs and partitioning give the same state hash, independent of the number of region threads (`crates/server/tests/regions.rs`).

Measured (cloud VM 4 vCPU Xeon 2.1 GHz, release, `cargo run -p server --release --example region_load -- <players> <clusters> 30`, view 10, simulation 6, players walking circles, no network, 3 region threads):

| Load | Tick p50 / p95 / p99 | Slowest region p99 |
| --- | --- | --- |
| 200 players in 8 regions (3 runs) | 2.8–3.1 / 4.8–4.9 / **5.8–6.8 ms** | 3.2–4.3 ms |
| 100 players in 1 region | 3.0 / 4.4 / 5.4 ms | 4.5 ms |
| 200 players in 1 region | 13.2 / 24.7 / **25.7 ms** | 25.1 ms |
| 400 players in 1 region | 31.8 / 49.5 / 58.0 ms | 57.0 ms |

So on this VM one region holds ≈ 200 players within 25 ms p99 (all of them in view of each other — the worst case for replication, which is O(n²) per region). The laptop numbers (8 cores, the stage 2 exit check) are for the owner.

## Tests

- 500 bots on 8 regions and on 1 region, 16 cores: tick p99 ≤ 25 ms on 8 regions; find the maximum for 1 region.
- Merge/split with 1,000 entities: none is lost or duplicated (property test).
- Reproducibility: the same input and the same partitioning → the same state hash.
