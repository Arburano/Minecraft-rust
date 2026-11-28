# Architecture overview

The engine is a "voxel OS": the core (world, rendering, networking, scheduling, mod host) knows nothing about specific blocks or mobs. All gameplay is mods on the public API: the base game is split into modules (`core`, `movement`, one per world, `mobs`, …) and a build lists the modules a game runs with (ADR-0017, `modding/builds.md`). Client and server share one codebase; single-player runs an embedded server in the same process.

## Crates

```text
                 ┌────────────┐
                 │engine-core │  tick, phases, budgets, task scheduler
                 └─────┬──────┘
          ┌────────────┼───────────────┬──────────────┐
          ▼            ▼               ▼              ▼
       ┌─────┐      ┌─────┐        ┌────────┐     ┌────────┐
       │world│◄─────│ sim │        │modhost │     │protocol│
       └──┬──┘      └──┬──┘        └───┬────┘     └───┬────┘
   ┌──────┼──────┬─────┼───────┐       │              ▼
   ▼      ▼      ▼     │       ▼       │           ┌─────┐
 mesh   light  worldgen│      lod      │           │ net │
   │                   │                │           └──┬──┘
   ▼                   │                │              │
 render                │                │              │
   │                   │                │              │
   └──────►  client ◄──┴────────────────┼──────────────┤
             (bin)                      │              │
               ▲  auth, assets          ▼              │
               │                     server ◄──────────┘
            updater (bin)             (bin)

 mod-sdk  — library for mod authors (built to wasm32-wasip2)
 engine-cli — new / dev / test / bench / publish
```

| Crate | Type | Depends on | Responsibility |
| --- | --- | --- | --- |
| `engine-core` | lib | — | `Tick`, `TickPhase`, `Budget`, prioritized task scheduler, profiler zones |
| `world` | lib | engine-core | coordinates, `BlockStateId`, palette sections, columns, registries, ID table |
| `mesh` | lib | world | gather, masks, `fast_mesh` for 32³, 64³ render batch assembly |
| `light` | lib | world | incremental add/remove BFS lighting on section snapshots |
| `worldgen` | lib | world | generation pipeline, height and biome map |
| `lod` | lib | world | LOD sections, dirty nodes, disk cache |
| `render` | lib | mesh | wgpu: capability probe, vertex pulling, culling, texture arrays, render graph |
| `protocol` | lib | world | versioned client↔server↔gateway messages |
| `netcode` | lib | protocol, sim | what both ends of a connection compute the same way: intents ↔ physics input, state hash, corrections, entity deltas (T2-11) |
| `net` | lib | protocol | QUIC (quinn): prioritized streams, datagrams |
| `sim` | lib | engine-core, world | regions, region ECS world, player physics, AI, prediction |
| `modhost` | lib | engine-core, world | wasmtime, mod loading, batched hooks, epoch, permissions |
| `mod-sdk` | lib | — | API for mod authors on top of generated WIT bindings |
| `assets` | lib | — | Mojang asset download, conversion, fallback assets, cache |
| `auth` | lib | — | Microsoft token chain, session JWT client, secret storage |
| `client` | bin | many (`server` only with the `singleplayer` feature) | window, in-game launcher menu, render loop; with `singleplayer` (default) the embedded server for single-player |
| `server` | bin + lib | many | dedicated server; the library is also the client's embedded server |
| `updater` | bin | — | binary update and game launch |
| `engine-cli` | bin | — | tools for mods and development |

Dependency rule: arrows only point down. `world` knows nothing about rendering, `sim` knows nothing about the network (the network feeds intents into `PreTick` and picks up deltas in `PostTick`). A new link between crates that violates the direction requires an ADR.

Optional engine features (planned): a build starts only the **capabilities** its mods require — `dimensions`, `moving_grids`, `space`, `zones`, `instances`, `progression` ([ADR-0018](../adr/0018-engine-capabilities-per-build.md), accepted 2026-11-27). Moving structures would live in `sim` (grids, Rapier behind the `moving_grids` feature), `server` (regions, network) and `render` (per-object transforms): [moving-structures.md](moving-structures.md), [dimensions.md](dimensions.md).

## Threads

Client:

| Thread | Work | Rate |
| --- | --- | --- |
| Main | winit, input, menu UI | frame |
| Simulation | client ECS schedule: prediction, animations, client mods | 20 Hz + interpolation |
| Render | state snapshot → GPU commands | frame, lags by 1 frame |
| Network | tokio + quinn | async |
| Task pool (N−2) | gather/meshing, lighting, decoding, LOD, pathfinding | priority queue |
| IO/audio | assets, cache, sound | async |

Server: network (several threads), chunk IO, generation, region tick pool (≈ 80% of cores), task pool. Details in [server-regions.md](server-regions.md).

## Block data flow (from change to pixel)

1. The player presses "break" → the client predicts the change locally and sends an intent.
2. The server accepts the intent in `PreTick`; in `Simulate` the region applies the change and emits a `block-broken` event.
3. Mods receive a batch of events; their changes are applied atomically after the call.
4. `PostTick`: the section delta goes to clients in the visibility zone; lighting is queued for an incremental update; LOD ancestors are marked dirty.
5. The client applies the delta (or confirms the prediction), updates masks, and queues the mesh chunk with high priority.
6. The task pool re-meshes 32³, copies quads into the 64³ render batch, and the render thread uploads them to the GPU buffer.

## Where things live in the repository

```text
crates/      engine code (workspace)
wit/         mod contract (WIT)
mods/        base gameplay as modules (core, movement, overworld, …)
builds/      builds: lists of mods (ADR-0017)
docs/        PRD, specifications, ADRs, process, research
tasks/       task backlog by stage
research/    benchmarks that justified decisions (separate workspace)
```
