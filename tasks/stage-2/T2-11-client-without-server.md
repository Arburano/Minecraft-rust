# T2-11. The client without the server's code

- **Status:** done
- **Depends on:** T2-08
- **Owner:** agent (cloud)

## Goal

Owner request (2026-11-28): the game is meant to be played online on the owner's servers, so the client must not depend on the server's files. Single-player on an embedded server stays possible, so that players can still play locally and with each other.

## Deliverables

- A `netcode` crate for what both ends compute identically: intent ↔ physics input, state hash, corrections (`input`), entity deltas (`replication`). The server re-exports them.
- The client depends on `netcode`, not on `server`. The embedded server is the optional `singleplayer` feature (default on, `client::embedded`).
- Without the feature there is no "Single-player" button, and `Session::open` explains that single-player is not part of the build. `bots` requires the feature.
- CI checks that `cargo check -p client --no-default-features` builds and that its dependency tree has no `server`.
- Docs: `overview.md` (crate table), `client-menu-and-updater.md`, `networking-protocol.md`, `simulation-and-ai.md`.

## Acceptance criteria

- [x] `cargo tree -p client --no-default-features -e normal` lists no `server`.
- [x] The default build behaves as before (all tests green).
- [x] The client's compiled capabilities equal the embedded server's (test).

## Out of scope

"Open to LAN" for a running single-player world; a separate release build profile for players.
