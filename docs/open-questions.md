# Open questions

Questions that cannot be decided without the product owner (or a lawyer). An agent does not decide them itself: it uses the default from the "For now" column and marks the place in the code `// OPEN-QUESTION: <number>`.

| No. | Question | For now | Who decides | When |
| --- | --- | --- | --- | --- |
| 1 | Is a web client (WebGL2) needed? | No. Path C (GL ES) is minimal, only for old desktop GPUs | owner | before the end of stage 1 |
| 2 | Are bit-exact server replays needed? | No. Repeatability only with the same region partitioning; fuel only in tests | owner | before stage 4 |
| 3 | Source of truth for entity ownership between shards: JetStream or PostgreSQL with a record version? | Not implemented until stage 6 | architect | stage 6 |
| 4 | Can the Minecraft token be passed to our login service? | Implement per the scheme in auth-and-assets.md, but do not launch publicly | lawyer | stage 0 |
| 5 | Mojang assets in a custom engine: download and conversion on the player's machine | Prepare the code, the game and CI run on fallback assets | lawyer | stage 0 |
| 6 | Capacity of one region (players at tick ≤ 25 ms) | Unknown | benchmark | stage 2 |
| 7 | Real compressed column size and server RAM for 500 players | Estimates from PRD §8 | benchmark on a real world | stage 1–2 |
| 8 | Re-copy a render batch entirely or 8 sub-ranges | Re-copy entirely — confirmed by T0-05 (VM: full re-copy ≈ 1.6 µs vs ≈ 0.2 µs sub-range, budget ≤ 50 µs; the ~1.4 µs saving does not justify 48 ranges with per-slot counters). Bare-metal confirmation pending | benchmark T0-05 | done (stage 0) |
| 9 | Project name and trademark | Working name of the repository | owner + lawyer | before the open beta |
| 10 | Hiding other players' builds in special modes (creative plots) | Not done in the shared world | owner | stage 4 |
| 11 | Mod interruption bound: hard "≤ 1 ms after the deadline" (T0-07) cannot be guaranteed by an OS thread under CPU load (Windows 1.51 ms during compilation; Linux VM 1.66–5.80 ms with all cores busy). Reformulate as: CI functional bound + latency target p99 ≤ 1 ms on a quiet desktop? | CI functional test bound ≤ 10 ms; latency target p99 ≤ 1 ms quiet, measured separately and reported, not asserted in CI. See [T0-13](../tasks/stage-0/T0-13-modhost-interruption-latency.md) | owner | before closing T0-07 |
| 12 | Target hardware for the T0-06 GPU evidence (no GTX 1050 available) | Decided by the owner 2026-11-19: iGPU = Radeon RX Vega 10, discrete = GTX 1660 Ti, both in one laptop (Ryzen 7 3750H). GTX 1050 targets stay unchanged and unverified; 1660 Ti numbers do not verify them. Work: [T0-14](../tasks/stage-0/T0-14-gpu-hardware-evidence.md) | owner | done (stage 0) |
| 13 | Default render path on iGPUs: B (vertex pulling) or C (per-instance)? T0-14: C is 25–32% faster on Radeon Vega 10 on DX12/Vulkan/GL (e.g. 1.65 vs 2.42 ms p50), equal on GTX 1660 Ti; synthetic unit quads, B non-indexed (3M vs 2M vertex invocations) | Keep `select_path` as specified (B when `VERTEX_STORAGE`). Re-measure in stage 1 on the real T0-04 terrain corpus with an indexed/4-vertex B variant before changing the default | owner + benchmark | stage 1 |
| 14 | Statistic for the "CPU command recording ≤ 1 ms per frame" budget: p95 or every frame? T0-14: p95 ≤ 0.83 ms everywhere, but single frames reach 1.32 ms (Vega 10, DX12) | p95; report max alongside | owner | stage 1 |
