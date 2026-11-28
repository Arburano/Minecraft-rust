# T2-06: regional multithreading on the real server

## What was done

The single-threaded world of T2-02…T2-04 is now a node of parallel regions (ADR-0008, `server-regions.md`).

**`crates/server`:**

- `partition.rs` (pure functions, unit tests): territories are squares of the simulation distance plus a 1-column buffer around each player, and radius 2 around the world-spawn anchor.
  - Union–find joins members whose areas touch, so regions never border each other directly.
  - Hysteresis: members already together split only when their areas are 2 columns apart.
  - Region IDs are kept by majority vote, with deterministic tie-breaks.
  - The pair scan is sorted by x and stops early.
- `region.rs`: a region is a `bevy_ecs::World` with the players as entities (`Player` component), plus a `WorldMap` of its live columns. Its tick (spawn, movement and reconciliation, edits, streaming, replication) is the T2-02…T2-04 logic, now per region. It reads only immutable shared state:
  - the registry and physics;
  - the encoded-column cache;
  - precomputed view offsets;
  - last tick's player snapshot.
- `game.rs` is the node. Each tick:
  - **PreTick** routes connection messages to the owning region and makes arriving columns live in the covering region.
  - **Restructure** runs between ticks. If nobody crossed a column border, nothing happens. If only positions changed, the area edges are updated through coverage counts. Otherwise it repartitions fully: player entities move between ECS worlds and live columns between maps, and empty regions are retired.
  - **Simulate** runs the regions in parallel on a rayon pool (`ServerConfig::region_threads`, default 80% of CPUs).
  - **Exchange** sends block changes to players of other regions that hold the column, re-encodes edited columns into the cache, requests missing columns, takes the player snapshot and trims the cache once a second (spatial grid).
- Columns between the simulation and view distances exist only as encoded bytes in the node cache. A column entering a territory is decoded on the job pool, outside the tick. Edited columns leaving every territory are saved.
- Storage and generation now return the encoded bytes with the column, so encoding also happens off the tick.
- `columns.rs`: record, encode, decode.
- New `ServerConfig` fields `sim_distance` (default 8) and `region_threads`. Metrics: `World::times()` (total, pre, restructure, simulate, slowest region, exchange), `regions()`, `state_hash()`.
- `replication.rs`: fitting a datagram now uses a binary search on the number of updates instead of re-encoding after each removal. This was the main cost of crowded regions.
- `examples/region_load.rs`: N players in K clusters walking circles on the real world thread, without the network, with tick percentiles.

**Tests:**

- `tests/regions.rs`:
  - **1,000 players** in 25 clusters, 60 steps of random walks with jumps and pulls to the centre; regions go from 1 to 36 and 48 region IDs are used. Every step checks that no player is lost or duplicated, that territories never overlap or border, and the player count. The run is repeated: same seed → same per-step state hashes, and **1 region thread = 4 region threads**.
  - A player flying 240 blocks away splits the region, and flying back merges it.
- All T2-02…T2-04 server and client tests pass on the regional node unchanged.
- `cargo fmt --all`, `cargo clippy --workspace --all-targets` with `-D warnings`, `cargo test --workspace`.

**Measured** (cloud VM, 4 vCPU Xeon 2.1 GHz, release, view 10, simulation 6, 3 region threads, 30 s after warm-up):

| Load | Tick p99 |
| --- | --- |
| 200 players in 8 regions (3 runs) | **5.8 / 6.8 / 6.8 ms** (p50 2.8–3.1 ms) |
| 100 players in 1 region | 5.4 ms |
| 200 players in 1 region | **25.7 ms** |
| 400 players in 1 region | 58 ms |

One region holds ≈ 200 players within 25 ms p99, with everybody in view of everybody.

## What went wrong

- First 8-region run: p99 13.5 ms, mostly sequential node work. Three changes fixed it:
  - the full territory rebuild on every column crossing (nearly every tick with 200 walkers) became an incremental coverage update;
  - decoding cached columns in the tick became jobs on the pool;
  - the per-second cache trim (every cached column against every player) now uses a spatial grid.

  Sequential work went from p95 ≈ 4.3 ms to ≈ 1.0 ms. Streaming also stopped sorting the 441 view columns per player per tick (precomputed offsets, plus a skip when everything is sent).
- In one region, 100 → 200 players jumped from 6 to 43 ms because fitting a full datagram re-encoded it once per removed update. A binary search brought 200 players down to 25.7 ms.
- An unfinished script edit left `ServerConfig` half-changed for one build; the compiler caught it.
- The first version of the fly test waited for spawns with a wrong loop condition and read the player before the first tick; fixed.

## Issues encountered

- **Players of other regions are replicated one tick late** (from the previous tick's snapshot), which is how other regions are read without locks. Same-region players are current.
- **Inter-region messages:** player transfers happen only in the restructure, so `Simulate` has no message traffic yet. Distant actions (teleports, projectiles beyond the buffer) will reuse `sim::message` from the T0-09 prototype when gameplay needs them. The prototype stays as the reference and keeps its measurements.
- **A crowded region is O(n²)** in replication (each player gets deltas of all others in view). About 200 players in one place is the limit on this VM. The plan in `server-regions.md` (a queue or an instance beyond capacity) is not implemented. The rest of the replication cost is cloning the baseline map per player per tick; that can be optimized later.
- **Simulation distance 8 / view 14 by default.** Territories cover only the simulation distance; the rest of the view is streamed from encoded bytes, so the server does not hold live columns for the whole view.
- **Laptop run pending:** the acceptance number (200 bots in 8 regions, p99 ≤ 25 ms on 8 cores) and the single-region maximum on the laptop are for the owner. The VM results suggest a large margin. The bots here are in-process and skip the network; network bots are T2-05.
- Pinning heavy regions to fast cores is stage 6.
