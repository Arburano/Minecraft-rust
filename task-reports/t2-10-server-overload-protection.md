# T2-10: server overload protection — generation backpressure and entity limits

## What was done

**Generation backpressure** (`crates/server`, `crates/client`):

- **Holding players at the edge.** Unloaded cells were already solid for the physics on both sides (`MapBlocks`, T1-12). The server already applied a player's intents only once the 3×3 columns around it were live; until then the intents stay queued. That is the hold. Nothing new was needed to keep players out of unloaded terrain, and the test below proves it under load.
- **Hold metric.** A region counts player-ticks at the edge (`RegionOutput::held`): the 3×3 not live, or the 5×5 not all live while intents are arriving. The node sums them (`World::held_ticks`). `World::player_on_loaded` exposes the check for tests.
- **Terrain ahead first.** Column requests from regions now carry a score: the distance to the player's column, or to where it will be in 2 s (velocity × 40 ticks), whichever is nearer. The node sorts all regions' requests by score before the in-flight limit applies. Before, they were sorted by coordinates. Territory columns are filled nearest to a member first, computed per member area rather than scanning every member.
- **Client notice.** `Session::terrain_loading()` is true when a column around where the player will be in half a second is missing; the HUD then shows "Loading terrain…".

**Entity limits:**

- `mods/core/data/entities/limits.toml` sets the simulated entities per region by kind: `[default] max_per_region = 2000`, and `test:walker` = 1,000.
- `server::entities::EntityLimits` (load, defaults, `of(kind)`), passed through `WorldSetup`.
- Relocation and waking keep entities stored (dormant) when the target region is at its limit; they wake when there is room. `EntityStats::over_limit` counts them, and the `entity_load` example prints it.

**Tests and measurements** (cloud VM 4 vCPU Xeon 2.1 GHz, `crates/server/tests/backpressure.rs`, one run each):

- 50 players at the highest fly speed (≈ 176 blocks/s) fly straight out for 20 s over a never-generated world:
  - **0 ticks with a player inside unloaded terrain or below the world**, counted from when each first stood on loaded ground;
  - farthest 1,633 blocks (debug) / 1,784 (release);
  - 82% of player-ticks held at the edge;
  - tick p99 **10.7 ms** in debug and release (max 13.7 / 15.1 ms).
- 10,000 entities in one region: 1,000 simulated (the limit), 9,000 stored, none lost; tick p99 4.0 ms (release). `entity_load -- 10000 20 0`: p99 4.1 ms, 9,000 over the limit.
- Unit: limits parse, defaults, and the core datapack value.
- `cargo fmt --all`, `cargo clippy --workspace --all-targets` with `-D warnings`, `cargo test --workspace`.

## What went wrong

- The first flyer run reported 100 violations: 50 players × 2 ticks before the spawn area existed (they are placed at y = 120 before generation). The check now starts from each player's first tick on loaded ground.
- `over_limit` was part of the drained statistics, so a second read in the same tick showed 0. It is now state of the node.

## Issues encountered

- **Falling blocks and items do not exist yet** (stage 4 gameplay), so the "10,000 falling blocks" criterion is measured with 10,000 test entities. The limit mechanism is general (by kind name, in the datapack); falling blocks settling as blocks and items merging are behaviors that come with those entities.
- **82% held** is expected: 50 players at 176 blocks/s need far more generation than 4 cores give (the ring of a 21-column view per column crossed). The point of the test is that they wait instead of falling through, and that the tick stays bounded. Generation speed for such flyers is a separate optimization (or a lower fly speed limit on servers: a gameplay decision).
- **The client is held as well** (its own map lacks the columns), so a player at the edge stops on both sides without corrections. If the server lags behind a client that already has the columns (e.g. after a server restart), the server's hold corrects the client back — rare, smoothed.
- **The laptop number** (8 cores) for the flyer test is for the owner, together with T2-09: `cargo test -p server --release --test backpressure -- --nocapture`.
