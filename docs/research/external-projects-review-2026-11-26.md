# External projects review: what to borrow (2026-11-26)

Source: an architecture analysis the owner ran in an external chat on 2026-11-26 (two videos plus a look at this repository's root and README). That analysis assumed the project was at stage 0. This note adapts it to the real state of the repository (stage 1 → 2, ADR-0001…0014 accepted) and records where each idea went.

**Status of the facts below.** Claims about other projects (licenses, benchmark numbers from the videos, the state of their repositories) come from that analysis and were **not verified** by us. Before copying any code, open the project's LICENSE file yourself (`docs/legal.md`, "Third-party content"). The video numbers are third-party measurements on unknown hardware (AGENTS.md rule 7): they illustrate failure modes, they are not our budgets.

## 1. Lessons from the "Rust Minecraft" comparison video

The video compares Horizon (a Rust rewrite of the vanilla game) with mc.rs (a Bevy ECS engine) under load.

| Observation (third-party, unverified) | Lesson | Already covered here |
| --- | --- | --- |
| Horizon: fast start (no JVM warm-up), no GC pauses, high client FPS, built-in LOD | Native start and no GC are what Rust gives for free; LOD belongs in the engine | Start ≤ 5 s (performance-budgets), LOD rings (ADR-0011, `lod.md`) |
| Horizon kept the single-threaded vanilla tick: ≈ 30,000 pigs → TPS ≈ 2.2 | Porting the tick model ports its ceiling | ADR-0008 (regions), engine-core.md (systems in parallel by declared access) |
| 50 players on elytra over ungenerated land: TPS held, but players flew into empty space and fell below the world | Generation must apply **backpressure to movement**, not silently drop work | Client: unloaded cells collide (T1-12). **Server: gap → [T2-10](../../tasks/stage-2/T2-10-server-overload-protection.md)** |
| mc.rs: same 30,000 pigs at 20 TPS, parallelism by data access graph | Two levels of parallelism: between regions and inside a region | ADR-0008 + engine-core.md; **a crowded region still needs both levels → server-regions.md updated** |

## 2. What to take from which project

| Project | License (as reported, verify) | What we take | How |
| --- | --- | --- | --- |
| Horizon | LGPL-3.0 (was MIT); public repository reportedly removed in November 2026; reported dispute over code copied from Iris | **Nothing in code.** Three ideas: local asset preparation from the player's own installation, engine-level LOD, a native WGSL pipeline | All three already exist: T1-14, ADR-0011, T1-11/T1-19 |
| mc.rs (`andreypfau/mcrs`) | MIT / Apache-2.0 | Closest stack (bevy_ecs + wgpu). Ideas: no `Arc<Mutex>` on the hot path, inactive entities cost 0 CPU, GPU-driven culling (Hi-Z, section/entity culling on the GPU) | Ideas → T2-07 (sim LOD) and path A (ADR-0003, stage 2+ after measurements). Code reuse is legally possible with attribution in `THIRD_PARTY.md`, after checking LICENSE |
| Folia (PaperMC) | GPL-3.0 — **ideas only** | Region / entity / global schedulers; dynamic merge and split; ownership checks so code cannot touch another region | Already the model of ADR-0008/0009 and `server-regions.md` |
| Paper / Purpur | GPL-3.0 — **ideas only** | Entity activation range; limits and merging for falling blocks and item entities (lag machines); generation backpressure at the edge of the generated world | Activation range = simulation LOD (T2-07). Limits and backpressure → **T2-10** |
| FerrumC | MIT | Multithreaded chunk streaming, lock-free structures, low RAM | Read before T2-02 (column streaming); code reuse with attribution |
| Pumpkin | GPL-3.0 — **ideas only** | Start in milliseconds, ≈ 100 MB idle RAM, multi-language plugins | Plugins are covered better by WASM components (ADR-0006); idle RAM → a server budget row to measure in T2-09 |
| Iris / OptiFine | LGPL-3.0 / proprietary — **no code, no shader files** | The idea that a shader pack controls the whole frame graph (shadow, G-buffer, deferred, translucent, composite) | **[ADR-0015](../adr/0015-programmable-render-graph.md) (accepted 2026-11-27)**. Compatibility with their pack format only as a separate adapter written from the public format description (open question 17) |

## 3. Shader packs: "change everything, as Iris does"

Today (ADR-0014, hook version 1) a pack changes shading, light curve, fog and post passes. The owner wants packs to be able to change the whole frame: geometry deformation (waving plants, curved world), shadows, materials (PBR), deferred lighting, water and translucency, composite chains. That revisits an accepted decision, so it is written as [ADR-0015](../adr/0015-programmable-render-graph.md) with status Proposed (accepted by the owner on 2026-11-27) and a specification [`render-graph.md`](../architecture/render-graph.md). The main idea that keeps our guarantees: the pack controls **passes and shaders**, the engine keeps **quad decoding, culling and draw submission**, so the paths B/Bi/C and the CPU budget still hold.

## 4. A non-cubic world in the future (lessons from the second video)

The second video (the developer of the game "Arrival") reports:

- **Micro-voxels for smooth terrain failed visually:** splitting a 1 m block into 4,096 cubes of 6 cm barely cost performance but looked like striped carpet or saw teeth on diagonals. Removed from terrain.
- **What worked:** non-cubic flora (procedural trunks and branches, individual leaves via mesh shaders with a lighter fallback, impostors far away); layered water waves with refraction and caustics; smooth terrain via Surface Nets / Dual Contouring over a density field rather than micro-voxels; one consistent art style (16×16 pixel art fights photoreal light); contact shadows; correct mip-mapped textures; and checking the result visually (RenderDoc, a reference room with a "white furnace" and a material stand).
- **For agent-driven development:** code can be mathematically correct with green tests and still look like grey plastic on screen.

What we prepare now so the engine does not need a rewrite later: [ADR-0016](../adr/0016-geometry-agnostic-meshing.md) (accepted 2026-11-27) — a `ChunkMesher` boundary, render buckets by geometry type, physics colliders separate from visual meshes, and a reserved place for a density layer in the save format. The visual check is a task of its own: [T1-21](../../tasks/stage-1/T1-21-render-reference-scene.md).

## 5. Where each item went

| Item | Where |
| --- | --- |
| Two-level parallelism inside a crowded region | `server-regions.md` ("What parallelizes poorly") |
| Generation backpressure, falling-block and item limits | T2-10, `server-regions.md` |
| Programmable render graph for shader packs | ADR-0015 (accepted), `render-graph.md`, T1-22 |
| Iris pack compatibility adapter | open question 17 |
| Non-cubic terrain readiness | ADR-0016 (accepted), T1-23, open question 18 |
| Reference scene and screenshot tests | T1-21 |
| Licenses of donor projects | `docs/legal.md` ("Third-party content") |
