# T2-02. Dedicated server: world, join, chunk streaming

- **Status:** in progress (cloud part done, report: [task-reports/t2-02-dedicated-server-world.md](../../task-reports/t2-02-dedicated-server-world.md); owner: login over LAN on the laptop)
- **Depends on:** T2-01, T1-02, T1-03
- **Owner:** agent (cloud)

## Goal

`server --offline` runs a world: QUIC endpoint (BBR, `net`), offline admission (T1-08), generation and saving (T1-03, T1-02), player join and spawn, and column streaming by distance over unidirectional streams (≤ 16 concurrent per client, priority by distance).

## Deliverables

- **Server world:** one region at first (T2-06 adds regional threading), a generation job pool and a storage writer.
- **Join:** the handshake and the first 3×3 columns before control (the "login ≤ 5 s" metric).
- **Streaming:** the rest of the radius in the background; unload and resend as the player moves.
- **Client "Multiplayer" mode:**
  - the loader takes columns from the network instead of generating them;
  - light stays on the client (ADR-0011);
  - meshing and rendering are unchanged.

## Acceptance criteria

- [x] A local client joins `server --offline` and plays in the streamed world; the nearest 3×3 is controllable in ≤ 1 s at 2% loss / RTT 100 ms (`LossyRelay` in CI): 0.68–0.70 s on the VM, `client` test `multiplayer`.
- [ ] Login to control ≤ 5 s on the laptop over LAN. VM over loopback: 0.68–0.73 s (`client` example `net_join`); the laptop run is the owner's (command in the report).
- [x] The server never panics on malformed client input (fuzzed decode, limits on sizes and rates): `server` test `robustness`.

## Out of scope

Other players' entities (T2-03), edits (T2-04), regions (T2-06).
