# T2-04: block edits over the network

## What was done

**Server (`crates/server`):**

- `conn.rs` reads the client's events stream (the first bidirectional stream the client opens) in its own task. It forwards `Dig` and `Place` to the world thread with a rate flag: ≤ 20 edits/s, burst 40; edits over the limit are refused. A malformed event closes the connection. Chat is ignored until there is UI (stage 4).
- `game.rs`, `apply_edits` (each tick, after movement), validates edits in arrival order:
  - the player is spawned and the cell is loaded and inside the height;
  - the eye is within reach + 1.5 blocks of the cell center;
  - digging needs a non-air block;
  - placing needs a known non-air state, an air or fluid cell, and no other player inside the block.

  Accepted edits change the map, which marks the column for saving through the storage thread, and drop its encoded-column cache so later joiners get the new data. Each player that has the column gets one `BlockChanges` per tick, then the editor gets its `BlockAck`s, on the same ordered events stream. `ToConn::Forget` became `ToConn::Event(ServerEvent)`.

**Client (`crates/client`):**

- `remote.rs`:
  - sends `ClientEvent`s on a client-opened events stream;
  - maps block states both ways (`server_state` for placing; server → client for `BlockChanges`);
  - reports `RemoteEvent::Blocks` and `EditAck`.
- `session.rs`:
  - `dig` and `place` now work on servers: the local edit (T1-12 editor: remesh, deferred relight) happens at once and is sent with a sequence number;
  - per cell with pending edits, a shadow keeps the server's state; when the last pending edit there is answered, the cell takes the server's state (an accepted edit stays, a refused one rolls back, a concurrent foreign edit wins in server order);
  - changes for columns not yet arrived are buffered (≤ 4,096 per column) and applied on arrival; changes for columns still in the network queue are applied to them directly;
  - new: `NetStats::rejected_edits` / `pending_edits`, `pending_light()`, and test hooks `edit_for_test` and `light_for_test`.

**Tests** (`crates/client/tests/edits.rs`, cloud VM 4 vCPU Xeon 2.1 GHz, debug, one run each):

- Two clients: Alice places a torch and stone, and Bob sees them after **36 ms** (loopback). Bob digs the stone and Alice sees it. After everything is acknowledged and relit, the light around both edits **equals a fresh `light_column`** on both clients (the T1-12 equality check). No refusals.
- A placement 12 blocks away is predicted, refused by the server and **rolled back**.
- An edit survives a **server restart** and is seen by a new player.
- Both clients behind relays with RTT 100 ms: the worst of 5 edits reached the other client in **152 ms**, which is one tick plus RTT.
- `cargo fmt --all`, `cargo clippy --workspace --all-targets` with `-D warnings`, `cargo test --workspace`.

## What went wrong

- A missing import (`BlockPos` in `session.rs`) and a non-exhaustive match after adding the new events; both were found by the compiler and fixed. Otherwise none.

## Issues encountered

- **Edits use the events stream, not input datagrams**, as decided in T2-01: they must arrive exactly once and in order.
- **The editor is not checked for overlapping its own block on the server.** The client checks itself, and the server's copy of the player lags by a round trip, so a server check would refuse valid placements while walking. Other players are checked. Anti-cheat is stage 7.
- **Reach tolerance +1.5 blocks** on the server for the same lag reason.
- **No block-update rules yet** (sand falling, fluids, supports): the server only applies the edit. Those belong to gameplay (stage 4) and to region simulation (T2-06).
- **Inventory and item costs** are out of scope (stage 4); any registered block can be placed.
- A column the server re-sends after a failed stream while the client already has it could, in a rare case, bring back older data from the network queue. Not seen in tests; noted for T2-06, when the column ownership moves to regions.
