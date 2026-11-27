# Tasks

The backlog is split by stage of the roadmap ([docs/process/roadmap.md](../docs/process/roadmap.md)). Each task is a separate file with its goal, links to specifications, deliverables and acceptance criteria.

## How to work with tasks

1. Take a task with status `todo` whose dependencies are all `done`.
2. Change the status to `in-progress` in the task file and in the table below, in the same PR as the code (or in a separate first commit on the branch).
3. A task is closed when all acceptance criteria are checked and CI is green. Status → `done`; the file links to the PR.
4. Found new work? Add a file `T<stage>-<NN>-<slug>.md` using the same template; do not expand someone else's task.
5. If a task contradicts a specification or ADR, stop and record the question in [docs/open-questions.md](../docs/open-questions.md); do not resolve it silently.

## Stage 0

| Task | Name | Depends on | Owner | Status |
| --- | --- | --- | --- | --- |
| [T0-01](stage-0/T0-01-ci-and-toolchain.md) | CI green and reproducible | — | human + agent | done |
| [T0-02](stage-0/T0-02-core-job-scheduler.md) | Job scheduler with priorities and cancellation | T0-01 | agent | done |
| [T0-03](stage-0/T0-03-world-palette-section.md) | 16³ section with palette | T0-01 | agent | done |
| [T0-04](stage-0/T0-04-mesh-chunk-32.md) | Gather, incremental masks and fast_mesh for the 32³ mesh chunk | T0-03 | agent | done |
| [T0-05](stage-0/T0-05-mesh-render-batches.md) | Render batches of 2×2×2 mesh chunks | T0-04 | agent | done |
| [T0-06](stage-0/T0-06-render-capability-probe.md) | GPU capability probe and vertex pulling prototype | T0-04 | agent writes, human runs on hardware | done |
| [T0-07](stage-0/T0-07-wit-and-modhost-spike.md) | WIT 0.1 and mod host spike | T0-01 | agent | done |
| [T0-08](stage-0/T0-08-light-incremental-bfs.md) | Incremental light add/remove | T0-03 | agent | done |
| [T0-09](stage-0/T0-09-sim-region-prototype.md) | Server regions prototype | T0-02 | agent | done |
| [T0-10](stage-0/T0-10-human-legal-and-azure.md) | Lawyer and Minecraft API access | — | human (not an agent) | in-progress (waiting for answers) |
| [T0-11](stage-0/T0-11-net-quic-spike.md) | QUIC transport spike | T0-01 | agent | done |
| [T0-12](stage-0/T0-12-worldgen-test-corpus.md) | Deterministic test terrain | T0-03 | agent | done |
| [T0-13](stage-0/T0-13-modhost-interruption-latency.md) | Mod interruption latency: split criterion, diagnose overrun | T0-07 (open question 11 answered) | agent | done |
| [T0-15](stage-0/T0-15-net-flaky-test.md) | Stabilize the flaky QUIC spike test | T0-11 | agent (cloud) | done |
| [T0-16](stage-0/T0-16-bare-metal-benchmarks.md) | Bare-metal confirmation of VM-only benchmarks | T0-03/04/05/08/12 | agent on the owner's laptop | done |
| [T0-17](stage-0/T0-17-stage-0-exit-and-stage-1-backlog.md) | Stage 0 exit review and stage 1 backlog | T0-15, T0-16 | agent prepares, owner approves | done |
| [T0-14](stage-0/T0-14-gpu-hardware-evidence.md) | GPU hardware evidence for T0-06 (Vega 10 + GTX 1660 Ti) | T0-06 implementation | agent + human runs on the laptop | done |

Recommended order for a single agent: T0-01 → T0-03 → T0-12 → T0-04 → T0-05 → T0-08 → T0-02 → T0-09 → T0-07 → T0-13 → T0-11 → T0-06 → T0-14.

### Stage 0 result (2026-11-21)

Stage 0 is closed by the owner's decision on the [exit review](../docs/research/stage-0-exit-2026-11-21.md): 14 budget rows met, 5 deviations kept as targets with follow-ups, 3 hypotheses moved to later stages. T0-10 stays open as an external dependency (lawyer, Microsoft); it does not block work — offline mode and own licensed assets are used meanwhile (open questions 4 and 5).

### Carry-over from stage 0 — where each item went

- Renderer backend/adapter selection with fallback, GL shutdown on AMD iGPU (T0-14) → **T1-05**.
- B vs C default on iGPU (open question 13), CPU recording statistic (open question 14) → **T1-11**.
- Bare-metal confirmation of VM-only numbers → done in T0-16.
- Section `get`/`set`, gather + masks misses on bare metal (T0-16) → **T1-04**.
- Client RAM hypothesis → **T1-15**.
- GTX 1050 targets unverified (open question 12) → noted in **T1-15**; stays open.
- **For stage 2:** bare-metal re-run of the T0-11 QUIC spike with a shaped link (bandwidth cap + jitter, `tc netem`) and BBR fairness on a shared bottleneck ([net-quic-spike](../docs/research/net-quic-spike-2026-11-19.md)); measure "Network CPU" and "Node RAM" (still Hypothesis).
- **For stage 3:** mod hook dispatch 2.27 µs vs ≤ 1 µs and 308 ns/event vs ≤ 5 ns (T0-07/T0-13 deviations) — optimization task before the stage 3 exit criterion "hook ≤ 1 µs with epoch"; disk compilation cache for mods (cold start measured only without a cache).

## Stage 1

Core and single-player world ([roadmap](../docs/process/roadmap.md), PRD §9). Exit: 60 FPS on the iGPU (Vega 10) at 12 chunks with 1% low ≥ 45, RAM ≤ 1.5 GB without LOD, 30 min of walking without spikes > 33 ms, start to menu ≤ 5 s.

| Task | Name | Depends on | Owner | Status |
| --- | --- | --- | --- | --- |
| [T1-01](stage-1/T1-01-world-columns-registry.md) | World columns, block registry and ID table | T0-03 | agent (cloud) | done |
| [T1-02](stage-1/T1-02-world-save-format.md) | On-disk world format: save and load | T1-01 | agent (cloud) | done |
| [T1-03](stage-1/T1-03-worldgen-pipeline.md) | Terrain generator pipeline | T1-01, T0-12 | agent (cloud) | done |
| [T1-04](stage-1/T1-04-section-mesh-hot-paths.md) | Optimize section get/set and gather + masks | T0-16 | agent (cloud) + laptop confirmation | done |
| [T1-05](stage-1/T1-05-render-backend-selection.md) | Renderer backend and adapter selection with fallback | T0-14 | agent (cloud) + laptop checks | done |
| [T1-06](stage-1/T1-06-profiling-and-ci-benchmarks.md) | Profiler zones, Tracy and benchmarks in CI | T0-01, T0-02 | agent (cloud) + Tracy screenshot on the laptop | done |
| [T1-07](stage-1/T1-07-assets-fallback-and-cache.md) | Fallback asset set, asset cache and texture arrays | T0-01 | agent (cloud) | done |
| [T1-08](stage-1/T1-08-offline-mode.md) | Offline mode for client and server | owner decision (legal.md rule 7) | agent (cloud) | done |
| [T1-09](stage-1/T1-09-light-in-world.md) | Lighting integrated with the world | T1-01, T1-03 | agent (cloud) | done |
| [T1-10](stage-1/T1-10-loading-queue.md) | Chunk loading queue with priorities | T1-01, T1-03 | agent (cloud) + laptop | done |
| [T1-11](stage-1/T1-11-renderer-terrain.md) | Terrain renderer: upload, culling, texture arrays, profiles | T1-05, T1-07 | agent (cloud) + laptop | done |
| [T1-12](stage-1/T1-12-player-physics-and-editing.md) | Player physics, digging and placing blocks | T1-01, T1-09, T1-10 | agent (cloud) + laptop | done |
| [T1-13](stage-1/T1-13-client-app-and-menu.md) | Client application, main menu and single-player | T1-07, T1-08, T1-10, T1-11 | agent (cloud) + laptop | done |
| [T1-14](stage-1/T1-14-local-asset-import.md) | Import assets from a local licensed installation | T1-07 | agent (cloud) + laptop | done |
| [T1-16](stage-1/T1-16-light-fast-world-path.md) | Faster world-path lighting (same look) | T1-09 | agent + laptop | done (follow-ups: deep-shaft pass, persistent-grids ADR — see Stage 1 follow-ups) |
| [T1-17](stage-1/T1-17-worldgen-modes.md) | World generation modes: normal and extended (Tectonic as reference) | T1-03, T1-13 | agent + owner review | todo (later) |
| [T1-18](stage-1/T1-18-gameplay-slots.md) | Gameplay slots: replaceable movement profile and generator (ADR-0013) | T1-12, T1-13 | agent (cloud) | todo |
| [T1-19](stage-1/T1-19-resource-and-shader-packs.md) | Resource, texture and shader packs (ADR-0014) | T1-11, T1-13 | agent (cloud) + laptop | in-progress (owner: try packs, shader-pack GPU cost) |
| [T1-15](stage-1/T1-15-stage-1-exit-harness.md) | Stage 1 exit measurement | T1-01 … T1-14 | agent + owner's laptop | in-progress (harness done; owner: laptop runs) |
| [T1-20](stage-1/T1-20-stage-1-exit-review.md) | Stage 1 exit review and stage 2 backlog | T1-15 | agent + owner | in-progress (stage 2 backlog done; review waits for T1-15) |

### Stage 1: order and parallel work

| Wave | Tasks that can run in parallel | Waits for |
| --- | --- | --- |
| 1 | T1-01, T1-04, T1-05, T1-06, T1-07, T1-08 | — |
| 2 | T1-02, T1-03, T1-11, T1-14 | T1-01 (T1-02, T1-03); T1-05 + T1-07 (T1-11); T1-07 (T1-14) |
| 3 | T1-09, T1-10 | T1-01 + T1-03 |
| 4 | T1-12, T1-13 | T1-09 + T1-10 (T1-12); T1-07, T1-08, T1-10, T1-11 (T1-13) |
| 5 | T1-15 | everything above |
| later | T1-16, T1-17 | after the main stage 1 tasks (owner decision 2026-11-22) |

Wave 1 tasks touch different crates (`world`, `mesh`, `render`, `engine-core`, `assets`, `auth`), so up to six agents can work at once; merge `main` before each PR and keep both sides' rows in shared files.

### Stage 1 follow-ups

- World map `get_block`/`set_block` are 2.8× / 2.4× the section access (VM, T1-01); the owner kept the 2× target (2026-11-22). Optimize when profiling shows map access in a hot path (physics, edits), e.g. a per-caller column cache or batched access APIs; re-measure on the laptop.
- Replaceable movement and world generation ("gameplay slots") — [ADR-0013](../docs/adr/0013-replaceable-movement-and-worldgen.md), accepted by the owner 2026-11-22 → [T1-18](stage-1/T1-18-gameplay-slots.md).
- T1-16 leftovers (the owner accepted both options, 2026-11-22): an ADR for persistent per-region light grids (≤ 100 µs per edit), and a cheaper pass for sky changes reaching below the snapshot (≈ 10 ms on the VM). Chained slabs are preferred because they are exact and have no visible delay; spreading over frames is the fallback.

## Stage 2: network and server

The owner decided on 2026-11-23 to start stage 2 while the T1-15 laptop runs are pending. The stage 1 exit review (T1-20) is written once the numbers arrive. Exit criterion ([roadmap](../docs/process/roadmap.md)): 200 bots on 8 cores, tick ≤ 25 ms p99, ≤ 50 kbit/s per player, measured region capacity.

| Task | Name | Depends on | Owner | Status |
| --- | --- | --- | --- | --- |
| [T2-01](stage-2/T2-01-protocol-messages.md) | Protocol crate: messages, versioning, handshake | T0-11, T1-08 | agent (cloud) | done |
| [T2-02](stage-2/T2-02-dedicated-server-world.md) | Dedicated server: world, join, chunk streaming | T2-01, T1-02, T1-03 | agent (cloud) | todo |
| [T2-03](stage-2/T2-03-movement-prediction-replication.md) | Player movement: prediction, reconciliation, entity replication | T2-02, T1-12 | agent + owner's feel review | todo |
| [T2-04](stage-2/T2-04-network-block-edits.md) | Block edits over the network | T2-02, T2-03 | agent (cloud) | todo |
| [T2-05](stage-2/T2-05-headless-bots.md) | Headless bot clients and the load harness | T2-03, T2-04 | agent (cloud) | todo |
| [T2-06](stage-2/T2-06-server-regions.md) | Regional multithreading on the real server | T2-02, T0-09 | agent (cloud) | todo |
| [T2-07](stage-2/T2-07-entity-sim-lod-pathfinding.md) | Entity simulation LOD and asynchronous pathfinding | T2-06 | agent (cloud) | todo |
| [T2-08](stage-2/T2-08-multiplayer-menu-and-embedded-server.md) | Multiplayer menu and single-player on an embedded server | T2-02, T2-04 | agent + owner's playtest | todo |
| [T2-09](stage-2/T2-09-stage-2-exit-measurement.md) | Stage 2 exit measurement | T2-01 … T2-08 | agent + owner's machine | todo |

### Stage 2: order and parallel work

| Wave | Tasks | Waits for |
| --- | --- | --- |
| 1 | T2-01 | — |
| 2 | T2-02 | T2-01 |
| 3 | T2-03, T2-06 | T2-02 |
| 4 | T2-04, T2-07 | T2-03 (T2-04); T2-06 (T2-07) |
| 5 | T2-05, T2-08 | T2-04 |
| 6 | T2-09 | everything above |

Stage 1 work that continues in parallel: T1-15 laptop runs, T1-17, T1-18, the T1-19 laptop check, the T1-16 follow-ups, and gameplay polish (block highlight, hold to dig, transparent pass, block models, biome tints).
