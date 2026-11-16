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
| GPU geometry time | ≤ 8 ms per frame on iGPU | Hypothesis |
| CPU command recording | ≤ 1 ms per frame | Hypothesis |
| `fast_mesh` of a 32³ mesh chunk | ≤ 0.2 ms p50 on a 4-core desktop | Measured (VM: ≈ 42 µs on the T0-04 terrain corpus; `mesh` bench `mesh_chunk/fast_mesh`) |
| Gather + masks of a 32³ mesh chunk | ≤ 100 µs p50 | Measured (VM: ≈ 60 µs on the T0-04 corpus; `mesh` bench `mesh_chunk/gather_masks`, [details](../../task-reports/t0-04-mesh-chunk-32.md)) |
| Mask update per edit | ≤ 20 ns | Measured (VM: ≈ 2.6 ns; `mesh` bench `mesh_chunk/mask_update`) |
| Render batch copy after an edit | ≤ 50 µs | Hypothesis |
| Lighting: add / remove per edit | ≤ 100 / ≤ 200 µs p95 | Hypothesis |
| `get` in a section | ≈ 3.8 ns | Measured (VM: ≈ 3.4 ns paletted, 0.5 ns single, 0.7 ns direct; `world` bench `section_get`) |
| `set` in a section, any bit width | ≤ 10 ns | Measured (VM: ≈ 6 ns typical; 6–8-bit readings of 13–15 ns are 4K-aliasing stalls on this VM, see [T0-03 report](../../task-reports/t0-03-world-palette-section.md); bare-metal confirmation pending) |
| `unpack_into` per section | ≤ 5 µs | Measured (VM: ≈ 2.9–4.1 µs paletted, < 0.1 µs single/direct; `world` bench `section_unpack_into`) |

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
| Batched hook dispatch | ≤ 1 µs | Measured (VM: ≈ 0.4 µs, component) |
| Event in a batch | ≤ 5 ns | Measured (VM: ≈ 3.6 ns/element of `list<u32>`) |
| Epoch cost | ≤ 10% | Measured (VM: 0–9%) |
| Cold start of a mod with cache | ≤ 50 ms | Hypothesis |
| Default mod | 0.5 ms per tick per region, 32 MB | Target |

"VM" figures — 2 vCPU Xeon 2.1 GHz, noise ±30%: `docs/research/optimization-tests.md`.
