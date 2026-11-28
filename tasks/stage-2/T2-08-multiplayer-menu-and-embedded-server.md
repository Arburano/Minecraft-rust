# T2-08. Multiplayer menu and single-player on an embedded server

- **Status:** in progress (cloud part done; owner: the T1-15 route and a two-laptop LAN game) — [report](../../task-reports/t2-08-multiplayer-menu-and-embedded-server.md)
- **Depends on:** T2-02, T2-04
- **Owner:** agent (cloud) + owner's playtest

## Goal

- The client menu gains Multiplayer: direct connect, a saved server list, LAN (offline, private only; legal.md rule 7).
- Single-player runs an embedded server in-process over a local channel, so there is one code path for both modes. `client::session` becomes a client of it.
- World saves stay compatible (migration test).

## Acceptance criteria

- [ ] Single-player feels unchanged (the T1-15 route on the laptop within 5% of the stage 1 numbers).
- [ ] Two laptops (or two clients on one) play together over LAN.

## Out of scope

Public servers before T0-10; mod sync (stage 3).
