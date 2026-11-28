# T2-11: the client without the server's code

## What was done

Owner request (2026-11-28): the client must not depend on the server's files. Single-player and playing with each other must stay possible.

- **New crate `netcode`** (depends on `protocol`, `sim`, `glam`). `server/src/input.rs` and `server/src/replication.rs` moved there unchanged, with their tests:
  - `input`: intent ↔ physics input, state hash, corrections, fly-speed steps;
  - `replication`: the per-client replicator and the client's reconstructor.

  The server re-exports both (`pub use netcode::{input, replication}`), so server code and tests keep their paths.
- **Client:**
  - depends on `netcode` for these;
  - `server` is an optional dependency behind the new feature `singleplayer` (default on);
  - `client::embedded` holds everything about the embedded server: starting it for a world and the stage 1 save migration (player file). Without the feature, `EmbeddedServer` is an empty type and `start` returns a clear message;
  - `Session::open` uses `client::embedded`; `Session::connect` never touched the server;
  - the "Single-player" button is shown only with the feature;
  - `client::capabilities()` computes the compiled set from the client's own `moving_grids` feature; a test checks it equals the embedded server's;
  - `moving_grids = ["server?/moving_grids"]` (weak dependency feature), and `bots` has `required-features = ["singleplayer"]` (its `--embedded` mode runs a server);
  - integration tests still run a real server: `server` is a dev-dependency.
- **CI:** `cargo check -p client --no-default-features`, plus a check that its normal dependency tree has no `server`.
- **Docs:** `overview.md` (new `netcode` row; `client` and `server` rows), `client-menu-and-updater.md`, `networking-protocol.md`, `simulation-and-ai.md`.

**Checks:**

- `cargo build -p client` and `cargo build -p client --no-default-features` both build;
- `cargo tree -p client --no-default-features -e normal` has no `server`, `netcode` replaces it;
- `cargo fmt --all`, `cargo clippy --workspace --all-targets` with `-D warnings`, `cargo test --workspace`.

## What went wrong

None.

## Issues encountered

- **Playing with each other** works through a server: a dedicated `server` on a LAN address now, or the owner's online servers later. "Open to LAN" for a running single-player world (the embedded server listening on the LAN) is not done. It would be a small follow-up on top of `client::embedded`.
- **No separate player release profile yet.** A build for players without single-player is `cargo build --release -p client --no-default-features --features moving_grids` (or without it, per the build's capabilities). The updater (ADR-0010) will decide which binary ships.
- **`netcode` naming:** it is game logic shared by both ends, not transport (that stays in `net`). The crate table says so.
