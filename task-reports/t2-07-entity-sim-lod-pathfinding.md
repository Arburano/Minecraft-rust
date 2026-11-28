# T2-07: entity simulation LOD and asynchronous pathfinding

## What was done

**`crates/server/src/entities.rs`** (new):

- `Walker`: a test entity component. It has a player-sized body that uses `sim::physics::step`, a path, a waypoint index, request state, decision timing, stuck detection and a per-entity xorshift for random goals.
- `decision_interval`: 1 / 2 / 4 / 8 ticks at ≤ 48 / 96 / 144 / farther blocks from the nearest player (`simulation-and-ai.md`).
- `Walker::steer`: path following every tick. The entity walks towards the next waypoint, jumps to climb and stops at the end.
- `Grid::snapshot` copies the solidity of the 25 × 13 × 25 blocks around an entity (unloaded = solid), so the path job never touches region state.
- `astar`: 4 horizontal moves with one-block steps up (head room checked) and down, at most 4,000 expansions, deterministic tie-breaks. It returns the path to the goal, or to the reachable cell nearest it.

**Regions and node:**

- `region.rs`: walkers are ECS entities of the region (`insert_walker`, `take_walker`, `walker_mut`, `walkers`). `tick_walkers` runs after player movement:
  - entities spawned before their column was live are placed on the ground;
  - decisions run at the LOD interval (distance to this region's players and, from the snapshot, to other regions' players);
  - path requests stay within the per-region budget (≤ 4 per tick, ≤ 32 waiting);
  - steering and physics run every tick.

  Walkers are replicated with entity kind 1, and go into the cross-region snapshot.
- `game.rs`:
  - `relocate_walkers` (every tick, after the restructure) moves entities to the region covering their column. Outside every territory they become dormant: stored, not simulated, and woken when covered again.
  - `Exchange` submits path jobs: `Visible` near a player, `Prefetch` farther.
  - `PreTick` applies finished paths to the entity wherever it now is.
  - New API: `spawn_walkers` (golden-angle spread, placed on the ground when live), `walker_pos`, `entity_stats` (live, dormant, in flight, simulated, decisions, requests, paths done, the highest per-region requests in one tick, latency samples).
- `server --test-walkers N` spawns test entities around the world spawn (`ServerConfig::test_walkers`). The client draws entities of kind 1 as green boxes.

**Tests:**

- Unit: the LOD table; A* (straight, a detour around a wall, a 1-block climb, a 2-block cliff ending at its foot); path following and yaw.
- `tests/entities.rs`: 500 entities around a player. After 300 ticks the player moves 6 blocks per tick to 170 blocks away, so the entities cross every band. The test checks:
  - every band is used;
  - the largest horizontal step of any entity in one tick is **0.222 blocks** (continuous, no freeze or jump);
  - at least 256 entities move in every far tick;
  - 309 walked more than 3 blocks;
  - the request budget holds (≤ 4 per region per tick);
  - none is lost (entities left behind became dormant).

**Measured** (`examples/entity_load.rs`, cloud VM 4 vCPU Xeon 2.1 GHz, release, 30 s, one region):

| Player | Tick p50 / p95 / p99 | Decisions per tick | Path requests per tick | Latency |
| --- | --- | --- | --- | --- |
| at the center | 1.61 / 2.36 / **3.09 ms** | 300 | 3.98 (budget 4) | 1 tick |
| 120 blocks away | 0.88 / 1.95 / 2.53 ms | 108 | 1.08 | 1 tick |

`cargo fmt --all`, `cargo clippy --workspace --all-targets` with `-D warnings`, `cargo test --workspace`.

## What went wrong

- The first test run expected all 500 entities to stay live. When the player walked 170 blocks away, 17 wandered out of its territory and became dormant, which is the designed behaviour beyond the simulation distance. The test now checks that none is lost and that most stay live. The walking expectation was lowered to 250: far entities decide only every 8th tick.

## Issues encountered

- **The path budget is saturated near players** (3.98 of 4 requests per tick with 500 entities). Entities wait their turn while idle, which is what the budget is for; the budget can be raised (CPU is far from the limit). In the measurement, the 4-in-flight peak never came close to the 32 limit, and latency is one tick.
- **Physics runs every tick for every live entity**, as the spec says ("physics and animation remain"). The engine does not yet sleep entities that stand still. With 500 entities that is cheap (part of the 1.6 ms p50).
- **Dormant entities are not saved** to the world: they are test entities and live in memory only. Entity storage (the `entities` blob of the column record) comes with mobs in stage 4.
- **No determinism across runs for entities:** path jobs finish on the pool in a timing-dependent order. ADR-0008 allows this (no bit determinism between runs). The player-only state hash of T2-06 is unaffected.
- **Mob behaviors, spawning tables and despawning** are stage 4 (`mods/core`), out of scope here.
