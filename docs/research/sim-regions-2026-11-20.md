# T0-09 region prototype measurements (2026-11-20)

Machine: ASUS TUF Gaming FX505DU, AMD Ryzen 7 3750H (4 cores / 8 logical
processors), approximately 14 GiB RAM, Windows 11 build 28000; physical
machine, not a VM. Rust 1.97.1. Release profile (thin LTO, one codegen unit)
for the latency test; Criterion bench profile for the benchmarks. CPU
frequency was not pinned and background activity was not controlled; these
are development measurements, not a production capacity guarantee.

## What was measured

The `sim` region prototype (T0-09): a flat world of 64 × 8 sections
(1024 × 128 blocks), one solid ground layer, scripted wandering bodies
(4 blocks/s, a deterministic direction change every 32 ticks). Partitioning:
1 region over the whole arena, or 8 regions of 8 × 8 sections. The region
pool uses `min(regions, logical CPUs)` threads (1 and 8 here).

The load is deliberately light — generic kinematics plus the region
machinery (ECS iteration in `NetId` order, phase overhead, sorted exchange,
transfer messages). These numbers bound the machinery cost; they are **not**
a gameplay capacity measurement (open question No. 6 stays with stage 2).

## Tick latency, 3 runs × 1 000 ticks (+100 warmup)

`cargo test -p sim --release --test tick_latency -- --ignored --nocapture`;
full tick = PreTick + Simulate + Exchange. Raw outputs:
`tick-latency-2026-11-20-run{1,2,3}.txt`. Values in ms, per run:

| Configuration | p50 (3 runs) | p99 (3 runs) | max (3 runs) |
| --- | --- | --- | --- |
| 100 bots, 1 region | 0.004 / 0.003 / 0.005 | 0.024 / 0.016 / 0.035 | 0.029 / 0.024 / 0.044 |
| 500 bots, 1 region | 0.018 / 0.022 / 0.022 | 0.054 / 0.061 / 0.062 | 0.119 / 0.070 / 0.100 |
| 2 000 bots, 1 region | 0.056 / 0.058 / 0.060 | 0.181 / 0.206 / 0.180 | 1.159 / 0.388 / 0.249 |
| 100 bots, 8 regions | 0.003 / 0.004 / 0.004 | 0.020 / 0.016 / 0.016 | 0.029 / 0.065 / 0.029 |
| 500 bots, 8 regions | 0.014 / 0.013 / 0.013 | 0.071 / 0.068 / 0.066 | 0.190 / 0.131 / 0.169 |
| 2 000 bots, 8 regions | 0.031 / 0.045 / 0.029 | 0.173 / 0.269 / 0.151 | 0.350 / 0.611 / 0.224 |

Every configuration is far under the 25 ms server tick budget with this
load. The single-run 1.159 ms max (2 000 bots, 1 region, run 1) is an
uncontrolled-background outlier; the other maxima are ≤ 0.611 ms.

## Criterion benchmarks

`cargo bench -p sim --bench region_tick -- --sample-size 30 --warm-up-time 1
--measurement-time 3`; estimate [95% CI], raw output
`criterion-2026-11-20.txt`. Per-tick values from `iter_custom` (state
evolves across iterations; no per-iteration setup):

| Benchmark | Estimate [95% CI] |
| --- | --- |
| `tick_100_bots_1_regions` | 4.8651 [4.8105, 4.9173] µs |
| `tick_500_bots_1_regions` | 21.358 [20.808, 22.134] µs |
| `tick_2000_bots_1_regions` | 66.573 [65.566, 67.708] µs |
| `tick_100_bots_8_regions` | 4.7017 [4.5617, 4.8570] µs |
| `tick_500_bots_8_regions` | 19.537 [18.960, 20.265] µs |
| `tick_2000_bots_8_regions` | 45.038 [43.502, 46.691] µs |
| `entity_transfer_between_worlds` | 168.52 [161.17, 179.38] ns |

The transfer benchmark is the despawn + spawn pair by stable `NetId` between
two `bevy_ecs::World`s (the merge/split/handoff primitive), including the
`NetId → Entity` index updates. It is ≈ 30× under the ≤ 5 µs hypothesis in
the performance budgets.

## Observations

- 8 regions vs 1 at 2 000 bots: ≈ 1.5× faster p50 (66.6 → 45.0 µs), p99
  comparable. The per-region work is small, so exchange, pool synchronization
  and SMT contention (8 logical CPUs on 4 physical cores) dominate the tail;
  the scaling headroom appears with heavier per-region systems, not with
  this load.
- At 100 bots the partitioning makes no measurable difference (≈ 4.7–4.9 µs
  either way): fixed phase/pool overhead dominates.
- First data for open question No. 6: one region carries 2 000 lightweight
  bots at p99 ≈ 0.18–0.21 ms. The capacity limit will be set by real gameplay
  systems (stage 2 benchmark), not by the region machinery.
- The spec's 500-player / 16-core / p99 ≤ 25 ms target is **not** verified
  here: this machine has 8 logical CPUs and the load is bots, not players.

## Reproduce

```bash
cargo test -p sim --release --test tick_latency -- --ignored --nocapture
cargo bench -p sim --bench region_tick -- --sample-size 30 --warm-up-time 1 --measurement-time 3
```

`SIM_LATENCY_TICKS` overrides the measured tick count (default 1 000).