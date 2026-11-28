# T2-08: multiplayer menu and single-player on an embedded server

## What was done

**Single-player is a client of an embedded server** (`crates/client/src/session.rs`):

- `Session::open(dir)` starts `server::Server` in the process and joins it like any server. The server runs offline on `127.0.0.1:<free port>` with:
  - view distance = render distance + 2, simulation distance ≤ 8;
  - the settings' worker threads, 2 region threads;
  - the configured world bounds and no LAN announcement.

  The player is the offline name (`SessionConfig::player_name`). The local backend (loader generation, local `WorldStore` writes) is removed: `Session` always works through its server link, so single-player and multiplayer share prediction, reconciliation, edits, streaming and saving.
- Saving:
  - the server autosaves edited columns and players every 30 s (new `World::autosave`), saves columns when they leave every territory, and saves everything on shutdown;
  - a leaving player is saved after applying the intents it already sent (`Region::take_leaving`), also at shutdown;
  - `Session::close` waits up to 2 s until the server has applied the last sent tick (`ack_tick`) and answered pending edits, then leaves and stops the embedded server;
  - `world.toml` keeps the menu's view (name, seed, last played, player).
- Fly speed:
  - `Spawn` carries it (protocol version 3, golden files refreshed) and the player file saves it;
  - `fly_speed()` includes wheel notches not sent yet, computed exactly as the server will;
  - `set_fly_speed` rounds to whole ×1.25 notches (used by the T1-15 route).
- Migration: stage 1 worlds keep the player only in `world.toml`. On first open it is copied to `players/<player id>.toml`, and the seed goes to the server, which writes `server.toml`. Columns use the same T1-02 store.

**Multiplayer menu** (`menu.rs`, `app.rs`, `client::lan`, `settings.rs`):

- Title screen "Multiplayer" (needs a valid offline name).
- Direct connect: `host`, `host:port` or `[v6]:port`, default port 47777, host names resolved (`client::lan::resolve`).
- Saved servers: name + address in `settings.toml`, with add (validated), join and remove.
- "On this network": servers heard on the LAN while the screen is open. They can be joined if they speak this build's protocol; otherwise they are marked "other game version".
- LAN discovery (`protocol::lan`):
  - an offline server bound to a private, non-loopback address broadcasts `VXEN-LAN <protocol> <port> <name>` to UDP 47778 every 2 s (`ServerConfig::lan_announce`);
  - the listener accepts only private, link-local or loopback senders and forgets servers not heard for 7 s.

**Tests:**

- `crates/client/tests/session.rs` (now through the embedded server):
  - create → play (dig, walk, place) → quit → load returns the same blocks, columns and player position;
  - flight and fly speed are saved;
  - unloaded columns are reported to the renderer;
  - new: a stage 1 save (player only in `world.toml`, a glass column written with `WorldStore`) opens at the saved position, flying, with its fly speed and the saved column, and `server.toml` is written.
- Unit tests:
  - LAN announcement round trip and garbage;
  - the listener hears an announcement, ignores garbage and marks another protocol version;
  - address parsing and resolution;
  - big seeds survive `server.toml`.
- Manual check in the cloud: `client --offline-name Tester --autoplay Test --screenshot` under Xvfb with software Vulkan opened a single-player world through the embedded server and rendered it (first menu frame after 609 ms). The world folder has `server.toml` and `players/`.
- `cargo fmt --all`, `cargo clippy --workspace --all-targets` with `-D warnings`, `cargo test --workspace`.

## What went wrong

- **Edits placed right before quitting were lost**: the server stopped before applying them. `close` now waits for the answers.
- **Fly speed came back as 1.0**: the spawn did not carry it. It is now part of `Spawn` and the player file.
- **Position after reload differed.**
  - First cause: the server had not applied the last intents yet. `close` now waits for the server's `ack_tick`, and leaving applies the queued intents.
  - Second cause, found in the test helper: it ticked the client about every 3 ms, ≈ 250 Hz instead of 20 TPS. The server applies at most 3 intents per tick and drops the oldest beyond 40 queued, so the simulations diverged. The helper now ticks at the real rate, as the game does.
  - Third cause: digging under one's own feet is predicted at once but applied by the server a tick later, so the server may correct the position. The test now stands still before recording the position. This is normal network play, and the server's state wins.
- **The block-placing part of the round-trip test** depended on where the player ended up after walking; it now sweeps the view until it finds a placeable face.
- **Xvfb needed `libxkbcommon-x11`** in this container again (installed for the session only, as in T1-13).

## Issues encountered

- **Transport: QUIC over loopback, not an in-memory channel.** The task asked for "a local channel"; loopback keeps exactly one code path (handshake, TLS, streams) at well under a tick of latency. An in-memory transport could come later if profiling shows the loopback cost.
- **Single-player now has the server's semantics.** Digging under one's feet can cause a small smoothed correction, and the world (entities later) keeps ticking while paused; the player stands still because no intents arrive. The T1-15 route needs the owner's re-run to show "feels unchanged" (within 5% of the stage 1 numbers). The route's fly speed is now 1.25⁻³ ≈ 0.51 instead of 0.5 (whole wheel notches), about 2% faster.
- **CPU in single-player** includes the embedded server (2 region threads plus generation workers). Generation moved from the client's loader jobs to the server's pool, so the total work is similar, but the laptop run decides.
- **LAN:** discovery needs UDP broadcast on 47778 to be allowed by the OS firewall. If another program holds the port, the menu says discovery is unavailable; direct connect still works. "Open to LAN" for a running single-player world is not done (a dedicated `server --offline --bind <LAN address>` is the way for now).
- **Owner checks left:** the T1-15 route on the laptop, and a two-laptop LAN game (or two clients on one machine via the multiplayer screen).
