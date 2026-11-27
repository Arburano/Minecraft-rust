# Roadmap

The full description of stages and risks is PRD §9–10. Here is the working view: what is included, the exit criterion, tasks.

**Current stage: 1 → 2** (stage 0 closed 2026-11-21, [exit review](../research/stage-0-exit-2026-11-21.md)). On 2026-11-23 the owner decided to start stage 2 ([`../../tasks/stage-2/`](../../tasks/stage-2/)) while the stage 1 exit runs (T1-15) are pending on the laptop. Stage 1 closes formally with the exit review (T1-20). Stage 1 tasks: [`../../tasks/stage-1/`](../../tasks/stage-1/).

| Stage | Duration (estimate) | Result | Exit criterion |
| --- | --- | --- | --- |
| 0. Research and prototypes | 2–3 months | benchmarks, renderer prototype, WIT 0.1, ADRs | all numbers from performance-budgets confirmed or adjusted; ADRs recorded |
| 1. Core and single-player world | 6 months | ECS core, sections, generation, lighting, rendering, physics, menu, assets | 60 FPS on iGPU at 12 chunks (1% low ≥ 45), RAM ≤ 1.5 GB without LOD (internal budget 1 GiB), 30 min without stutters > 33 ms, start to menu ≤ 5 s |
| 2. Network and server | 4–5 months | QUIC protocol, replication, prediction, regions, bots | 200 bots on 8 cores, tick ≤ 25 ms p99, ≤ 50 kbit/s per player; measured region capacity |
| 3. Mod API v1 and in-game launcher menu | 4 months | WASM host, SDK, engine-cli, Microsoft login, mod sync, updater, registry | test mods install automatically; hook ≤ 1 µs with epoch; a hung mod does not stop the server |
| 4. Gameplay as the `core` mod | 6 months | blocks, items, inventory, crafting, mobs with AI, biomes, zones, contraptions, action log; engine capabilities for the planned builds: dimensions, moving grids (ADR-0018, ADR-0019, accepted 2026-11-27) | closed alpha for 50 players; `core` without private API |
| 5. LOD and low-end hardware | 3–4 months | LOD rings, server LOD, auto-profile | 64 chunks at 60 FPS on GTX 1050; RAM ≤ 2 GB |
| 6. Shards | 6 months | gateway, coordinator, handoff, global services, own hardware and tuning | 5,000 bots on 10 nodes, ≈ 500 per node; 0 dupes in 10,000 automated tests |
| 7. Beta and open SDK | ongoing | documentation, second SDK language, anti-cheat, moderation | third-party mods without the team's help |

The sum of stages 0–6 is 31–34 months for a team of 3–5 engineers.

## What was added from the review of old ideas (PRD §14)

- Stage 1: a loading queue with priorities (direction of movement, height, speed), lazy asset loading, 2×2×2 render batches, start ≤ 5 s.
- Stage 2: entity simulation LOD, asynchronous pathfinding.
- Stage 4: game zones, an action log with rollback, hiding players (avatar, chat, voice).
- Stage 6: own hardware, pinning of heavy regions, OS tuning.

## Parallel track

While stages 0–3 are underway, the community can live on a Fabric + Folia server where mod ideas are tested (PRD §9). This is a separate repository.
