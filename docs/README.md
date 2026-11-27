# Documentation

The main document is [`PRD.md`](PRD.md): why, goals, metrics, risks, plan. The remaining folders are development specifications: what exactly we build, with which invariants, and how we verify it.

Reading order for a new developer or agent:

1. [`../AGENTS.md`](../AGENTS.md) — rules for working in the repository.
2. [`architecture/overview.md`](architecture/overview.md) — map of crates, threads and data.
3. [`process/roadmap.md`](process/roadmap.md) — current stage and exit criteria.
4. The specification of the area you work on (table below).
5. [`adr/`](adr/) — why the key decisions were made. Do not revisit them without a new ADR.

## Document map

| Area | Document | Crate |
| --- | --- | --- |
| Tick, phases, scheduler, budgets | [architecture/engine-core.md](architecture/engine-core.md) | `engine-core` |
| Sections, palette, IDs, saving | [architecture/world-storage.md](architecture/world-storage.md) | `world` |
| Meshing and rendering | [architecture/meshing-and-rendering.md](architecture/meshing-and-rendering.md) | `mesh`, `render` |
| Render graph and shader packs v2 (proposed, ADR-0015) | [architecture/render-graph.md](architecture/render-graph.md) | `render` |
| LOD | [architecture/lod.md](architecture/lod.md) | `lod` |
| Lighting | [architecture/lighting.md](architecture/lighting.md) | `light` |
| World generation | [architecture/worldgen.md](architecture/worldgen.md) | `worldgen` |
| Simulation, physics, mob AI | [architecture/simulation-and-ai.md](architecture/simulation-and-ai.md) | `sim` |
| Network protocol | [architecture/networking-protocol.md](architecture/networking-protocol.md) | `protocol`, `net` |
| Server regions | [architecture/server-regions.md](architecture/server-regions.md) | `sim`, `server` |
| Moving structures (grids, planned) | [architecture/moving-structures.md](architecture/moving-structures.md) | `sim`, `server`, `render` |
| Dimensions and space (planned) | [architecture/dimensions.md](architecture/dimensions.md) | `server`, `worldgen`, `protocol` |
| Sharding | [architecture/sharding.md](architecture/sharding.md) | `server` (+ future `gateway`) |
| In-game launcher menu and updater | [architecture/client-menu-and-updater.md](architecture/client-menu-and-updater.md) | `client`, `updater` |
| Login and assets | [architecture/auth-and-assets.md](architecture/auth-and-assets.md) | `auth`, `assets` |
| Mods: overview | [modding/overview.md](modding/overview.md) | `modhost`, `mod-sdk` |
| Builds and base-game modules | [modding/builds.md](modding/builds.md) | `world::modset`, `server`, `client` |
| Movement profiles and abilities | [modding/movement-profiles.md](modding/movement-profiles.md) | `sim` |
| Mod manifest | [modding/mod-manifest.md](modding/mod-manifest.md) | `modhost` |
| Datapacks | [modding/datapacks.md](modding/datapacks.md) | `modhost`, `world` |
| WASM API | [modding/wasm-api.md](modding/wasm-api.md), [`../wit/engine-api.wit`](../wit/engine-api.wit) | `modhost`, `mod-sdk` |
| How to write mods | [modding/writing-mods.md](modding/writing-mods.md) | `mod-sdk`, `engine-cli` |
| Rules for fast mods | [modding/performance-rules.md](modding/performance-rules.md) | — |
| Stages and exit criteria | [process/roadmap.md](process/roadmap.md) | — |
| Performance budgets | [process/performance-budgets.md](process/performance-budgets.md) | — |
| Code style | [process/coding-standards.md](process/coding-standards.md) | — |
| Tests and benchmarks | [process/testing-and-benchmarks.md](process/testing-and-benchmarks.md) | — |
| Legal restrictions | [legal.md](legal.md) | — |
| Open questions | [open-questions.md](open-questions.md) | — |
| Terms | [glossary.md](glossary.md) | — |
| Research and benchmarks of the pre-code stage | [research/](research/) | `../research/bench` |

## Source-of-truth rule

- Decisions live in ADRs. The PRD and specifications refer to ADRs rather than duplicating the reasoning.
- Numbers (budgets, targets) live in [`process/performance-budgets.md`](process/performance-budgets.md). If a number changes, change it there and in the PRD identically.
- If a specification contradicts the code, one of them is wrong: fix either the code or the specification in the same PR.
