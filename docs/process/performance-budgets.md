# Performance budgets

The single source of numbers. Change here and in PRD §1 at the same time. Status: **Target** — a commitment; **Hypothesis** — not yet measured; **Measured** — a benchmark exists (marked with what it ran on).

## Engine core (client and server)

| What | Value | Status |
| --- | --- | --- |
| Job scheduler: submit + complete overhead | ≤ 2 µs per job | Measured (VM: ≈ 0.37 µs/job under load, ≈ 0.57 µs idle round trip; `engine-core` bench `jobs`, [details](../research/job-scheduler-2026-11-15.md)) |

## Client

| What | Value | Status |
| --- | --- | --- |
| FPS on iGPU / GTX 1050, 12 chunks | ≥ 60, 1% low ≥ 45 | Target |
| Frame spike | none > 33 ms over 30 min of walking | Target |
| Minimum full-data distance | 8 chunks (Low), 12 (default), 16 (High) | Target |
| Distance with LOD | 64 chunks on GTX 1050 | Target (stage 5) |
| Client RAM, 12 chunks | ≤ 1 GiB internal budget; ≤ 2 GB external target | Hypothesis: 0.5–0.9 GiB |
| Cold start to menu | ≤ 5 s | Target |
| Joining a server until control | ≤ 5 s with asset cache | Target |
| GPU geometry time | ≤ 8 ms per frame on iGPU | Measured (Radeon RX Vega 10, path B as the engine would select: 2.417 ms p50 / 2.619 ms p95 on DX12, worst backend GL 2.612 / 2.821; path C 1.637–1.948 ms p50; bare-metal ASUS FX505DU, 3 runs per combination, [T0-14 summary](../research/gpu/summary-2026-11-19.md)) |
| CPU command recording | ≤ 1 ms per frame | Measured (Ryzen 7 3750H, worst adapter/backend Vega 10 + DX12: 0.535 ms p50 / 0.821 ms p95 median of 3 runs, worst single-run p95 0.923 ms; judged at p95 — the budget does not name a statistic; single-frame maxima exceed 1 ms: 1.323 ms Vega 10 DX12 B, 1.061 ms Vega 10 DX12 C; [T0-14 summary](../research/gpu/summary-2026-11-19.md)) |
| `fast_mesh` of a 32³ mesh chunk | ≤ 0.2 ms p50 on a 4-core desktop | Measured (bare-metal Ryzen 7 3750H: ≈ 54 µs on the T0-12 corpus, seed 0, median of 3 runs [VM: ≈ 49 µs]; `mesh` bench `mesh_chunk/fast_mesh`, [T0-16 summary](../research/bare-metal-2026-11-21/summary.md)) |
| Gather + masks of a 32³ mesh chunk | ≤ 100 µs p50 | Measured (bare-metal Ryzen 7 3750H: 79.6–83.6 µs, median 82.0 µs [was 112.8 µs in T0-16]; target met after T1-04 division-free addressing; `mesh` bench `mesh_chunk/gather_masks`, [T1-04 summary](../research/bare-metal-2026-11-22-t1-04/summary.md)) |
| Mask update per edit | ≤ 20 ns | Measured (bare-metal Ryzen 7 3750H: ≈ 4.4 ns on the T0-12 corpus [VM: ≈ 3.7 ns]; `mesh` bench `mesh_chunk/mask_update`, [T0-16 summary](../research/bare-metal-2026-11-21/summary.md)) |
| Render batch copy after an edit | ≤ 50 µs | Measured (bare-metal Ryzen 7 3750H: ≈ 1.7 µs full re-copy of a 64³ batch, 5 410 quads / 42.3 KB on the T0-12 corpus; sub-range copy ≈ 0.17 µs [VM: ≈ 1.2 µs / 0.15 µs]; `mesh` bench `mesh_chunk/render_batch`, [T0-16 summary](../research/bare-metal-2026-11-21/summary.md), [T0-05 report](../../task-reports/t0-05-mesh-render-batches.md)) |
| Lighting: add / remove per edit | ≤ 100 / ≤ 200 µs p95 | Measured (bare-metal Ryzen 7 3750H, 3 runs: T0-12 corpus with 8 torches/section — add ≤ 2.8 µs p95, remove ≤ 14.2 µs p95; synthetic worst cases ≤ 24.7 / ≤ 20.5 µs p95 [VM 4 vCPU Xeon 2.1 GHz: add ≤ 3.4 µs, remove ≤ 16.8 µs, worst ≤ 24.5 µs p95]; `light` bench, [T0-16 summary](../research/bare-metal-2026-11-21/summary.md), [T0-08 report](../../task-reports/t0-08-light-incremental-bfs.md)) |
| Lighting through the world path (snapshot + import + relight + export) | accepted for stage 1 (owner 2026-11-22): updates deferred and capped per tick, the ≤ 100 / 200 µs row above applies to the BFS; faster algorithm with the same look in T1-16 | Measured (VM 4 vCPU Xeon 2.1 GHz, 3 runs, generated world y −64..320, 192 edits per kind per run): torch place/remove p50 0.66–0.94 ms, p95 0.71–1.16 ms; a block that changes direct sky (stone on the surface) p50 ≈ 9.1–9.2 ms, p95 12.8–15.5 ms (recomputes the sections below); initial light 5.4–5.6 ms p50 per column (24 sections, about 11 by BFS); `light` bench `world_light`, [T1-09 report](../../task-reports/t1-09-light-in-world.md) |
| `get` in a section | ≈ 3.8 ns | Measured (bare-metal Ryzen 7 3750H: 3.03–3.15 ns paletted, 0.85 ns single, 1.14 ns direct; T0-12 corpus 2.91–2.99 ns paletted; target met after T1-04 division-free addressing [was 8.3–8.8 ns in T0-16]; `world` bench `section_get` / `section_corpus/get`, [T1-04 summary](../research/bare-metal-2026-11-22-t1-04/summary.md)) |
| `set` in a section, any bit width | ≤ 10 ns | Measured deviation (bare-metal Ryzen 7 3750H: 11.4–17.5 ns across widths [improved ~1.8× from 12.7–29.4 ns in T0-16]; T0-12 corpus 11.7–20.4 ns; target kept, follow-up in stage 1; `world` bench `section_set` / `section_corpus/set`, [T1-04 summary](../research/bare-metal-2026-11-22-t1-04/summary.md)) |
| `unpack_into` per section | ≤ 5 µs | Measured (bare-metal Ryzen 7 3750H: ≈ 2.8–3.0 µs paletted, 0.14–0.16 µs single/direct; T0-12 corpus ≤ 3.91 µs paletted, 0.15 µs single [VM: ≈ 2.9–4.1 µs paletted, < 0.1 µs single/direct; corpus ≈ 3.3 µs / 0.08 µs]; `world` bench `section_unpack_into` / `section_corpus/unpack`, [T0-16 summary](../research/bare-metal-2026-11-21/summary.md)) |
| World map `get_block` / `set_block` | within 2× of section `get` / `set` (owner decision 2026-11-22: target kept, optimize later where possible) | Measured deviation (VM 4 vCPU Xeon 2.1 GHz, 3 runs, 4,096 random cells on the T0-12 corpus, 64 columns): `get_block` ≈ 9.2–9.7 ns (2.8× section `get` ≈ 3.3 ns), `set_block` ≈ 44–50 ns (2.4× section `set` ≈ 19 ns, includes heightmap and dirty tracking); the 2× proposal is not met; `world` bench `world_map`, [T1-01 report](../../task-reports/t1-01-world-columns-registry.md) |
| Fallback asset conversion | — (feeds "Cold start to menu ≤ 5 s") | Measured (VM 4 vCPU Xeon 2.1 GHz, 3 runs): all 16 fallback textures generated + converted with mip chains ≈ 39–42 µs, 21,824 bytes; cache entry decode ≈ 1.2–1.5 µs; `assets` bench, [T1-07 report](../../task-reports/t1-07-assets-fallback-and-cache.md) |
| World save / load (region format v1) | — (background job; no frame budget) | Measured (VM 4 vCPU Xeon 2.1 GHz, 3 runs, T0-12 corpus, 8 sections/column, no light): one column save 0.34–0.38 ms incl. 2 fsyncs, load 42–47 µs; 625 columns (12-chunk radius) save 31.7–32.1 ms, load 28.6–34.2 ms; 680 B/column compressed (3.9 KB raw); `world` bench `world_storage`, [T1-02 report](../../task-reports/t1-02-world-save-format.md) |
| World generation per column | ≤ 1 ms per core incl. packing (owner 2026-11-22: reach it where reasonable, not at the cost of rewriting the project) | Measured (VM 4 vCPU Xeon 2.1 GHz, 3+2 runs): all stages 0.28–0.37 ms, packing into sections 0.26–0.32 ms (was 1.46–1.66 ms before the `from_blocks` rewrite), 64 columns on 4 threads 6.0–7.8 ms; `worldgen` bench `pipeline`, [T1-03 report](../../task-reports/t1-03-worldgen-pipeline.md) |
| Nearest 3×3 columns ready after spawn (generated, lit, meshed) | ≤ 0.5 s (proposed in T1-10, owner to confirm; laptop run pending) | Measured (VM 4 vCPU Xeon 2.1 GHz, 3 workers at low priority, radius 12, y −64..320, 3 runs): 0.47–0.49 s; full radius 12 ready 2.0–2.2 s; `client` example `flythrough`, [T1-10 report](../../task-reports/t1-10-loading-queue.md) |
| Loader main-thread work per tick | apply ≤ 2 ms (T1-10) | Measured (same runs): apply batch max 0.91–1.46 ms; whole tick max 2.2 ms at spawn, ≤ 3.5 ms while flying at 20 blocks/s (submission stops at a 2 ms tick budget; the overshoot is the last job's input copy) |

GTX 1050 targets (FPS on iGPU / GTX 1050 and 64-chunk LOD distance) remain
**not verified on GTX 1050**: no such hardware is available (owner decision,
[open question No. 12](../open-questions.md)); the T0-14 discrete-GPU evidence
was taken on a GTX 1660 Ti, which is considerably faster and does not verify
those targets. They stay unchanged.

Bare-metal confirmation of the VM-only client rows was taken in T0-16
(2026-11-21) on the owner's laptop: three criterion runs per crate
(`world`, `mesh`, `light`), AC power, best-performance mode, machine idle.
CPU frequency could not be pinned; after ≈ 35 minutes of sustained load the
laptop dropped to its base clock (≈ 1.5× slower), so the confirmed numbers
are the boost-state ones. The misses (`get`, `set`, gather + masks) are
recorded as deviations with targets unchanged, per the owner decision —
see the [T0-16 summary](../research/bare-metal-2026-11-21/summary.md) and the
stage 1 carry-over in `tasks/README.md`.

## Server

| What | Value | Status |
| --- | --- | --- |
| Tick with 500 players per node (16+ cores, players in ≥ 8 regions) | ≤ 25 ms p99, hard 50 ms | Target |
| Capacity of one region | determined by the stage 2 benchmark | Open (first data: T0-09, 2 000 lightweight bots in 1 region at p99 ≈ 0.18–0.21 ms — machinery overhead only, not gameplay capacity) |
| Entity transfer between `World`s | ≤ 5 µs | Measured (bare-metal Ryzen 7 3750H: ≈ 168.5 ns despawn + spawn by `NetId` incl. index updates; `sim` bench `region_tick/entity_transfer_between_worlds`, T0-09, [details](../research/sim-regions-2026-11-20.md)) |
| Entity handoff between nodes | ≤ 50 ms p99, 0 dupes | Target (stage 6) |
| Network CPU | ≤ 1 core per 10,000 packets/s | Hypothesis |
| Nearest 3×3 columns at 2% loss, RTT 100 ms | ≤ 1 s | Measured (VM, T0-11 spike, 8 KB random columns, BBR: 0.46–0.77 s over 6 seeds; quinn's default Cubic misses it — 1.13 s worst; [details](../research/net-quic-spike-2026-11-19.md)) |
| Entity-delta latency while columns stream (2% loss, RTT 100 ms) | not blocked by chunk streams | Measured (VM, T0-11 spike, BBR: p95 52–58 ms ≈ the 50 ms one-way path delay; [details](../research/net-quic-spike-2026-11-19.md)) |
| Traffic per player, steady state | ≤ 50 kbit/s (≤ 20 moving entities nearby) | Target |
| Node RAM, 500 scattered players | ≤ 16 GB | Hypothesis |

### T0-09 measurements (2026-11-20)

Same machine and toolchain as T0-07 (ASUS TUF FX505DU, Ryzen 7 3750H, 8 logical CPUs, Windows 11 build 28000, physical; Rust 1.97.1). Region prototype: flat world 64 × 8 sections, scripted wandering bodies (4 blocks/s, turn every 32 ticks). Full tick = PreTick + Simulate + Exchange; pool of `min(regions, logical CPUs)` threads. 3 runs × 1 000 ticks (+100 warmup), release profile; Criterion tick benchmarks with 30 samples, 1 s warmup, 3 s measurement. Raw outputs in [docs/research/sim/](../research/sim/), tables and interpretation in the [T0-09 summary](../research/sim-regions-2026-11-20.md).

| Configuration | p50 (3 runs) | p99 (3 runs) | Criterion tick estimate |
| --- | --- | --- | --- |
| 100 bots, 1 region | 0.003–0.005 ms | 0.016–0.035 ms | 4.8651 µs |
| 500 bots, 1 region | 0.018–0.022 ms | 0.054–0.062 ms | 21.358 µs |
| 2 000 bots, 1 region | 0.056–0.060 ms | 0.180–0.206 ms | 66.573 µs |
| 100 bots, 8 regions | 0.003–0.004 ms | 0.016–0.020 ms | 4.7017 µs |
| 500 bots, 8 regions | 0.013–0.014 ms | 0.066–0.071 ms | 19.537 µs |
| 2 000 bots, 8 regions | 0.029–0.045 ms | 0.151–0.269 ms | 45.038 µs |

All configurations are far under the 25 ms tick budget with this load. This measures the region machinery with light kinematic bodies, **not** player capacity; the 500-player / 16-core Target above stays unverified (this machine has 8 logical CPUs). 8 regions give ≈ 1.5× p50 speedup at 2 000 bots with comparable p99 — exchange and pool synchronization dominate the tail at this load. CPU frequency was not pinned and background activity was not controlled.

## Mods

| What | Value | Status |
| --- | --- | --- |
| Batched hook dispatch | ≤ 1 µs | Measured deviation: 2.2675–2.2873 µs end-to-end empty hook on Ryzen 7 3750H, Windows (T0-13, 2026-11-20; includes the owner-approved per-call thread-priority elevation, ≈ +1 µs over the same-session baseline 1.2994 µs); T0-07 below |
| Event in a batch | ≤ 5 ns | Measured deviation: 308.27 ns/event in a 1,000-string-event batch on the same machine; T0-07 below |
| Epoch cost | ≤ 10% | Measured (VM: 0–9%; earlier research, not remeasured in T0-07) |
| Deadline-to-interruption overrun | ≤ 1 ms p99 on a quiet desktop (measured, not a CI assertion); CI functional bound ≤ 10 ms — owner decision, open question 11 | Measured (bare-metal Ryzen 7 3750H, Windows, 3 runs × 1,000 interruptions per condition, 2026-11-20: quiet p99 0.633–0.637 ms, max ≤ 2.961 ms; all cores busy p99 0.534–0.539 ms, max ≤ 4.038 ms; 20/20 debug functional runs under load passed with maxima 356–973 µs; T0-13 below) |
| Cold start of a mod with cache | ≤ 50 ms | Measured upper bound (VM 4 vCPU Xeon 2.1 GHz, 3 runs): **without** a cache, compile + link + instantiate + `init` of the 127 KB hello component 12.3–13.9 ms; the disk compilation cache is not implemented and larger mods compile longer; `modhost` bench `modhost_cold_start`, [stage 0 exit review](../research/stage-0-exit-2026-11-21.md) |
| Default mod | 0.5 ms per tick per region, 32 MB | Target |

### T0-07 measurements (2026-11-19)

Machine: ASUS TUF Gaming FX505DU, AMD Ryzen 7 3750H (4 cores / 8 logical processors), approximately 13.94 GiB usable RAM, Windows 11 build 28000; physical machine, not a VM. Rust 1.97.1, Wasmtime 50.0.2, wit-bindgen 0.63.0, Criterion 0.8.2. Release profile with thin LTO and one codegen unit. CPU frequency was not pinned and background activity was not controlled; these are development measurements, not a production capacity guarantee.

Three consecutive Criterion runs, each with 30 samples, 1 s warmup and 3 s requested measurement time per benchmark:

```bash
cargo bench --workspace --bench hooks -- --sample-size 30 --warm-up-time 1 --measurement-time 3
```

| Benchmark | Run 1 estimate [95% CI], µs | Run 2 estimate [95% CI], µs | Run 3 estimate [95% CI], µs |
| --- | --- | --- | --- |
| `modhost/empty_run_system` | 1.1604 [1.1388, 1.1835] | 1.1396 [1.1259, 1.1555] | 1.1508 [1.1324, 1.1733] |
| `modhost/on_events_1000_strings` | 324.83 [314.61, 339.71] | 308.14 [303.96, 313.26] | 308.27 [303.26, 313.10] |
| `modhost/set_blocks_4000` | 148.17 [142.90, 155.25] | 139.88 [137.41, 142.29] | 137.84 [135.53, 140.83] |

The median of the three point estimates is 1.1508 µs/hook, 308.27 µs/event batch (308.27 ns/event), and 139.88 µs/block batch (34.97 ns/change). These are not per-call latency percentiles. The run-to-run range exceeds 5% for events and blocks; do not interpret differences at that scale as an optimization result.

The engine, pre-linked component, instance, initialization and host event construction are outside the timed loop. Measurements include the epoch-enabled hook wrapper, canonical ABI work, guest execution, host imports, validation and transaction commit. Each event is a `Message` with a 13-byte channel and 4-byte payload; the guest consumes the strings and writes a byte count to a host component. The block benchmark alternates two registered states so that every iteration changes 4,000 blocks rather than repeating no-op writes.

**Documented deviations:** the empty hook is approximately 15.1% above its target; the measured event path is approximately 61.7 times the event target. Owned string/list marshalling, allocation and guest/host work are included, unlike the earlier VM experiments (approximately 0.4 µs/component call and 3.6 ns/element of `list<u32>`). Those experiments are not comparable before/after measurements of this implementation. The targets remain unchanged; the spike is not allocation-free, and it does not establish the default 0.5 ms budget for every hook.

**Interruption:** `park_timeout(200 µs)` initially produced 4.7229 ms and 4.3402 ms overruns on Windows. Switching the watchdog to `thread::sleep(200 µs)` uses the platform's high-resolution sleep timer. With that correction, one repeated test during concurrent compilation still failed at 1.5101 ms. After compilation finished, three complete integration-suite repetitions (10 interruptions each) passed with maxima 689.4, 458.4 and 593.2 µs. A further ten focused repetitions (100 interruptions total) passed with maxima 604.6, 476.9, 550.6, 565.9, 603.0, 444.1, 562.8, 579.2, 583.7 and 534.1 µs. Each uses a 10 ms hook budget and requires all 4,000 writes to be staged before the infinite loop, then discarded while another worker continues the tick. The strict 1 ms assertion remains enabled; quiet passes do not resolve the observed failure under load or prove an OS scheduling guarantee. T0-07 remains in progress pending that acceptance criterion; the follow-up is [T0-13](../../tasks/stage-0/T0-13-modhost-interruption-latency.md).

See the [T0-07 report](../../task-reports/t0-07-wit-and-modhost-spike.md) for verification and limitations.

### T0-13 measurements (2026-11-20)

Same machine and toolchain as T0-07 (ASUS TUF FX505DU, Ryzen 7 3750H, 8 logical CPUs, Windows 11 build 28000, physical; Rust 1.97.1, Wasmtime 50.0.2). Release profile, 100 ms hook budget so every interruption is epoch-driven; "quiet" = heavy applications closed by the owner, background activity not controlled; "loaded" = a spin thread on every logical CPU. Evidence: `research/modhost/run-latency.cmd`, raw outputs and per-interruption samples in [docs/research/modhost/](../research/modhost/), diagnosis and kept/rejected optimizations in the [T0-13 report](../../task-reports/t0-13-modhost-interruption-latency.md).

Three runs per condition, 1,000 interruptions each, deadline-to-return overrun:

| Condition | p50 (3 runs) | p99 (3 runs) | max (3 runs) |
| --- | --- | --- | --- |
| Quiet | 0.327 / 0.324 / 0.353 ms | 0.633 / 0.633 / 0.637 ms | 2.070 / 1.340 / 2.961 ms |
| All cores busy | 0.301 / 0.300 / 0.312 ms | 0.539 / 0.534 / 0.534 ms | 0.694 / 4.038 / 0.984 ms |

The quiet-desktop p99 ≤ 1 ms target was met in all six runs (informational; under load it is expected to be fragile on other machines). The CI functional test (debug, ≤ 10 ms bound per owner decision No. 11) passed 20/20 consecutive runs under full CPU load with per-run maxima 356.4–973.2 µs.

Same-session baseline reference (original always-ticking watchdog, backtraces on, normal priorities): quiet p99 1.019 ms / max 26.041 ms; loaded p99 35.199 ms / max 78.794 ms. The kept reductions are interruption backtraces off, raised watchdog priority, per-call worker-thread priority elevation (owner-approved; ≈ +1 µs per hook, three syscalls) and in-flight gating of the watchdog (no idle wakeups); a 100 µs tick was measured and rejected (no gain, +5% empty-hook cost). Final empty-hook bench on the idle machine: 2.2873 and 2.2675 µs (two runs, 30 samples each) vs the same-session baseline 1.2994 µs; a first post-load measurement of 3.5844 µs was discarded as a machine-state outlier and re-measured. Breakdown: watchdog wake dominates (p50 ≈ 0.3 ms), trap/unwind ≈ 13–15 µs p50, measuring-thread preemption is reported separately (post-return max up to ≈ 35 ms under load while the engine-internal overrun stayed ≤ 4.038 ms).

Earlier "VM" figures — 2 vCPU Xeon 2.1 GHz, noise ±30%: `docs/research/optimization-tests.md`.
