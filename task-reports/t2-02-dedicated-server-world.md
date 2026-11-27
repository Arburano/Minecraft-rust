# T2-02: dedicated server — world, join, column streaming

## What was done

`server --offline` now runs a world that a client can join and play in. Columns stream in over the network; the client lights and meshes them as before (ADR-0011).

- **`server` crate** (now a library and a binary):
  - `server::start(ServerOptions) -> ServerHandle` starts everything. The binary adds `--world`, `--mods-dir`, `--seed`, `--view-distance`, `--max-players` and `--name` to the T1-08 flags.
  - **World thread** (`world_thread`): one region for now (T2-06 adds regional threading). It owns the block map, the players and what each client was sent. It ticks at 20 TPS and also wakes on every message.
  - **Generation:** a `JobPool` job generates a column and packs its `ColumnData` frame.
  - **Storage thread** (`storage`): owns `world::WorldStore` and the player files (`players/<id>.toml`). Storage answers and job results go to the world thread's own channel.
  - **Connections** (`conn`, tokio): handshake, admission, rate-limited input datagrams, and column streams.
  - **`interest`:** the pure rules of what each client gets and in which order (`ClientView`, `SendQueue`), with unit tests.
- **Join:**
  1. `Hello`, then admission: offline policy, name rules, no session token (`// OPEN-QUESTION: 4`).
  2. The world admits the player at once and rejects duplicates or a full server. It starts loading the spawn area right away.
  3. `Welcome` + `Registry`, then the client's `Ready`.
  4. The world places the player: a new player on the ground of column (0, 0), a returning one where it left.
  5. `Spawn`, then columns.

  Generating the spawn column overlaps the handshake round trips.
- **Streaming:**
  - The server sends every column within `view_distance` (Chebyshev) of the player's column, nearest first.
  - Each column has its own unidirectional stream; stream priority is the squared distance.
  - At most 16 streams are in flight per client, and each counts until the client acknowledged it.
  - When the player moves, columns farther than `view_distance + 1` are forgotten and pending ones are dropped. They are resent if the player comes back.
  - The client keeps columns up to `view_distance + 2`, so a one-column difference between the client's and the server's position leaves no holes.
- **Input:**
  - The client sends an `InputDatagram` with its last 3 frames every tick. It predicts with the dequantized frame, which is exactly what the server simulates (`sim::physics::step` on the server's map).
  - The server applies new ticks only, at most 4 per tick, and waits while the area around the player is still loading.
  - The server's position drives streaming. Reconciliation is T2-03.
- **Limits:**
  - `Hello` ≤ 16 KiB and control frames ≤ 1 KiB; every length is checked before allocating.
  - The client may open no unidirectional streams and at most 2 bidirectional ones.
  - Input is limited to 60 datagrams/s with a burst of 30; a client that keeps flooding is closed.
  - Timeouts on every handshake step; idle timeout 30 s with keep-alive 5 s; close codes in `server::conn::close`.
- **`net`:**
  - `net::client`: `PendingClient::connect` (blocking handshake with a timeout) and `start(unpack)`, which returns a `NetClient<T>`. Columns are decoded and unpacked (remapped to the client's registry) on the network threads; the game loop only polls `try_event`.
  - `net::frame`: length-prefixed frames on QUIC streams.
  - `game_server_transport()` and `client_endpoint_offline()`.
- **Certificates:** an offline server makes a self-signed certificate at start (`rcgen` becomes a normal dependency of `server`). An offline client accepts it without checking the issuer, but still verifies the TLS handshake signatures. New open question 16 covers real certificates; the code is marked `// OPEN-QUESTION: 16`.
- **Client:**
  - `Session::connect(&ConnectParams, SessionConfig)` gives the same `Session` the app already drives. It has a remote backend: no saving, no edits until T2-04, and `time_to_control()` / `connect_time()` / `columns_received()` for the login metric.
  - `Loader::set_remote(true)`: columns come only from the network and are never generated.
  - `client --offline-name NAME --connect ADDR` joins a server and prints the login time. The multiplayer menu is T2-08.
  - The headless example `net_join` measures login on any machine.
- **`net::lossy::LossyRelay`** now keeps packet order (see "What went wrong").
- **Docs:**
  - `networking-protocol.md`: a section on the server, join and streaming, plus the tests;
  - `performance-budgets.md`: "Joining a server until control" and "Nearest 3×3 at 2% loss" rows;
  - `open-questions.md`: question 16.

### Tests

- `server` unit tests:
  - interest rules (send once, forget and resend, queue order, the queue and the view forget alike);
  - the input rate limit;
  - admission;
  - player save round trip.
- `server/tests/robustness.rs`, against a real server:
  - not-our-protocol bytes, a 4 GiB frame length and 20 random bodies behind a valid prefix are all refused;
  - a newer protocol version gets the version prefix back;
  - offline players are refused without `--offline`; invalid names and a second login with the same name are refused;
  - 20,000 garbage datagrams, then an oversized control frame, close only the offending connection;
  - a well-behaved client joins after all that, and another one keeps receiving columns.
- `client/tests/multiplayer.rs`, a real server in the test process:
  - join through `LossyRelay` (2% loss per direction, RTT 100 ms) and control in ≤ 1 s, then the whole view (289 columns at view 8);
  - flying 12+ columns away streams the new area and unloads the start column; rejoining places the player where it left;
  - an unreachable server is a clear error.
- `cargo fmt --all --check`, `RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets`: clean.
- `cargo test -p client -p server -p net -p protocol`: green.
- `cargo bench --workspace --no-run`: compiles.

### Numbers

All on a cloud **VM** (4 vCPU Intel Xeon 2.1 GHz, KVM), dev profile (workspace crates at opt-level 1, dependencies at 3).

| What | Result |
| --- | --- |
| Start of connecting → control, 2% loss / RTT 100 ms (`multiplayer` test, run alone, 5 runs) | **0.68–0.70 s**; connect + handshake 0.21 s (2 RTT) |
| Same, while the whole test suite loads the VM (1 run) | 0.69 s |
| Login over loopback, view 12, server binary + `net_join` example (3 runs) | connect + handshake 3–4 ms; control after **675–733 ms**; all 625 columns received 0.42–0.47 s after the handshake |

On loopback the client's lighting and meshing, not the network, sets the time to control: every column has arrived before the nearest 3×3 is ready.

## What went wrong

- **Columns lost on rejoin.** The client pruned received columns relative to the viewer computed *before* `Spawn` moved the player, so columns near a returning player's position were thrown away and never resent. Fixed: received columns are kept until the player is spawned, and the viewer is recomputed after `Spawn`.
- **Slow streaming at first** (49 columns took 0.6 s after the handshake). The world thread noticed storage answers and finished jobs only on its 50 ms tick. Storage and jobs now send their results to the world thread's channel, which wakes it at once. The time to control dropped from ≈ 1.25 s to ≈ 0.9 s.
- **The welcome waited for the spawn column.** Admission and placement are now separate answers, so generating the spawn column overlaps the `Welcome` / `Registry` / `Ready` round trip.
- **`LossyRelay` reordered packets.** Each packet had its own timer task, so packets arriving within the same timer tick could leave in any order. The QUIC handshake then took 2–6 half-RTT steps instead of 2 (0.2–0.65 s), even with no loss. The relay is now one delay line that keeps order, like `tc netem delay` without jitter. The time to control became a steady 0.68–0.70 s (from 0.67–1.13 s).
- **Tried and reverted:** a lower QUIC initial RTT (100 ms instead of 333 ms). It made no measurable difference once the cause above was found.
- **Clippy:** `u32::is_multiple_of` is newer than the MSRV (replaced with `%`), and one `if` could be collapsed.

## Issues encountered

- **Laptop LAN login (≤ 5 s) is not measured yet**, so the task stays "in progress (cloud part done)". The owner's run:
  - server: `cargo run --release -p server -- --offline --bind <laptop LAN address>`
  - client: `cargo run --release -p client --example net_join -- <laptop LAN address>:47777` from another machine, or `cargo run --release -p client -- --offline-name NAME --connect <addr>` to play (it prints the login time).
- **The T0-11 spike numbers were measured with the reordering relay.** The spike's "nearest 3×3 at 2% loss" figures (0.46–0.77 s, Cubic 1.13 s) may be pessimistic. A re-run with the fixed relay is worth doing with the stage 2 bare-metal network work.
- **Stream limit.** The task fixes ≤ 16 column streams per client. Before the relay fix, 32 streams brought the 49 columns the 3×3 needs ≈ 0.2 s sooner (0.77 s vs 0.89 s to control). The owner can decide whether to raise the limit; not re-measured with the fixed relay.
- **Certificates:** open question 16 (offline servers only until it and T0-10 are answered).
- **Not in this task:**
  - other players' entities and reconciliation (T2-03): client and server simulate the same frames, but nothing corrects a divergence yet;
  - flight state is not in `Spawn`, so a returning player starts walking;
  - edits are refused on a server until T2-04;
  - the multiplayer menu (T2-08);
  - regional threads (T2-06).
- **Ctrl-C saving:** the server binary saves players when they leave and on `ServerHandle::shutdown`. A Ctrl-C kills the process without a final save (no signal handling yet; tokio's `signal` feature is not enabled). Only player positions are at stake until edits exist.
- **Pre-existing flake:** one full run of the client tests failed `client/tests/edit.rs` (`dig_and_place_keep_meshes_and_light_exact`) while the VM was loaded. It passed 6 times on its own and in the next full run; the T1-18 report notes the same flake. The code it covers was not changed here.
- **Stacked on T2-01.** This branch builds on PR #38 (T2-01), which is not merged yet. Merge #38 first.
