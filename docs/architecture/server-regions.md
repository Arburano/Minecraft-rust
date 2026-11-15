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

## Memory

500 scattered players × 15,000 sections — ≈ 8.9 GB of voxels alone (estimate). Therefore: simulation distance 6–8 chunks, the rest of the view radius is an LRU cache of compressed columns for sending; light only where gameplay needs it.

## Tests

- 500 bots on 8 regions and on 1 region, 16 cores: tick p99 ≤ 25 ms on 8 regions; find the maximum for 1 region.
- Merge/split with 1,000 entities: none is lost or duplicated (property test).
- Reproducibility: the same input and the same partitioning → the same state hash.
