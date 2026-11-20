# QUIC transport spike (T0-11, 2026-11-19)

First measurement of the `net` crate spike: one `quinn` connection with the
channels from [networking-protocol.md](../architecture/networking-protocol.md)
— datagrams for intents (client → server, duplicated 2×) and entity deltas
(server → client, 20 Hz), one prioritised unidirectional stream per column
(625 columns × 8 KB of deterministic random bytes, ordered by distance,
≤ 16 concurrent streams). Acceptance row: "Nearest 3×3 columns at 2% loss,
RTT 100 ms" in [performance-budgets.md](../process/performance-budgets.md).

## Machine and versions

- Cloud VM (same class as the T0-02 machine): **4 vCPU Intel Xeon Platinum**,
  7.5 GiB RAM, Linux 5.10. Not bare metal; treat numbers as ±30% like the
  other VM results.
- rustc/cargo **stable 1.98.1** — the pinned 1.97 (`rust-toolchain.toml`)
  could not be downloaded in the sandbox (mirror 404); the pin file was not
  changed. rustfmt/clippy 1.98.1 installed manually from the official dist.
- quinn 0.11.12 (quinn-proto 0.11.19), rustls 0.23.46 (ring provider),
  tokio 1.54.1, rcgen 0.14.7 (dev only), bytes 1.x (from `Cargo.lock`).
- Dev profile (`opt-level = 1`, dependencies `opt-level = 3`), one process
  hosting server, client and the loss relay.

## Method

`cargo run -p net --example quic_spike -- --loss 0.02 --rtt-ms 100 --secs 5
--seed <s> [--cc bbr|cubic]`. Loss and delay are emulated at the transport
level by `net::lossy::LossyRelay`: a loopback UDP relay that drops each
packet independently with probability 0.02 per direction (SplitMix64, seeded)
and delays every surviving packet by 50 ms per direction (RTT 100 ms). This
is the task's alternative to `tc netem` (no root in the sandbox). The relay
has **no bandwidth cap**: throughput below reflects how the congestion
controller reacts to random loss, not a link capacity.

The client measures from just before `connect` (handshake included). Delta
latency = arrival − send timestamp on a shared `Instant` epoch (one process).
The integration test `crates/net/tests/quic_spike.rs` asserts the acceptance
criteria with the same setup (seed 42); scenarios are serialized so parallel
tests do not distort timing.

## Results: 2% loss, RTT 100 ms, 5 s observation

Time to the nearest 3×3 columns (9 × 8 KB), per seed:

| Seed | BBR | Cubic (quinn default) |
| --- | --- | --- |
| 1 | 531 ms | 721 ms |
| 2 | 456 ms | 719 ms |
| 3 | 524 ms | **1129 ms** (> 1 s budget) |
| 4 | 519 ms | 719 ms |
| 5 | 766 ms | 759 ms |
| 42 (test seed) | 521 ms | 951 ms |

Entity-delta latency while columns stream (20 Hz, 136-byte datagrams):

| CC | p50 | p95 | max | deltas received / sent |
| --- | --- | --- | --- | --- |
| BBR | 51.1–51.3 ms | 51.8–58.0 ms | 137 ms | 87–92 / ~90 (0–3 lost) |
| Cubic | 64.6–102.4 ms | 118–149 ms | 153 ms | 89–95 / ~90 (0–2 lost) |

Columns delivered in the 5 s window (of 625): BBR **580–625**
(≈ 0.9–1.0 MB/s), Cubic **48–157** (≈ 0.08–0.25 MB/s). Duplicated intents:
87–92 unique ticks of 88–92 sent reached the server (≥ 95%) with both CCs.

## Results: no loss

| Path | First 3×3 | All 625 columns (5000 KiB) |
| --- | --- | --- |
| Loopback, no relay (BBR) | ≈ 2.9 ms | 75 / 92 / 96 ms (3 runs, ≈ 52–66 MB/s) |
| Relay, RTT 100 ms, 0% loss (BBR) | 433–435 ms | 4.45 s |

No delta was lost without loss; payloads verified byte-for-byte by the
client in every run.

## Verdict

- **Acceptance met with BBR**: nearest 3×3 in 456–766 ms across 6 seeds
  (budget ≤ 1 s), and deltas are not blocked by column streaming — p95
  51.8–58.0 ms sits at the 50 ms one-way path delay while ~1 MB/s of columns
  flow, with no stall in any 1-second bucket.
- **Cubic (quinn's default) misses the budget** on this path: 1 of 6 seeds
  over 1 s, delta p95 up to 149 ms, and ~6× less column throughput. Random
  (non-congestion) loss makes a loss-based controller halve its window
  repeatedly; BBR's delivery-rate model ignores it. The spike server
  therefore defaults to BBR (`net::server_endpoint`); `--cc cubic` in the
  example reproduces the comparison.
- Column streaming order works: the nearest 3×3 completes within the first
  ~13–20 arrivals even with 16 streams in flight (completion order inside
  one flight is arbitrary — priority orders transmission start, not finish).

## Caveats and follow-ups

- VM numbers, single process, no bandwidth cap and no competing traffic;
  re-run on bare metal with a shaped link (bandwidth cap + jitter) before
  stage 1 treats them as final. Real column sizes (4–16 KB compressed) will
  differ from the 8 KB assumption.
- The relay delays packets with a per-packet tokio task; timer granularity
  adds ~1 ms of noise (visible as p50 51 ms vs the 50 ms path delay).
- BBR vs Cubic on a *congested* bottleneck (shared with other flows) was not
  measured; BBR's fairness to loss-based flows is a stage 1 question.
- The spike handshake is 4 bytes of version each way; the real handshake
  (Hello/Welcome/Registry, `bitcode` messages in `protocol`) is stage 1.