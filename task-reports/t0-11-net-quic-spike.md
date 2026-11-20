# T0-11. QUIC transport spike

## What was done

Implemented the T0-11 spike in `crates/net` on quinn 0.11 (TLS 1.3, ALPN
`vxl/1`) with the channels from networking-protocol.md:

- `src/endpoint.rs` — server/client endpoint construction. The client trusts
  exactly the spike's self-signed certificate (no verification bypass).
  `server_endpoint` defaults to **BBR** congestion control (see below);
  `server_endpoint_with_transport` allows comparing controllers.
- `src/spike/` — server and client:
  - control: one bidirectional stream, a minimal 4-byte version handshake
    (the real Hello/Welcome/Registry with `bitcode` messages in `protocol`
    is stage 1);
  - input: datagrams, client → server, each intent duplicated 2×;
  - entities: datagrams, server → client at 20 Hz (136 bytes ≈ 20 entities);
  - chunks: one unidirectional stream per column, opened in Euclidean
    distance order, `set_priority(-rank)`, at most 16 concurrent streams
    (the window waits for stream acknowledgement, not just buffered writes).
  - Payloads are deterministic random bytes (task: no real column format);
    the client verifies every column byte-for-byte.
- `src/lossy.rs` — `LossyRelay`, a seeded loopback UDP relay with per-packet
  loss and one-way delay: transport-level loss emulation without root
  (the task's alternative to `tc netem`).
- `examples/quic_spike.rs` — 625 columns × 8 KB + 20 Hz deltas; flags
  `--loss`, `--rtt-ms`, `--secs`, `--seed`, `--cc bbr|cubic`; prints time to
  the first 3×3 columns and delta latency percentiles.
- `tests/quic_spike.rs` — integration tests: the acceptance scenario
  (2% loss, RTT 100 ms: 3×3 ≤ 1 s, deltas ≥ 60 in 5 s with no 1-second stall,
  delta p95 ≤ 250 ms, ≥ 90% of intents delivered), a lossless full transfer
  (all 625 columns intact, 0 deltas lost, 3×3 among the first 20 arrivals),
  and a wrong-version handshake rejection (clean error, no panic).
  Scenarios are serialized so parallel tests do not distort timing.
- Unit tests: wire framing roundtrips, garbage/truncation decoding without
  panics, distance ordering, relay forwarding/dropping, PRNG determinism.

Workspace dependencies added: `bytes = "1"` (quinn's datagram API takes
`bytes::Bytes`; already in the tree via quinn) and `rcgen = "0.14"` (dev-only,
self-signed certs for tests/examples, ring backend — no new C build).

**Key finding:** quinn's default Cubic misses the acceptance budget on a
random-loss path (worst 1.13 s over 6 seeds; delta p95 up to 149 ms), while
BBR meets it with margin (3×3 in 0.46–0.77 s; delta p95 52–58 ms ≈ the 50 ms
one-way delay) and delivers ~6× more column throughput. The server endpoint
therefore defaults to BBR; the spec and budgets were updated accordingly.

Numbers (4 vCPU Xeon VM, ±30%, rustc stable 1.98.1 — pinned 1.97 not
downloadable in the sandbox, pin file unchanged):
[docs/research/net-quic-spike-2026-11-19.md](../docs/research/net-quic-spike-2026-11-19.md).

Checks run: `cargo fmt --all --check`, `cargo clippy --workspace
--all-targets` with `RUSTFLAGS="-D warnings"`, `cargo test --workspace`,
`cargo bench --workspace --no-run` — all green. The `net` test suite was
 additionally run 5 times in a row to check timing stability (5/5 green).

Docs updated: networking-protocol.md (BBR decision, CI loss-emulation note),
performance-budgets.md (two new measured rows), docs/research/README.md.

## What went wrong

- First acceptance run failed (3×3 in 1.33 s): two causes — the lossless
  full-transfer test ran in parallel and distorted timing (fixed by
  serializing scenarios), and quinn's default Cubic genuinely misses the
  1 s budget at 2% random loss (fixed by defaulting to BBR, measured across
  6 seeds, recorded in the research doc rather than silently tuned away).
- The first lossless ordering assertion assumed strict distance order of
  arrivals; with 16 streams in flight, completion order inside one flight
  is arbitrary. Relaxed to "3×3 among the first 20 arrivals", which is what
  stream priority actually guarantees.
- `Option::map_err` does not exist (confused with `Result`); replaced with a
  `match`. `tokio::sync::Mutex::const_new` needs its value argument.

## Issues encountered

- The sandbox rustup mirror 404s for both the pinned 1.97 toolchain and the
  1.98.1 rustfmt/clippy components; components were installed manually from
  the official dist tarballs. The `wasm32-wasip2` std (needed by the existing
  `modhost` component tests) was also missing from the stable toolchain and
  was installed the same way; without it `cargo test --workspace` fails in
  `modhost` for environmental reasons, not because of this change.
  `rust-toolchain.toml` unchanged (per AGENTS.md).
- The relay has no bandwidth cap, so throughput numbers reflect congestion
  controller behaviour under random loss, not link capacity. Bare-metal
  re-run with a shaped link (bandwidth cap + jitter) is a stage 1 follow-up,
  as is BBR fairness on a shared congested bottleneck.
- Spike framing lives in `net::spike::wire` (fixed-size LE headers), not in
  `protocol`: the task scopes everything to `net` and excludes real message
  formats. Real `bitcode` message enums, the full handshake and zstd column
  compression move to `protocol`/`net` in stage 1.
- Branch name: the session's designated outcome branch
  (`qoder/general-session-cc802o`) is used instead of the AGENTS.md
  `t0-11-net-quic-spike` convention, per the platform's repository context.