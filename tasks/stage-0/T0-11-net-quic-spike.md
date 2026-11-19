# T0-11. QUIC transport spike

- **Status:** done
- **Depends on:** T0-01
- **Owner:** agent
- **Evidence:** [docs/research/net-quic-spike-2026-11-19.md](../../docs/research/net-quic-spike-2026-11-19.md), [task report](../../task-reports/t0-11-net-quic-spike.md)

## Goal

In the `net` crate: a server and client on `quinn` with the channels from networking-protocol.md: datagrams for intents and deltas, priority streams for columns. Test with loss emulation (in the test, artificial loss at the transport level, or `tc netem` in a CI script).

## Specs

- [docs/architecture/networking-protocol.md](../../docs/architecture/networking-protocol.md)
- [docs/adr/0007-quic-own-protocol.md](../../docs/adr/0007-quic-own-protocol.md)

## Deliverables

- Example: the server sends 625 "columns" of 8 KB each, ordered by distance, and 20 Hz deltas; the client measures time to the first 3×3 columns and delta latency.
- Integration test in `crates/net/tests/`.

## Acceptance criteria

- [x] At 2% loss and RTT 100 ms, the nearest 3×3 columns arrive ≤ 1 s; deltas are not blocked by column streaming. (BBR: 3×3 in 0.46–0.77 s over 6 seeds; delta p95 52–58 ms ≈ the 50 ms one-way delay while ~1 MB/s of columns stream. quinn's default Cubic misses the budget — the spike server defaults to BBR; see the research doc.)
- [x] Numbers in docs/research/.

## Out of scope

No real column format (use random bytes of the required size).
