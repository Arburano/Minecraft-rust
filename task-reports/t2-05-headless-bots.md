# T2-05: headless bot clients and the load harness

## What was done

**Bots are real game clients without a window** (`crates/client`):

- `Session` headless mode (`SessionConfig::headless`):
  - no light, meshes, loader or job pool;
  - the block editor only changes blocks (`Editor::headless`);
  - columns within 2 of the player are decoded straight into the map; others arrive and are kept compressed (`RemoteEvent::Packed`), and are decoded only when the bot walks up to them (`Remote::unpack`);
  - "playing" means the 3×3 columns are present.

  Prediction, reconciliation (state hashes, corrections), entity replication and edits are the normal client code.
- `Remote::connect_with(ConnectOptions { runtime, focus })`: many sessions share one tokio runtime. A `Focus` (an atomic center plus radius) decides which columns the network task decodes.
- `client::bots` (library): behaviors
  - **walk**: random turns, jumps, steps around walls;
  - **fly**: circles at a varying height, sprinting;
  - **build**: walks slowly and places, then digs, a stone in front of its head every half second.

  With `groups`, bots first fly to homes on a square grid `group_spacing` apart (default 400 blocks, beyond the view and simulation radii). They climb over mountains, then land and start their behavior. Bots join at a set rate; one loop runs `update` every 5 ms and a tick every 50 ms. After the warm-up it measures received/sent bytes, corrections, edits, others in view, distance from home and columns received. `Report::summary` and `Report::csv` format the results.
- `bots` binary: `--connect ADDR` or `--embedded DIR` (a server in the same process), `--count`, `--mix walk,fly,build`, `--groups`, `--group-spacing`, `--join-rate`, `--warmup`, `--seconds`, `--roam`, `--out`. It writes `bots-summary-<time>.txt` and `bots-<time>.csv`, and the server tick percentiles in embedded mode.
- `Session::edit_cell` (public edit for bots); `NetStats::columns`.

**Server metrics** (`crates/server`):

- `metrics.rs`:
  - `TickLog`, a lock-free ring of 24,000 tick records (tick time, players; regions with players);
  - `Percentiles`;
  - `Process` (memory via `memory-stats`, already a workspace dependency; CPU time from `/proc/self/stat` on Linux).
- `World::set_log` records every tick; `Server::tick_log()`.
- `server --stats-out FILE [--stats-warmup 30]`: a line every 10 s (players, regions, tick percentiles, memory), and on stop a summary file (tick after the warm-up, players, regions, memory, CPU time).

**Tests and runs** (cloud VM, 4 vCPU Xeon 2.1 GHz; bots and server in one process, sharing the CPU):

- `crates/client/tests/bots.rs` (CI smoke, debug): **50 bots** (30 walk, 10 fly, 10 build) join, play for 15 s and leave. 0 join failures, 0 errors, every bot exchanged data, builders' edits were accepted; server tick p99 4.9 ms.
- `bots --embedded … --count 200 --groups 10 --mix 8,0,2 --warmup 200 --seconds 60` (release, one run):
  - all 200 joined and arrived home (max 71 blocks away); others in view p50 19, max 19;
  - **received per bot p50 19.4 kbit/s, p95 26.8, max 38.3** (QUIC UDP payload); sent p50 12.6;
  - server tick p50 4.1 / p95 6.1 / **p99 7.6 ms**, 8 regions;
  - 115 corrections over 200 bots × 60 s; 11,163 edits, 1,168 refused;
  - bot process 997 MB.

  This is not the stage 2 exit run: bots and server share one VM, and the exit run is the owner's 8-core machine (T2-09).

`cargo fmt --all`, `cargo clippy --workspace --all-targets` with `-D warnings`, `cargo test --workspace`.

## What went wrong

- **Headless bots froze after a few steps.** Columns outside the decode focus were dropped on arrival, and the server never resends a column, so a bot walking into a new column never got it and stopped "playing". Fixed by keeping them compressed and decoding on approach. Decoded columns now stay until the server forgets them.
- **Groups did not separate.** Walkers could not cross 2-block walls on the way home. Travel now flies (for every behavior), lands on arrival, and walkers step aside when blocked. The first 200-bot run still left 67 bots stuck at mountains (flight held only 12 blocks above the home's height). Climbing to +40 and climbing when blocked fixed it: in the second run all 200 arrived.
- **Fly bots receive 300–400 kbit/s.** Sprint-flying in circles crosses column borders about 1.4 times a second, and each crossing forgets one 29-column edge and streams the next. This is correct column streaming, not a bug, but it is not steady state: the exit run uses `--mix 8,0,2`.
- The "regions" line was read after the bots had left (0); now it is taken while measuring.

## Issues encountered

- **Bot memory ≈ 5 MB per bot** (997 MB for 200), mostly compressed and decoded columns kept until the server forgets them. That is fine on the bot machine; a smaller keep radius would trade CPU (re-decoding) for memory.
- **10% of edits refused** in the 200-bot run: builders place in front of their head while other bots of the group walk around, and placing into another player is refused. Expected.
- **The server's CPU time is Linux-only** (`/proc`); on Windows the stats file says "n/a". Memory works everywhere.
- **Join rate:** joins are sequential in the bot loop (one blocking login each). At 20 per second, 200 bots take 10 s, enough for the harness.
- The exit criterion (200 bots on the 8-core machine, p99 ≤ 25 ms, ≤ 50 kbit/s) is T2-09. The commands are in `testing-and-benchmarks.md` ("Load runs with bots").
