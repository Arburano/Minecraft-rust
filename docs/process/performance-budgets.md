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
| `fast_mesh` of a 32³ mesh chunk | ≤ 0.2 ms p50 on a 4-core desktop | Measured (VM: ≈ 42 µs on the T0-04 terrain corpus; `mesh` bench `mesh_chunk/fast_mesh`) |
| Gather + masks of a 32³ mesh chunk | ≤ 100 µs p50 | Measured (VM: ≈ 60 µs on the T0-04 corpus; `mesh` bench `mesh_chunk/gather_masks`, [details](../../task-reports/t0-04-mesh-chunk-32.md)) |
| Mask update per edit | ≤ 20 ns | Measured (VM: ≈ 2.6 ns; `mesh` bench `mesh_chunk/mask_update`) |
| Render batch copy after an edit | ≤ 50 µs | Measured (VM: ≈ 1.6 µs full re-copy of a 64³ batch, 4 976 quads / 38.9 KB on the T0-04 corpus; sub-range copy ≈ 0.2 µs; `mesh` bench `mesh_chunk/render_batch`, [details](../../task-reports/t0-05-mesh-render-batches.md)) |
| Lighting: add / remove per edit | ≤ 100 / ≤ 200 µs p95 | Hypothesis |
| `get` in a section | ≈ 3.8 ns | Measured (VM: ≈ 3.4 ns paletted, 0.5 ns single, 0.7 ns direct; `world` bench `section_get`) |
| `set` in a section, any bit width | ≤ 10 ns | Measured (VM: ≈ 6 ns typical; 6–8-bit readings of 13–15 ns are 4K-aliasing stalls on this VM, see [T0-03 report](../../task-reports/t0-03-world-palette-section.md); bare-metal confirmation pending) |
| `unpack_into` per section | ≤ 5 µs | Measured (VM: ≈ 2.9–4.1 µs paletted, < 0.1 µs single/direct; `world` bench `section_unpack_into`) |

GTX 1050 targets (FPS on iGPU / GTX 1050 and 64-chunk LOD distance) remain
**not verified on GTX 1050**: no such hardware is available (owner decision,
[open question No. 12](../open-questions.md)); the T0-14 discrete-GPU evidence
was taken on a GTX 1660 Ti, which is considerably faster and does not verify
those targets. They stay unchanged.

## Server

| What | Value | Status |
| --- | --- | --- |
| Tick with 500 players per node (16+ cores, players in ≥ 8 regions) | ≤ 25 ms p99, hard 50 ms | Target |
| Capacity of one region | determined by the stage 2 benchmark | Open |
| Entity transfer between `World`s | ≤ 5 µs | Hypothesis |
| Entity handoff between nodes | ≤ 50 ms p99, 0 dupes | Target (stage 6) |
| Network CPU | ≤ 1 core per 10,000 packets/s | Hypothesis |
| Traffic per player, steady state | ≤ 50 kbit/s (≤ 20 moving entities nearby) | Target |
| Node RAM, 500 scattered players | ≤ 16 GB | Hypothesis |

## Mods

| What | Value | Status |
| --- | --- | --- |
| Batched hook dispatch | ≤ 1 µs | Measured deviation: 1.1508 µs end-to-end empty hook on Ryzen 7 3750H, Windows; T0-07 below |
| Event in a batch | ≤ 5 ns | Measured deviation: 308.27 ns/event in a 1,000-string-event batch on the same machine; T0-07 below |
| Epoch cost | ≤ 10% | Measured (VM: 0–9%; earlier research, not remeasured in T0-07) |
| Deadline-to-interruption overrun | ≤ 1 ms p99 on a quiet desktop (measured, not a CI assertion); CI functional bound ≤ 10 ms — owner decision, open question 11 | To be measured in T0-13 with ≥ 1,000 interruptions. Earlier data: final quiet repetitions ≤ 0.6894 ms; earlier corrected-timer run during compilation reached 1.5101 ms; Linux VM 4 vCPU with all cores busy: 1.66–5.80 ms (5/5 runs failed against the old hard 1 ms assertion) |
| Cold start of a mod with cache | ≤ 50 ms | Hypothesis |
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

Earlier "VM" figures — 2 vCPU Xeon 2.1 GHz, noise ±30%: `docs/research/optimization-tests.md`.
