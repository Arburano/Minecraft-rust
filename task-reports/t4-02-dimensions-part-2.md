# T4-02 (part 2): several dimensions, travel, protocol

## What was done

- **Server (`crates/server`):**
  - One `game::World` per dimension. Each has its own regions, column store, generator, bounds, cache and player physics.
    - The generator is the file of the dimension's `worldgen:<id>` module.
    - The seed is derived from the world seed and the id.
    - The gravity is the movement profile × `player_gravity_scale`.
    - Region threads are shared out between the worlds.
  - `server::node::Node` (new): owns the worlds on the world thread, routes every connection message to the player's world and steps all worlds at 20 TPS. A build with one dimension still runs its world directly.
  - A build with more than one dimension must require the `dimensions` capability (`Capability::Dimensions` is now in the built-in subsystem registry as a marker). Without it the start fails with a message naming the build and its dimensions.
  - Storage: the overworld stays at the world root; other dimensions use `<world>/dimensions/<id>/`.
  - Travel: `World::take_player` and `World::insert_traveler`, between two ticks.
    - The player arrives at the given position, or flying 32 blocks under the top at the same x and z.
    - It gets a new network id and a new `Spawn`.
    - Entry points: `Server::travel(player, dimension, pos)`, and on offline servers the chat command `/dimension <id>` (no id: list the dimensions).
  - Player files record `dimension` (left out for the overworld, so older files load); a returning player joins the dimension it left in.
  - An edit tagged with another dimension than the player's is refused.
- **Protocol (`crates/protocol`), version 6:**
  - The dimension index (position among the build's dimensions sorted by id) is on `Spawn`, `ColumnPayload`, `EntityDatagram`, `ServerEvent::BlockChanges` / `ForgetColumn` and `ClientEvent::Dig` / `Place`.
  - Golden files refreshed.
- **Client (`crates/client`):**
  - `RemoteEvent` carries the dimension, and a new `Respawn(Spawn)` event comes from the control stream.
  - Columns are decoded with their own bounds, because dimensions differ in height.
  - On a travel the session drops the old world (`Loader::clear`: columns, light, meshes, jobs in flight; plus pending columns, edits, other entities and prediction history). It then starts a map with the new bounds and the new gravity scale.
  - Messages tagged with another dimension, and columns whose bounds differ from the map's, are dropped.
  - New `Session::dimension()`, `travels()` and `send_chat()`.
- **Docs:** `dimensions.md` ("Implementation (T4-02 part 2)", status), `networking-protocol.md` (version 6), the task file, `tasks/README.md`.

**Tests:**

- `crates/client/tests/dimensions.rs` (new, 2 tests): a test module adds `moon` with a height of -64..256 and half the gravity.
  - A player starts in the overworld and travels with `/dimension moon`. On the moon the map has moon bounds, every column has them, and the gravity is half.
  - The player leaves; its file says `moon`; it rejoins on the moon.
  - `Server::travel` brings it back to the overworld with full gravity. Its file then names no dimension, and the moon's store is in `dimensions/moon`.
  - A build with two dimensions and no `dimensions` capability is refused.
  - Both pass in 8.2 s (cloud VM, 1 run).
- `server/tests/zones.rs`: an edit tagged with another dimension is refused where the player may build.
- `server::players`: the dimension round-trips, and the overworld writes no `dimension` key.
- Existing server and protocol tests and examples updated for the new fields.
- `cargo fmt --all`; `cargo clippy --workspace --all-targets` and `-p client --no-default-features` with `-D warnings`: clean. `cargo test --workspace`: see "Issues encountered".

## What went wrong

- Rust 2024 reserves `gen`, so a local named `gen` did not compile (renamed `dim_gen`).
- `World::saved(&self, &p)` borrowed `self` while a player was borrowed mutably out of a region; it became a free function `saved_in`.
- The client's reset on `Respawn` first called a `Link` method, which conflicted with the borrow of `link.remote` held by `poll()`. The fields are now cleared one by one at the call site.
- Several server tests and examples built `WorldSetup` and `Storage::start` by hand and broke on the new fields; they were updated.

## Issues encountered

- **Not measured:** "tick and memory unchanged for a build with one dimension". Such a build runs its world without the node, as before. The only change on its path is one byte on a few messages. No before/after benchmark was run.
- **Deviation from `dimensions.md`:**
  - Only players travel. Moving other entities and grids needs the mod API (stage 3) and `moving_grids` (T4-03).
  - The client keeps one map, not one per recently seen dimension, so going back streams the columns again. Keeping the last map is a possible later optimisation, if portal trips show a wait.
- Travel happens at the tick boundary between the node's world steps. That is what the doc's "Exchange phase" stands for, since no region of either world runs then. It is not a separate phase inside the region scheduler.
- `/dimension` works only on offline servers (development and tests); gameplay travel (portals) will come from mods.
- Dimension indices depend on the sorted ids of the build. Client and server must run the same build, which the mod manifest check (stage 3) will enforce.
- The arrival height (flying, 32 blocks under the top) is a placeholder until portals choose a position.
