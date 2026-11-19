# PRD: Rust voxel engine with native mods

Nov 15, 2026 · @hi

## 1. Overview

We are building our own voxel engine in Rust, not a port of Minecraft to another language. The engine is compatible with Minecraft assets (textures, sounds, models) but has its own architecture: ECS, multithreading by default, native mods, and a horizontally scalable server. The "Minecraft gameplay" itself is just the first mod on our API.

### Vision

A voxel game that can be fully reworked through mods, holds a steady 60 FPS on a weak PC, and supports thousands of players in one world.

### Targets

- **Performance on weak hardware.** 60 FPS on integrated graphics or a GTX 1050-class GPU at a 12-chunk render distance; another 64+ chunks via LOD.
- **Mods are first-class citizens.** All game content (blocks, items, mobs, crafting, world generation) is described through the same API as third-party mods.
- **Server scale.** 500+ players per node (16+ cores), 5,000+ per sharded world.
- **One-click launcher.** Login via Microsoft, asset download from official servers, automatic installation of the mods the server requires.

### Non-goals

- Compatibility with the vanilla Minecraft server protocol and with Java mods (Fabric/Forge). This is a separate world, and dragging it along means dragging along old architectural decisions.
- Copying code or logic from the decompiled Mojang client. Mechanics are implemented from scratch with our own design.
- Bedrock support.

### Success metrics

| Metric | Target |
| --- | --- |
| FPS on iGPU / GTX 1050, 12 chunks | ≥ 60 stable, 1% low ≥ 45 |
| Client RAM, 12 chunks + LOD | ≤ 2 GB |
| Meshing one 32³ mesh chunk (CPU, fast\_mesh, 1 thread) | ≤ 0.2 ms (desktop CPU, masks incremental; not confirmed — on a VM 62³ took 0.27 ms) |
| Server tick at 500 players per node | ≤ 25 ms (headroom up to 50 ms) |
| Time to join a server with a warm asset cache (until the character is controllable) | ≤ 5 s |
| WASM mod overhead per hook call | ≤ 1 µs |

The T0-07 prototype exceeds the hook and typed-event performance targets; the targets above are unchanged. Its measurements, documented deviations and unresolved interruption jitter are recorded in [performance budgets](process/performance-budgets.md#t0-07-measurements-2026-11-19).

## 2. Key decisions and stack

Main decision: the engine is an empty "voxel construction kit", and all gameplay comes from mods. This way the engine's optimizations automatically work for all mods, and the mod API is tested on our own content every day.

### Principles

1. **Data separate from logic (ECS).** No deep class hierarchies like Entity → Mob → Zombie, as in Minecraft. An entity is a set of components, behavior is systems.
2. **Multithreading through ownership, not locks.** A world region belongs to one thread during a tick; between regions — messages.
3. **Zero allocations in the hot loop.** Pools, arenas, buffer reuse — no GC and no frame spikes.
4. **The GPU does what it does best.** Culling and draw call generation — in compute shaders.
5. **Measurement from day one.** Profiling and benchmarks in CI; a speed regression breaks the build.
6. **The server is authoritative.** The client predicts movement and actions, the server confirms.
7. **One codebase for client and server.** Single player = an embedded server in the same process.

### Stack

| Layer | Technology | Why |
| --- | --- | --- |
| ECS and system scheduling | `bevy_ecs` (without the rest of Bevy) | Automatically runs systems in parallel based on declared access; Valence and Azalea are already built on it |
| Graphics | `wgpu` (Vulkan / DX12 / Metal, GL fallback) | Cross-platform, compute shaders, indirect draw |
| Window, input | `winit`, `gilrs` (gamepads) | Ecosystem standard |
| Task pool | `rayon` + custom job scheduler with priorities | Meshes, lighting, world generation |
| Network | QUIC (`quinn`) + custom protocol | Multiple streams without head-of-line blocking, unreliable datagrams for movement |
| Serialization | `bitcode` / `rkyv`, `zstd` compression | Fast and compact; rkyv gives zero-copy reads from disk |
| Mods (logic) | `wasmtime` + Component Model (WIT) | Sandbox, any language, hot reload, memory and time limits |
| Mods (content) | TOML/JSON datapacks + assets | Blocks, items, recipes without code |
| World storage | Region files with zstd + LMDB/`redb` for the index | Fast random access |
| Player and shard data | PostgreSQL + Redis/NATS | Accounts and event bus between nodes |
| Launcher | In-game menu (egui on wgpu) + separate updater in Rust | Small binary, code shared with the engine |
| Profiling | `tracing` + Tracy, `criterion` for benchmarks | Per-frame and per-tick analysis |

**Why not full Bevy.** Bevy's renderer is general-purpose and not optimized for millions of voxels, and its API breaks every 3–4 months. We take only `bevy_ecs` and pin the version; we write our own renderer on wgpu.

**Why a custom protocol rather than the Minecraft protocol.** The Minecraft protocol changes with every version, runs over TCP, and knows nothing about our mods, LOD, or shards. Valence and Azalea are useful as a code reference, but not as a foundation.

## 3. Engine core: threads, tick, memory

The core splits work by load type: simulation — by regions, heavy computation — in the task pool, rendering — in a separate thread one frame behind.

### Client thread map

| Thread | What it does | Frequency |
| --- | --- | --- |
| Main (window) | winit events, input, dispatching player intents | every frame |
| Simulation | ECS schedule: movement prediction, physics, animations, mods | 20 ticks/s + interpolation |
| Render | State snapshot → GPU commands; lags by 1 frame | up to the monitor refresh rate |
| Network (tokio) | QUIC, compression, packet deserialization | async |
| Task pool (N−2 cores) | Meshes, lighting, chunk decoding, LOD, pathfinding | by priority queue |
| Audio / IO | Sound, reading assets and cache | async |

### Tick and determinism

- Fixed simulation step (20 Hz by default, configurable); rendering interpolates between the last two states. Interpolation adds up to 50 ms of latency, so camera rotation is taken from the latest input at frame rate rather than interpolated.
- The tick is split into phases: `PreTick` (network, input) → `Simulate` (parallel by region) → `Exchange` (messages between regions) → `PostTick` (saving, sending deltas). A message sent in Simulate of tick N is processed by the receiver in Simulate of tick N+1; the reply returns no earlier than N+2 (100 ms at 20 Hz).
- Within a region, update order is stable (sorted by ID), so redstone-like mechanics are repeatable. Messages between regions in Exchange are also sorted (tick, sender position, counter) rather than taken in queue arrival order. We guarantee repeatability only for the same input and the same region partitioning: merges and splits depend on IO timing, so we do not promise bit-for-bit determinism between server runs.
- Time budget: systems and mods have a per-tick limit; deferred work (plant growth, random ticks) is spread over several ticks.

### Regions as the unit of parallelism

We take the idea from Folia and build it into the engine from day one. In Folia, neighboring loaded chunks are merged into independent regions, each with its own tick loop, and the loops run in parallel in a thread pool ([Folia](https://github.com/PaperMC/Folia), [region logic](https://docs.papermc.io/folia/reference/region-logic)). Folia's weak point is that almost no existing plugin is compatible without a rewrite. We have no such debt: the mod API by design gives access only to its own region.

- Region = a dynamic group of sections containing players or active entities. Regions merge when players come closer and split when they move apart. As in Folia, a merge with a region that is currently ticking is deferred until the end of its tick, and splits are checked at the end of the tick. Invariant: around occupied sections the region keeps a buffer of empty sections (merge radius ≥ 1 section), so two regions never directly touch.
- Region code neither reads nor writes other regions' data. Thanks to the buffer, local interactions (a neighboring block, light up to 15 blocks, a mob nearby) do not leave the region. Messages into a lock-free queue are needed only for distant actions (teleport, a projectile beyond the buffer, global services); they are delivered in the `Exchange` phase.
- Global state (time of day, weather, leaderboard) lives in a separate "global region" and is updated via commands. Regions and mods read its snapshot from the previous tick.

### Memory

- **16³ sections with a palette.** A section stores a list of unique block states and packed indices (1–8 bits per block; above 256 states in a section — a direct u16). A section with one block type (air, stone) takes a few bytes. Light (sky + block, 4 bits each) — up to 4 KB more per non-uniform section. Test for 12 chunks (625 columns × 24 sections = 15,000): blocks \~17 MiB with a palette (117 MiB without) + light \~59 MiB uncompressed, so chunk data is a small fraction of the 2 GB budget. A write into an 8-bit section with a linear palette lookup costs \~67 ns versus 6–7 ns, so large palettes have a reverse index (hash). Estimate for 64 chunks without LOD — 0.9–1.4 GB. Therefore beyond 12–16 chunks we do not keep full data on the client, only LOD.
- **Components in SoA format** (ECS archetypes): the physics system walks a contiguous array, far fewer cache misses (random access by Entity does not eliminate them).
- **Pools and arenas** for temporary mesh, packet, and task buffers; the arena is reset once per frame/tick.
- **Memory budgets** for chunks, meshes, textures, and each mod. On overrun the engine unloads the farthest (LRU) or lowers LOD.
- **Static data via mmap** (`rkyv`): block registries, models, atlases are read without deserialization.
- **Quality profiles** Low / Medium / High / Ultra, chosen by a benchmark on first launch.

## 4. World, rendering, LOD

The biggest win on weak PCs comes from three things: fast meshing, GPU-driven rendering, and LOD. All of this must be in the engine from the first version, not added by mods like Sodium or Distant Horizons in Minecraft.

### Meshing

- **Binary greedy meshing.** Chunk faces are computed with bit masks, adjacent identical faces merge into one rectangle. The Rust crate `binary-greedy-meshing` on a 62³ chunk gives 65 µs (opaque only) and 90 µs (with transparent) on a Ryzen 5 5500. This is the fast\_mesh method, which requires maintaining opacity masks on every block change; plain mesh is 300–340 µs on the same machine, and the version 0.6.4 README gives \~100 and \~700 µs. Our test (0.6.4, 62³, 8% transparent, slow 2.1 GHz VM): fast\_mesh 0.27 ms, mesh 1.1 ms, \~5,000 quads (39 KB) per chunk; building masks from scratch — another 0.35 ms, so masks must be updated incrementally ([source](https://docs.rs/crate/binary-greedy-meshing/0.4.2)).
- **Mesh chunk size. 62³ (64³ with padding)** does not divide into 16³ sections, so mesh and section boundaries would not coincide. We take a 32³ mesh chunk (2×2×2 sections, 34³ with padding): version 0.6 of the crate has a size parameter (up to 62), the decision is made, and rendering draws in batches of 2×2×2 mesh chunks (see section 14). A row of voxels still fits in a `u64`. For simulation we keep 16³ sections, and for rendering we group them as whole sections.
- **Quad = 8 bytes.** In the crate a u64 holds position (3×6 bits), size (2×6 bits), and block type; the face direction is given by one of 6 lists, and there is no light there. The free bits are enough for a 16-bit type and flat per-face light, but not for smooth light and AO at 4 corners: the shader reads those from the section's 3D light texture, otherwise greedy merging breaks quads on every light change. In total this is one `u64`; vertices are reconstructed by the vertex shader (vertex pulling). Compared to classic float vertices (4 × 24–32 B + indices = 120–152 B per quad) this is 12–19 times less VRAM (calculation, not measurement).
- Non-cube models (stairs, fences) — a separate path with precomputed model templates.
- Remeshing on a block change — only the affected 32³ mesh chunk and its neighbor if the block is on the border, with higher priority than new chunks.

### GPU-driven rendering

1. All meshes live in a few large GPU buffers with a custom allocator (suballocation), not one buffer per chunk. Default wgpu limits: one storage binding ≤ 128 MiB, buffer ≤ 256 MiB; we request higher limits only if the adapter provides them.
2. A compute shader does frustum culling, discards faces pointing away from the camera, and does occlusion culling against the previous frame's Hi-Z buffer. What was culled based on the previous frame is checked a second time against the current depth, otherwise holes appear on a sharp camera turn (Aokana does it this way).
3. The result is a `multi_draw_indirect` command buffer: the whole world in 1–3 draw calls. This holds only with MULTI\_DRAW\_INDIRECT\_COUNT (native only: DX12 and Vulkan 1.2+). Without it wgpu can emulate multi\_draw\_indirect with a series of draw\_indirect calls, and the command count is fixed: culled chunks are written with instance\_count = 0. INDIRECT\_FIRST\_INSTANCE is absent on GL ES/WebGL, so the chunk index is passed differently there.
4. For GPUs without compute or indirect (GL ES, WebGL2: 0 storage buffers and 0 compute there) — CPU culling and regular draw calls. We feed the same 8-byte quad as a per-instance vertex attribute (Uint32x2) instead of a storage buffer, so the data format does not change.
5. Textures — texture arrays instead of an atlas: no bleeding on mipmaps. The default wgpu limit is 256 layers per array, and Minecraft has more block textures than that, so several arrays (one per texture size) or a higher limit if the adapter allows.
6. A render graph with optional passes (shadows, SSAO, water, clouds) that are turned off on the Low profile. Mods can add their own passes via WGSL.

### LOD

Distant Horizons proved this works: distant terrain is rendered as simplified LOD chunks prepared on background threads ([DH](https://distant-horizons-guide.pages.dev/about)). It takes a snapshot of a chunk when it leaves the render distance, so distant changes are not visible ([discussion](https://www.vintagestory.at/forums/topic/12204-distant-horizons-minecraft-mod-for-vintage-story/)). Our engine makes LOD native and server-driven:

- **LOD rings (clipmap).** LOD0 = full sections. LOD n — the same 16³ voxel grid, but a voxel = 2ⁿ blocks, so a LOD section covers (16·2ⁿ)³ blocks (LOD5 — 512³). Each next ring is twice as wide: from 12 chunks, the LOD0 rings reach 24, 48, 96, 192, 384 chunks; for a 64-chunk target LOD0–LOD3 is enough. At ring boundaries — "skirts" or overlap, otherwise gaps are visible between levels.
- **The server computes LOD and keeps it up to date.** After a block change, parent LOD nodes are marked "dirty" and recomputed in batches. The client receives LOD sections in the same palette format (the dominant block type per voxel), not full chunks, so the mesher and renderer are the same. The "height + color" format is cheaper but loses caves and overhangs and is incompatible with 3D rings. All levels together add up to \~14% to the world volume on the server (1/8 + 1/64 + …).
- **Dynamic changes** (seasons, snow) are delivered as shader parameters, not as new geometry.
- **Disk LOD cache** on the client: re-entering the world does not download it again.
- **R&D track:** rendering distant LOD via SVDAG and raymarching. For example, Aokana holds scenes with tens of billions of voxels and loads only about 5% of the scene into VRAM ([Aokana](https://arxiv.org/html/2505.02017v1)). But this is not for the first version.

### Lighting and generation

- Lighting — wave BFS (flood fill) over sections in the task pool; beyond section boundaries — queues at the borders. Light removal (a broken torch) — a separate reverse BFS, then refill. Lighting tasks work on a snapshot of sections, and the region applies the result in the next tick, so there is no race with the tick. Light reaches 15 blocks at most, so with a region buffer ≥ 1 section it does not leave the region. Sky light: the heightmap gives the start, then the same BFS under overhangs and into caves. Test: a full recompute of one section — 25–45 µs, of 3×3×3 sections — 1.4–2.2 ms, so on a block change only an incremental update, not a full recompute.
- World generation — a pipeline of stages (terrain → biomes → caves → structures → decorations), each stage is a task in the pool, noise via SIMD. Mods add stages or replace them.
- The heightmap and biome map are generated by a cheap pass ahead of full chunks, so distant LOD is visible even before the world is generated inside.

## 5. Mod system

Mods have three levels: data (no code), scripts in WASM (sandbox), and native engine extensions (only for the project team). The engine parallelizes mods itself, because they declare what they access.

### Three mod levels

| Level | What you can do | Performance cost | Who writes it |
| --- | --- | --- | --- |
| 1. Datapack (TOML/JSON + assets) | Blocks, items, recipes, loot, biomes, structures, models, sounds, localization | Zero: compiled into registries at load time | designers, artists |
| 2. WASM mod (Rust, C, Zig, AssemblyScript, Go) | New systems, components, mob AI, mechanics, commands, UI, generation stages | A call across the host/guest boundary: in a test \~20–35 ns for a core module and \~0.4 µs for a component; 1,000 separate component calls — 0.5 ms, one batch — \~3 µs, so batched APIs are mandatory | modders |
| 3. Native engine plugin (Rust crate) | New render passes, world formats, physics | Same as the engine; built together with the engine | project team |

We do not support native `.dll`/`.so` files from third-party authors: Rust has no stable ABI, and such a mod can do anything to the player's PC. WASM solves both issues.

### Why wasmtime + Component Model

- A mature choice: Veloren (a voxel RPG in Rust) already loads plugins via `wasmtime::component` and a WIT file for its API ([Veloren code](https://docs.veloren.net/src/veloren_common_state/plugin/module.rs.html)). For Bevy there is an experimental Wasvy with the same idea ([Wasvy](https://lib.rs/crates/wasvy)).
- The WIT contract defines types at the binary level, so a mod's incompatibility with the API version is visible before launch ([explanation](https://maddevs.io/writeups/wasm-component-model-practical-guide-part-one/)).
- Fuel, epoch, and memory limits already exist in the ecosystem ([wasm-link](https://docs.rs/crate/wasm-link/0.4.1)): a hung mod will not stop the server. The cost per Wasmtime documentation: epoch — about 10% slowdown and a non-deterministic stopping point (depends on the clock); fuel is fully deterministic but more expensive. Our test (wasmtime 50): epoch 0–9%, fuel +50–90% on code with many function calls, so fuel is for tests and replays only.
- Risk: Component Model tooling is still changing quickly — we pin tool versions in the SDK.

### How a mod plugs into the engine

1. **Namespaced registries.** Everything has an ID of the form `mymod:copper_pipe`. The server assigns numeric IDs and passes the table to the client on join — ID conflicts are impossible. Numeric IDs change along with the mod list, so the world stores a "string ID → number" table, and adding or removing a mod does not shuffle blocks in saves.
2. **Systems by phase.** A mod registers a system in a tick phase and declares access (`read: Position`, `write: Health`). The scheduler runs independent systems in parallel, like engine systems. A wasmtime Store cannot be called from several threads at once, so each pool thread has its own mod instance. Mod state lives in ECS components on the host side, and linear memory is only scratch space for a single call. An instance per region would cost 1–15 GB (50–150 regions × 20–50 mods × 1–2 MB, estimate); a per-thread pool is tens of times less.
3. **Region context.** A system gets access only to its own region; to neighbors — via `send_to(pos, msg)`. So a mod is automatically multithreading-safe. Global queries (the nearest player in the world, the leaderboard) are asynchronous: they read the global region snapshot from the previous tick, not live state.
4. **Events and hooks.** `on_block_break`, `on_entity_damage`, `on_player_join`, etc. Events are delivered in batches once per tick, not one by one, so as not to pay for the host/guest transition hundreds of times.
5. **Batched world changes.** `set_blocks(&[(pos, state)])` instead of thousands of `set_block`; remeshing and broadcasting are merged. The host applies a batch only after the call completes successfully; if the mod is interrupted (epoch, panic), the entire batch is discarded and the world is not left half-modified.
6. **Client/server sides.** The manifest says where each part runs: `server`, `client`, `both`. The client never receives a mod's server code.
7. **Component synchronization.** A component marked `#[replicate]` is sent by the engine to clients as deltas within the visibility zone. This works only for mods with side both: the client must know the component type, so the schema of replicated components is part of the mod's versioned contract.

### Security and control

- Permissions in the manifest (`network`, `filesystem:mod_data`). A client mod has no access to the network or files by default.
- Per-mod budgets: time per tick (epoch interruption), memory (linear memory limit). On overrun — a warning in the profiler, then the mod is terminated. Epoch here is a watchdog against hangs, not precise accounting: accuracy equals the epoch timer period (e.g. 1 ms), and time per tick is counted by the profiler. In scenario tests (engine-cli test) fuel is used instead of epoch, so runs are repeatable.
- The built-in profiler shows each mod's time per tick (the `/profile mods` command).
- Mods for public servers are signed (ed25519); the launcher verifies the signature and hash.

### API versioning

- WIT package `engine:api@1.x` under semver. Minor versions only add functions, major ones may break.
- The engine supports the current and previous major versions via adapters, so old mods live for at least one update cycle.
- Dependencies between mods follow semver in the manifest; mods can export their own WIT interfaces to other mods.

## 6. Guide: how to write mods

A mod is a folder with a manifest, data, assets, and (if needed) a compiled `.wasm`. Start with data and write code only for behavior that data cannot describe.

### Mod structure

```text
copper_pipes/
├─ mod.toml            # manifest
├─ data/
│  ├─ blocks/copper_pipe.toml
│  ├─ items/wrench.toml
│  └─ recipes/copper_pipe.toml
├─ assets/
│  ├─ textures/block/copper_pipe.png
│  ├─ models/copper_pipe.json
│  └─ lang/uk.json
├─ wit/                # interfaces, if the mod provides an API to others
└─ src/lib.rs          # code (builds into server.wasm / client.wasm)
```

### Manifest

```toml
[mod]
id = "copper_pipes"
version = "0.3.1"
api = "^1.2"            # engine:api version
side = "both"
authors = ["Hi"]

[dependencies]
core = "^1.0"           # base gameplay mod
fluids = { version = "^0.5", optional = true }

[permissions]
filesystem = "mod_data"

[budget]
tick_ms = 0.5           # expected time per tick per region
```

### A block via data (no code)

```toml
# data/blocks/copper_pipe.toml
id = "copper_pipes:copper_pipe"
model = "copper_pipes:models/copper_pipe"
hardness = 2.5
tool = "pickaxe"
light_emission = 0
opaque = false
states = { facing = ["north", "south", "east", "west", "up", "down"] }
components = ["copper_pipes:flow"]   # behavior from WASM
```

### Behavior via code (Rust → WASM)

```rust
use engine_sdk::prelude::*;

#[derive(Component, Default)]
#[replicate]                     // the client will see the value
struct Flow { amount: u16 }

#[mod_init]
fn init(app: &mut ModApp) {
    app.register_component::<Flow>("copper_pipes:flow")
       .add_system(Phase::Simulate, flow_system)
       .on_event(on_wrench_used);
}

// Access is declared in the signature: the engine parallelizes it itself
fn flow_system(region: &mut Region, mut pipes: Query<(&BlockPos, &mut Flow)>) {
    let mut changes = Vec::new();
    for (pos, mut flow) in pipes.iter_mut() {
        let below = *pos + IVec3::NEG_Y;
        if region.contains(below) {
            // local change — directly
            changes.push((below, flow.amount / 2));
        } else {
            // rare: the region buffer usually contains the neighbor;
            // outside the region — only by message (reply after 2 ticks)
            region.send_to(below, FlowTransfer(flow.amount / 2));
        }
        flow.amount /= 2;
    }
    region.apply_flow_batch(&changes); // one host call instead of hundreds
}
```

### Modder workflow

1. `engine-cli new my_mod` — a template with a manifest and SDK.
2. `engine-cli dev` — starts a local server and client, watches files; data, assets, and `.wasm` reload without restarting the game. State in host components survives a reload; if the component schema changed, a migration or reset is needed.
3. `engine-cli test` — a headless server runs the mod's scenario tests (a world with a fixed seed, N ticks, state checks).
4. `engine-cli bench` — measures the mod's time per tick and memory; fails if it exceeds `budget`.
5. `engine-cli publish` — signs and uploads to the project's mod registry.

### Rules for fast mods

- Anything that can be described with data, describe with data.
- No global state: data lives in components or region resources.
- To other regions — only by messages; the reply arrives no earlier than 2 ticks later.
- World changes — in batches; one host call per system, not per block.
- Rare work — via `schedule_every(n_ticks)`, not a check every tick.
- Heavy computation (pathfinding, generation) — via `spawn_job`, the result returns as an event.
- Client code does not trust itself: decisions that affect gameplay are made by the server.

## 7. Launcher

Decision (section 14, no. 9): there is no separate launcher — the game's main menu plays its role, and a tiny updater updates the binary. The menu does four things: Microsoft login, game ownership verification, asset download, and mod synchronization for a specific server.

### Login

1. We register an Azure application and submit the form for Minecraft API access. Without approval `api.minecraftservices.com` returns 403 ([wiki.vg](https://www.wiki.vg/MSA)).
2. Device Code Flow or Auth Code + PKCE: Microsoft → Xbox Live → XSTS → Minecraft token. A ready Rust implementation exists in `lighty-auth` ([docs](https://hamadi.gitbook.io/lightylauncher/crates/auth/docs/microsoft)).
3. Game ownership verification is our working hypothesis that this player may download Mojang assets for our engine. This is not a legal conclusion; a lawyer checks it at stage 0.
4. The launcher passes the Minecraft token to our login service. It verifies the profile and ownership via api.minecraftservices.com, does not store the token, and issues its own session token (a JWT signed with Ed25519, 15–60 min + refresh). A 24-hour JWT cannot be quickly revoked on a ban or theft. Game servers verify only our token. Microsoft tokens never reach game servers.
5. The refresh token is stored in the system secret store (Keychain, Credential Manager, libsecret).

### Assets

- Minecraft assets are taken from the official version manifest and asset index (textures and models from client.jar, sounds from the object storage) on the player's machine. We do not distribute them; format conversion (below) happens only on the player's machine, and this is also a question for a lawyer.
- After download the launcher converts them into the engine's internal formats (texture arrays, baked models; BC7 compression on PC — only for HD packs, since a 16×16 texture is 1 KB anyway; ASTC is not needed for PC) and caches them locally.
- The engine has its own set of fallback assets. The game does not depend on Mojang technically, and this is insurance against legal problems (see Risks).

### Mods and updates

- The server publishes a manifest: engine version, list of mods with SHA-256 hashes and signatures. The launcher downloads only what is missing.
- Content-addressed cache: an identical file shared by several servers is stored once.
- Engine updates — binary deltas (zstd --patch-from), atomic replacement, rollback on error.
- Profiles (server + a set of client mods), a mod catalog, crash reports with the player's consent.
- One "Play" button: login → version check → download → engine launch with the token and server address.

## 8. Server and scaling

Scale is achieved on two levels: inside a node — by regions on all cores, between nodes — by spatial shards with player handoff without loading screens.

### Inside a node

- Regions as in section 3. By Folia's experience, servers where players are spread across the world benefit most (SMP, Skyblock).
- Threads: network, chunk IO, generation, region ticks. In a Folia test with \~330 players the reference was about 4 network and 3 IO threads per 200–300 players, and Folia itself recommends at least 16 physical cores ([Folia](https://github.com/PaperMC/Folia)). So the 500-players-per-node target is computed for 16+ cores. In our case the thread count is tuned automatically from metrics.
- World pregeneration is a standard command; on-the-fly generation is the most expensive part of the load.

### Network

- QUIC: reliable streams for chunks, inventory, and chat; unreliable datagrams for entity positions. A datagram must fit in one packet (\~1200 B) and is also subject to congestion control, so during chunk streaming positions need an explicit priority.
- Interest management: the client receives only entities within the visibility zone; distant ones are updated less often. Estimate: at 20 Hz and \~12 B per delta, 20 moving entities in view give \~48 kbit/s including QUIC headers, 50 entities — \~105 kbit/s. So the 50 kbit/s target holds only with less frequent updates for distant entities (5–10 Hz).
- Delta compression of state relative to the last acknowledged snapshot; chunks — zstd with a dictionary.
- Chunk streaming priority: first in front of the camera and close, then LOD, then the rest. Estimate for joining with 12 chunks: 625 columns × 4–16 KB per compressed column (assumption) = 2.4–9.8 MB, i.e. 1–4 s at 20 Mbit/s. So the "join ≤ 5 s" metric counts time until control (3–4 chunks around), and the rest is downloaded in the background.
- Client-side prediction and reconciliation by tick number for movement, digging, and block placement.

### Sharding between nodes

1. **Gateway.** The client keeps one connection to the gateway, and the gateway routes traffic to the node that owns its position. The transition between nodes is invisible to the player.
2. **Spatial partitioning.** The world is divided into cells (e.g. 512×512 blocks). The coordinator assigns cells to nodes and redistributes them by load (a hot cell is split).
3. **Overlap zones.** A node sees the neighbor's edge in read-only mode (shadow copies of entities), so players see each other across the border. Shadow copies are display-only: any action across the border (a hit, an explosion, a block change in the overlap zone) goes as a command to the owner node, and its result arrives with the inter-node network delay.
4. **Entity handoff.** Serialize components → transfer via the bus (NATS) → acknowledgement → change of owner. Each entity has an ownership number that grows with every handoff; receiving is idempotent, a command with an old number is discarded, and the source of truth about the owner is the event log. Core NATS delivers "at most once", so JetStream ("at least once" + idempotency) or a direct node-to-node connection with acknowledgement is needed. Classic 2PC hangs when the coordinator fails, so we do not use it. During the handoff the entity is frozen for 1–2 ticks; the client does not see this thanks to interpolation.
5. **Global services** are separate: chat, friends, economy, auctions, player profiles (PostgreSQL). Mods access them through an asynchronous API.
6. **Instances** (minigames, dungeons, shows) — separate worlds that the coordinator creates on free nodes on request.

### Persistence and reliability

- Region files on shared storage (or S3-compatible), written copy-on-write without stopping the tick.
- Event log for economy and inventory: after a node crash state is restored without dupes.
- Metrics (Prometheus): tick time per region, each mod's time, traffic, task queues.
- Bot load tests: headless clients that walk, dig, and build, by the thousands in CI.

## 9. Phased plan

The work is split into 8 stages, about 3–4 years in total for a team of 3–5 engineers (estimate, no source). The sum of the durations of stages 0–6 is 31–34 months (≈2.6–2.8 years); the rest is a risk reserve. Each stage ends with a working build and a numeric exit criterion; without it the next stage does not start.

### Stage 0. Research and prototypes (2–3 months)

- [ ] Benchmarks: meshing, palette storage, wasmtime calls, BFS lighting
- [ ] GPU-driven renderer prototype on wgpu + fallback for weak GPUs; Features/Limits dump on iGPU and GTX 1050 (Vulkan, DX12, GL) — B/C prototype and dumps done in T0-06/T0-14 on Radeon Vega 10 + GTX 1660 Ti (open question 12; GTX 1050 not verified); path A is stage 2
- [ ] First version of the WIT contract `engine:api@0.1`
- [ ] Submit the Minecraft API access form; legal consultation on assets

**Exit:** all numbers from "Success metrics" are confirmed or adjusted by benchmarks; architectural decisions are recorded in ADRs.

### Stage 1. Core and single-player world (6 months)

- [ ] ECS core, tick phases, task pool with priorities
- [ ] Palette sections, terrain generator, lighting, saving
- [ ] Rendering: meshing, culling, texture arrays, quality profiles
- [ ] Player physics, digging and placing blocks
- [ ] Loading Minecraft assets from a local cache + fallback assets
- [ ] CI with benchmarks and Tracy tracing

**Exit:** 60 FPS on iGPU at 12 chunks (1% low ≥ 45), RAM ≤ 1.5 GB without LOD (target with LOD — 2 GB), walking around the world for 30 min without frame spikes > 33 ms.

### Stage 2. Network and dedicated server (4–5 months)

- [ ] QUIC protocol, component replication, interest management
- [ ] Movement prediction and reconciliation
- [ ] Regional multithreading on the server
- [ ] Headless bots for load tests

**Exit:** 200 bots on one node (8 cores), tick ≤ 25 ms at p99, traffic ≤ 50 kbit/s per player in steady state (without chunk streaming, ≤ 20 moving entities nearby).

### Stage 3. Mod API v1 and launcher (4 months)

- [ ] Datapacks, WASM host, permissions, budgets, hot reload
- [ ] SDK for Rust + `engine-cli` (new / dev / test / bench / publish)
- [ ] In-game menu + updater: Microsoft login, assets, mod manifests, updates
- [ ] Mod registry with signatures

**Exit:** one test data mod + one WASM mod are installed automatically via the launcher; hook call overhead ≤ 1 µs with epoch enabled; a hung mod does not stop the server.

### Stage 4. Base gameplay as the `core` mod (6 months)

- [ ] Blocks, items, inventory, crafting, mobs and AI, biomes, structures — all through the public API
- [ ] Mechanical logic (a redstone analog) with deterministic ordering
- [ ] The project's first unique mods

**Exit:** closed alpha for 50 players; no `core` function required a private API.

### Stage 5. LOD and weak hardware (3–4 months)

- [ ] LOD rings, server LOD sections, disk cache
- [ ] Automatic quality profile selection, tests on a fleet of weak PCs

**Exit:** 64 chunks of visibility (12 full + LOD1–LOD3) at 60 FPS on a GTX 1050; RAM ≤ 2 GB.

### Stage 6. Scale: shards (6 months)

- [ ] Gateway, coordinator, overlap zones, handoff
- [ ] Global services, instances, metrics

**Exit:** 5,000 bots on a 10-node cluster (≈500 per 16-core node, tick ≤ 25 ms at p99 — this is the first check of the "500 per node" metric); transitions between nodes without dupes or item loss in 10,000 automated tests.

### Stage 7. Open beta and open SDK (ongoing)

- [ ] API documentation, examples, SDK for a second language (AssemblyScript or Zig)
- [ ] Anti-cheat, mod registry moderation

**Exit:** third-party authors release mods without the team's help.

A realistic path for a small team: while stages 0–3 are underway, the community lives on a Fabric + Folia server. Mod ideas are tested there and later ported to the new engine.

## 10. Risks

The biggest risk is not technical but legal and scope-related: Mojang assets in a third-party engine are a gray area, and the scope of work is years. Both risks are reduced the same way: the engine must work with its own assets and grow in stages.

| Risk | Probability | Impact | What we do |
| --- | --- | --- | --- |
| Mojang assets in a non-Minecraft engine violate the EULA or Usage Guidelines | medium | high | Legal consultation at stage 0; assets only from official servers for game owners; our own full asset set before the open release |
| Microsoft does not approve the Azure application or revokes access | medium | high | Own accounts as a fallback login method; the session token does not depend on Microsoft |
| The name or brand sounds like "Minecraft" | low | medium | Own name and logo; the "not affiliated with Mojang" formula |
| Scope of work exceeds the team's capacity | high | high | Strict exit criteria; a parallel Fabric + Folia server for the community and revenue |
| Breaking changes in the `bevy_ecs` and `wgpu` APIs | high | medium | Pinned versions, a thin abstraction layer, scheduled updates twice a year |
| Component Model tooling is still unstable | medium | medium | Pinned versions in the SDK; fallback — core modules with a custom ABI |
| WASM overhead higher than expected | medium | medium | Batched APIs, a benchmark at stage 0; hot `core` systems can be moved into a native plugin |
| Old GPUs without compute or indirect draw | medium | medium | CPU culling fallback with the same data format; quad as a per-instance attribute, since GL ES/WebGL2 has no storage buffers; Features/Limits dump on target GPUs at stage 0 |
| Item dupes during handoff between shards | medium | high | Ownership number + idempotent receiving, event log, automated tests for node crashes |

I am not a lawyer; the legal rows of the table are open questions for consultation, not conclusions.

## 11. Sources

- [binary-greedy-meshing 0.4.2 — meshing benchmarks](https://docs.rs/crate/binary-greedy-meshing/0.4.2)
- [Folia — regional multithreading (official PaperMC repository)](https://github.com/PaperMC/Folia)
- [Valence — a Rust server framework on Bevy ECS](https://www.sourceforge.net/projects/valence.mirror)
- [Pumpkin — a Rust Minecraft server](https://minecraft.how/blog/post/pumpkin-rust-minecraft-server-guide)
- [Veloren — plugins on wasmtime::component](https://docs.veloren.net/src/veloren_common_state/plugin/module.rs.html)
- [Wasvy — WASM mods for Bevy](https://lib.rs/crates/wasvy)
- [wasm-link — fuel/epoch/memory limits](https://docs.rs/crate/wasm-link/0.4.1)
- [WASM Component Model in practice](https://maddevs.io/writeups/wasm-component-model-practical-guide-part-one/)
- [Distant Horizons — LOD description](https://distant-horizons-guide.pages.dev/about)
- [Aokana — GPU-driven voxel rendering (arXiv)](https://arxiv.org/html/2505.02017v1)
- [Microsoft Authentication Scheme (wiki.vg)](https://www.wiki.vg/MSA)
- [lighty-auth — Microsoft login in Rust](https://hamadi.gitbook.io/lightylauncher/crates/auth/docs/microsoft)
- [Folia region logic — merging, splitting, region buffer](https://docs.papermc.io/folia/reference/region-logic)
- [Wasmtime — epoch and fuel: cost and determinism](https://docs.wasmtime.dev/examples-interrupting-wasm.html)
- [wgpu Features — MULTI\_DRAW\_INDIRECT\_COUNT, INDIRECT\_FIRST\_INSTANCE](https://docs.rs/wgpu/latest/wgpu/struct.Features.html)
- [wgpu Limits — storage buffers, texture arrays, WebGL2](https://docs.rs/wgpu/latest/wgpu/struct.Limits.html)
- [binary-greedy-meshing 0.6.4 — arbitrary chunk size, quad format](https://docs.rs/binary-greedy-meshing/latest/binary_greedy_meshing/)
- [NATS JetStream — delivery guarantees](https://docs.nats.io/nats-concepts/jetstream)

## 12. Review log

The review found three serious mechanics errors (a 62³ mesh chunk is incompatible with 16³ sections, WASM mod state under parallel regions, 2PC on top of core NATS) and several inflated or unverified numbers. Tests were run on a 2-core VM without a GPU (±30%); calculations are estimates with named assumptions.

| Section | What it was | What was fixed / why | Verification (source or test) |
| --- | --- | --- | --- |
| 4. Meshing | A 62³ mesh chunk "groups 16³ sections"; remeshing "sections only" | 62 is not divisible by 16 → a 32³ mesh chunk (or 48³), remeshing of the whole mesh chunk | Crate 0.6.4 code: `Mesher<const CS>` with CS ≤ 62 |
| 1, 4. Meshing | 65/90 µs without caveats; metric ≤ 0.2 ms for a "chunk" | Clarified: this is fast\_mesh with ready masks; the metric is for 32³, marked as unconfirmed | [docs.rs 0.4.2](https://docs.rs/crate/binary-greedy-meshing/0.4.2): mesh 300–340 µs; VM test: fast\_mesh 0.27 ms, mesh 1.1 ms, masks 0.35 ms |
| 4. Quad 8 B | "Position, size, type, and light in a u64"; "8–10× less VRAM" | Smooth light and AO do not fit and break greedy merging → light from a 3D texture; gain 12–19× | Bit layout from README 0.6.4; calculation `review/budget.py` |
| 4. GPU-driven | "The whole world in 1–3 draw calls" everywhere; fallback "the same format" | MDI count only on DX12/Vulkan 1.2+; without it emulation and instance\_count = 0; on GL ES/WebGL2 the quad as a per-instance attribute; limits 128/256 MiB and 256 layers; second Hi-Z pass | [wgpu Features](https://docs.rs/wgpu/latest/wgpu/struct.Features.html), [wgpu Limits](https://docs.rs/wgpu/latest/wgpu/struct.Limits.html) (wgpu 30) |
| 4. LOD | "Sections with a 32³ voxel" (larger than a section); "height + color" format with 3D rings | LOD n = 16³ voxels of 2ⁿ blocks; the same palette format; up to 64 chunks LOD0–LOD3 is enough; skirts against gaps; +14% volume | Arithmetic: 12·2ⁿ = 24…384 |
| 3. Regions | Neighboring regions communicate request/response in the same tick; merging without rules | Buffer invariant as in Folia (local actions do not leave the region), merge after the tick; reply after ≥ 2 ticks | [Folia region logic](https://docs.papermc.io/folia/reference/region-logic) |
| 3. Determinism | "Redstone is repeatable" with lock-free queues | Sorting of messages in Exchange; guarantee only with the same region partitioning | Mechanics analysis |
| 3, 4. Memory, lighting | No chunk memory budget; lighting in the pool not synchronized with the tick; light removal not described | Added RAM numbers, palette hash index, section snapshot, reverse BFS, incremental update | VM test: 15,000 sections ≈ 17 MiB + light 59 MiB; 8-bit set 67 ns; relight 3×3×3 1.4–2.2 ms |
| 5. WASM state | "The engine parallelizes mods itself" without an instance model | An instance per thread, state in host components (without 1–15 GB for per-region instances); hot reload preserves state | `review/budget.py` estimate; VM test: Store + instantiation 4.2 µs |
| 5. Epoch/fuel | Time budget via epoch without cost or effect on determinism | Epoch is a watchdog in prod, fuel — in tests; a batch of changes is discarded on interruption | [Wasmtime](https://docs.wasmtime.dev/examples-interrupting-wasm.html): epoch \~10%; VM test: epoch 0–9%, fuel +50–90% |
| 5. Hook call | "≤ 1 µs" without data | Added numbers; batched APIs are mandatory | VM test: core 17–35 ns, component \~0.4 µs; 1,000 calls 0.5 ms vs \~3 µs as a batch |
| 5. Registries, synchronization | Numeric IDs without a table in the save; #\[replicate\] for any mod; global queries not described | ID table in the world; replicate only for both; global queries — a snapshot of the previous tick | Mechanics analysis |
| 7. Login | A 24-hour JWT; not stated how the Minecraft token is verified | The login service verifies profile and ownership, JWT 15–60 min + refresh so that a ban takes effect quickly | [wiki.vg MSA](https://www.wiki.vg/MSA): token chain, 403 without approval, no secret needed |
| 7. Assets | "Ownership is our basis"; "we do not modify" but we convert; BC7/ASTC | Working hypothesis pending a lawyer; local conversion is also a question for the lawyer; ASTC removed for PC | Text reading (not a legal check) |
| 8. Network | Target 50 kbit/s and join ≤ 5 s without calculation | Estimates: 20 entities ≈ 48 kbit/s, 50 ≈ 105; join 1–4 s at 20 Mbit/s; datagram limit \~1200 B | `review/budget.py` (assumptions about delta and column size); zstd test unsuitable (synthetic data) |
| 8, 10. Shards | A "two-phase protocol" on top of NATS; shadow copies without rules for actions across the border | Ownership number + idempotent receiving + JetStream or a direct channel; actions across the border — a command to the owner | [NATS JetStream](https://docs.nats.io/nats-concepts/jetstream): core = at most once |
| 8. Server | A reference to the jdkeke142/Folia fork; 500 per node without hardware | Official PaperMC/Folia; "16+ cores" in the targets and the stage 6 exit | [Folia](https://github.com/PaperMC/Folia): ≥ 16 cores, \~330 players in the test |
| 9. Plan | 3–4 years without a schedule; stage exits not tied to metrics (500/node is never checked; RAM 1.5 vs 2 GB) | Sum of stages 31–34 months; exits of stages 1, 2, 3, 5, 6 refined; GPU capability dump in stage 0 | Duration arithmetic |
| 5. Veloren | — | Left unchanged: the claim is confirmed | [Veloren code](https://docs.veloren.net/src/veloren_common_state/plugin/module.rs.html): `wasmtime::component`, `bindgen!` from WIT |

### Open questions

- Mesh chunk size — decided: 32³. The extra draw commands are compensated by 2×2×2 mesh chunk batches (section 14).
- Is a web client (WebGL2) needed? If not, the GL path can be kept minimal.
- Determinism: are bit-exact server replays needed? Then the region partitioning must be logged, and mods run with fuel (+50–90%).
- Source of truth for an entity's owner between shards: JetStream or PostgreSQL with a record version?
- Passing the Minecraft token to our own login service — do Microsoft's terms allow it? A question for the lawyer together with assets.
- The real size of a compressed column and server RAM for 500 scattered players — measure on a real world (a synthetic zstd test is not suitable).
- Tick 25 ms at 500 players, MDI on iGPU and GTX 1050, 60 FPS — not verified (no GPU or server in the tests).

## 13. Optimization analysis: recommendations

**Main conclusion.** The client memory target (≤ 2 GB) and the mod hook overhead target (≤ 1 µs) are realistic with headroom. The 0.2 ms mesh budget is reachable only for `fast_mesh` with maintained masks. The real bottlenecks are not the mesh or WASM, but gathering data into a mesh unit (gather/masks ≈ 0.35 ms), incremental lighting, server memory at 500 players, and vertex throughput on iGPU. The server targets (25 ms @ 500 players, 5000+ with sharding) are not yet backed by any test and have the status of a hypothesis.

**Mesh chunk decision: 32³** (section 14). This analysis computed on a 62³ unit; the numbers scale proportionally to 32³ (≈ 1/7.3 voxels per unit, ≈ 7× more units). The number of draw commands stays as with 62³ thanks to 2×2×2 batches.

### 13.0 How to read this section

- Sources: benchmarks on a noisy VM (2 vCPU Xeon 2.1 GHz, error ±30 %), web research, raw benchmark logs.
- Meshing was additionally run with `-C target-cpu=x86-64-v3`: `fast_mesh` 262–291 µs vs 269–302 µs baseline — within noise.
- Checked on docs.rs: wgpu 30.0.2 has `DownlevelFlags::VERTEX_STORAGE` and `INDIRECT_EXECUTION` (absent on WebGL2, GLES 3.0, Apple1/2). quinn 0.11.12 has `SendStream::set_priority`.
- Labels: **\[test\]** — measured; **\[source\]** — cited source; **\[estimate\]** — arithmetic or assumption.

### 13.1 Where research and tests diverge

| Topic | Research | Tests | Why I trust it |
| --- | --- | --- | --- |
| Mesh time | 50–200 µs (without transparency); \~600 µs with transparency | `mesh()` 1.1 ms; `fast_mesh` 0.27 ms (VM, 8 % transparent) | The tests — for relative conclusions (`fast_mesh` is 4× faster). The research describes an old crate version. |
| Crate version | 0.3.6 | 0.6.4 (has `fast_mesh`, `mesh_lod`) | The tests (Cargo.lock). |
| Epoch | \~10 % (Wasmtime documentation) | 0–9 % | Consistent. |
| Fuel | "more expensive than epoch", no numbers | +50–92 % on `fib(35)`, \~0–2 % on a tight loop | The tests. |
| Palette packing | "indices may cross a long" | No crossing, as in MC 1.16+ | The test variant: simpler and faster access. |
| zstd for regions | Linear: −50–60 % (no methodology) | Synthetic: unrealistic 19–184× | Neither, for absolute numbers. |

### 13.2 Meshing

**Decision:** `binary-greedy-meshing` 0.6.4, `fast_mesh` mode, opaque/transparent masks maintained incrementally. Fallback: our own port of cgerikj v2 for a different unit size.

| Mesher \[test, 62³ unit\] | Median | p95 | Quads | Bytes (8 B/quad) |
| --- | --- | --- | --- | --- |
| `mesh()` | 1083–1178 µs | 1540–1990 µs | 4962 | 39 KB |
| `fast_mesh` (ready masks) | 269–278 µs | 457–556 µs | 4962 | 39 KB |
| naive | 898 µs | 1425 µs | 11762 | 92 KB |
| building masks from scratch | 341–366 µs | 550–636 µs | — | — |
| empty / solid unit | 52 µs | \~66 µs | 0 | 0 |

**Revision of the "≤ 0.2 ms/chunk" target:**

- `fast_mesh` ≤ 0.3 ms p50 and ≤ 0.6 ms p95 per 62³ unit on a reference 4-core CPU;
- full pipeline (gather + masks + mesh) ≤ 0.5 ms p50 per unit;
- skip empty and solid units based on section metadata, without calling the mesher.

**Load** \[estimate, full pipeline ≈ 0.62 ms\]: a 12-chunk radius ≈ 260 units of 62³. Two workers manage ≈ 53 units per frame, a full rebuild ≈ 5 frames. A block edit on a border affects up to 4–8 units ≈ 2.5 ms — fits in the same frame. Conclusion: CPU meshing time is not the bottleneck; the bottleneck is gather and masks.

| Risk | Experiment | Threshold |
| --- | --- | --- |
| Gather from the palette via `get()` ≈ 1 ms per unit \[estimate\] | Bulk unpacking of a section into a 64³ buffer, masks straight from the palette | gather + masks ≤ 100 µs p50 |
| Incremental masks not implemented | Bit updates on `set_block`, 10,000 edits | ≤ 20 ns per edit |
| The 62 grid is not a multiple of 16 | Proptests of index math at borders | 0 mismatches with the reference |
| AO not measured | AO bits in the quad | ≤ +30 % to `fast_mesh` |
| Desktop CPU not tested | The same bench on 4+ GHz | `fast_mesh` ≤ 0.2 ms p50 |

### 13.3 GPU rendering and the fallback path

**Decision (stages 0–1):** 8 B quads, vertex pulling from a shared storage buffer (slab allocator). One draw per unit per face direction per pass. CPU frustum culling and face-direction discarding. Vertex from `vertex_index`, so `first_instance` is not needed.

**Fallback path:** without `VERTEX_STORAGE` the same 8 B are fed as a per-instance attribute. **Stage 2+:** compute culling + `multi_draw_indirect(_count)` where `INDIRECT_EXECUTION` exists; Hi-Z — only after measurements. Confidence: medium, GPU not tested.

**Vertices** \[estimate\]: ≈ 100 non-empty units × 4962 quads ≈ 0.5 M quads = 3 M vertices per frame, at 60 FPS ≈ 180 M vertices/s (≈ half that after face discarding). **This is the main unmeasured renderer risk.** Draw calls without indirect ≈ 600.

| Risk | Experiment | Threshold |
| --- | --- | --- |
| Vertex throughput on iGPU | A 0.5 M quad scene: vertex pulling vs instanced vs indexed; Intel UHD + GTX 1050 | ≤ 8 ms GPU time per frame |
| Instanced quads are slow | Compare with `draw(0..6n)` | no worse than 1.3× |
| Feature support | Dump `features()`, `limits()`, `downlevel_capabilities()` on target GPUs | `VERTEX_STORAGE` on Vulkan and DX12 |
| Draw cost | 600 vs 2000 draws on iGPU | ≤ 1 ms CPU for command recording |
| Cave culling | Visibility graph between unit faces | ≥ 50 % of units culled in a cave |

### 13.4 LOD (up to 64 chunks)

**Decision:** 2× voxel downsampling per level, the same mesher (`mesh_lod` in 0.6.4, not tested). The server sends the base LOD level. Seams — "skirts". LOD lighting — sky light only, baked into the vertex. Aokana (SVDAG) rejected for stages 0–1: no editing, no transparency, needs compute.

| Ring \[estimate\] | Area, m² | Quad divisor | Mesh, MB | Units |
| --- | --- | --- | --- | --- |
| L0: 0–12 chunks | 160,000 | 1 | 1.6 | ≈ 260 |
| L1: 12–24 | 430,000 | 4 | 1.1 | ≈ 87 |
| L2: 24–48 | 1,770,000 | 16 | 1.1 | ≈ 44 |
| L3: 48–64 | 1,830,000 | 64 | 0.3 | ≈ 7 |
| Total | 4,190,000 | — | ≈ 4 (×5 reserve = 20) | ≈ 400 |

64 chunks via LOD are cheap. But **full data for 64 chunks is impossible**: 399,384 sections × 4 KB of light = 1.52 GiB for light alone. Full voxel data only within a radius of 12–16 chunks.

| Risk | Experiment | Threshold |
| --- | --- | --- |
| LOD update after edits | Edit → recompute ancestors L1..L3 | ≤ 1 ms per edit |
| Cracks at level borders | Screenshot test with skirts | 0 holes from 8 angles |
| `mesh_lod` | Compare with downsampling + `fast_mesh` | time ≤ `fast_mesh`; quads ≤ 1/3 of L0 |

### 13.5 Lighting

**Decision:** nibbles per section (4 KB per section), a Single variant for uniform sections, **incremental add/remove BFS queues**, two-pass lighting in 2×2 groups for initial generation. The client computes light for rendering; the server — only for gameplay (mob spawning), lazily.

| Scenario \[test\] | 16³ | 48³ (27 sections) |
| --- | --- | --- |
| 1 torch, open | 24 µs | 36 µs |
| 8 torches/section, open | 46 µs | 1415 µs |
| 26–45 % opaque, 8 torches | 25–27 µs | 2062–2181 µs |
| Sky | not representative | 554 µs |

&#91;Estimate\] One new torch incrementally ≈ 36–100 µs, removal ≈ 2×. 50 edits per tick ≈ 2–5 ms — acceptable. 1000 edits (TNT) ≈ 36–100 ms — **does not fit**, a per-tick limit on light updates with deferral is needed.

| Risk | Experiment | Threshold |
| --- | --- | --- |
| Remove-BFS not tested | Remove a torch, break a block under the sky | p95 ≤ 100 µs per edit |
| Section borders | A torch on a 3×3×3 border, compare with a full recompute | bitwise equality |
| Client/server determinism | Identical inputs → light hash | hashes match |

### 13.6 World storage and memory

**Decision:** 16³ sections with a palette without word crossing, a Single variant, **a hash index for palettes > 16 entries**. Disk and network: palette sections + zstd (3 — disk, 1 — network, 9 — backups). A dictionary — only after a test on real worlds.

Key numbers \[test\]: Single 56 B; surface 1640 B; dungeon 2124 B; worst case 6616 B. `get` ≈ 3.8 ns; `set` 6–7 ns with a palette ≤ 16, but 66.7 ns with a palette of 256 (linear search).

| Client component | 12 chunks | 64 chunks (LOD) |
| --- | --- | --- |
| Voxel sections (15,000) | 17 MiB \[test, assumed mix\]; worst case 95 MiB | same |
| Light | ≈ 15–30 MiB with Single (59 MiB without) | same |
| GPU meshes | ≈ 2–10 MiB | + LOD ≈ 4–20 MiB |
| Textures | ≈ 1.5 MiB (64 px packs ≈ 25 MiB) | same |
| 1080p framebuffers | ≈ 32 MiB | same |
| ECS, entities, UI, audio | ≈ 100–200 MiB | ≈ 100–200 MiB |
| WASM mods (10) | ≈ 100–200 MiB | same |
| Binary, driver, staging | ≈ 200–300 MiB | ≈ 200–300 MiB |
| **Total** \[estimate\] | **≈ 0.5–0.9 GiB** | **≈ 0.55–1.0 GiB** |

The 2 GB target is realistic; an internal budget of 1 GiB is proposed. **Server** \[estimate\]: 500 scattered players × 15,000 sections ≈ 8.9 GB of voxels alone. So the server keeps a simulation distance of 6–8 chunks and an LRU cache of compressed columns.

| Risk | Experiment | Threshold |
| --- | --- | --- |
| Real section mix | Bit-width histogram on a real world | ≤ 40 MiB per 15,000 sections |
| `set` on large palettes | Hash index | ≤ 10 ns |
| Real zstd ratios | Real columns, levels 1/3/9 | ≤ 50 µs per column |
| Client RSS | `dhat` + RSS after 10 min of flight | ≤ 1 GiB |

### 13.7 Network

**Decision:** one QUIC connection (quinn 0.11): datagrams for entity state (delta from the acknowledged snapshot, 20 Hz), priority streams (`set_priority`) for chunks by distance. Authoritative server + prediction + interpolation. Nothing was tested.

&#91;Estimate\] Join: 625 columns × 5–20 KB = 3–12 MB. Entities: 50 visible × 16 B × 20 Hz ≈ 128 kbit/s per player; × 500 ≈ 64 Mbit/s of node outbound traffic plus chunks; 10,000 datagrams/s.

| Risk | Experiment | Threshold |
| --- | --- | --- |
| quinn CPU at 500 connections | 500 bots, 20 Hz × 1 KB | ≤ 1 core per 10,000 packets/s |
| Head-of-line between streams | 2 % loss + RTT 100 ms (`tc netem`) | nearest 3×3 columns ≤ 1 s |
| Chunk packet size | See 13.6 | ≤ 20 KB per column |

### 13.8 Server region threads

**Decision:** a region = a separate bevy `World` owned by one worker per tick; stable global entity IDs (`NetId → Entity`), so merging/splitting is copying. The 25 ms target = p99 per region at 20 TPS. Fallback: one `World` per node with `par_iter` over a region component. Confidence low–medium.

&#91;Estimate\] 16 cores × 25 ms / 500 = 0.8 ms CPU per player per tick. Sufficient **if players are spread across at least 8 regions**. A crowd of 500 players in one place gives a single thread, and the target is not met. A separate "single region capacity" target and a hard limit with instances or a queue are needed. WASM: an instance per mod **per worker thread**, not per region (otherwise 0.6–3.2 GiB).

| Risk | Experiment | Threshold |
| --- | --- | --- |
| 500-player tick | 500 bots, 8 and 1 regions, 16 cores | p99 ≤ 25 ms on 8 regions; find the maximum on 1 region |
| Entity transfer between `World`s | Merge/split with 1000 entities | ≤ 5 µs per entity |
| Node memory | RSS after 500 bots over 10 km | ≤ 16 GB |

### 13.9 Sharding and entity handoff

**Decision:** a static grid of regions across nodes + a gateway (QUIC proxy), overlap zone ≥ AOI radius. Handoff with an entity ownership number and idempotent operations (consistent with the fix in section 8). Dynamic repartitioning — later. Confidence low: no tests.

&#91;Estimate\] 5000 / 500 = 10 nodes only with an even distribution. Border crossing ≈ once per 90 s per player ≈ 55 handoffs/s per cluster — a small load. Handoff target: ≤ 1 tick (50 ms), not "< 1 ms".

| Risk | Experiment | Threshold |
| --- | --- | --- |
| Duplicates/losses | Two nodes, RTT 100 ms, 1000 transitions | 0 duplicates, 0 losses, p99 ≤ 50 ms |
| Crowd at the border | 200 bots at a node border | tick of both ≤ 25 ms |
| Hot spot (2000 players) | Not solved by sharding | an instance policy is needed |

### 13.10 WASM modding

**Decision:** Component Model (WIT), but **coarse batched hooks**: one call per mod per event type per tick with `list<event>`. Epoch always; fuel off (only for replays/anti-cheat). `InstancePre` + pooling allocator. Fallback: core modules with a custom ABI (a 15–20× cheaper call, without WIT type safety).

| Measurement \[test, wasmtime 50.0.1\] | Value |
| --- | --- |
| Core: empty call | 17–30 ns |
| Core: 1000 updates separately / in one call | 18–35 / 0.9–2.9 ns per update |
| Guest → host | 3.7–4.9 ns |
| **Component: empty call** | **≈ 0.40 µs** |
| Component: 1000 separate calls | ≈ 0.5 µs per update |
| Component: one call with `list<u32>` of 4000 | ≈ 3.6 ns per update |
| Epoch / Fuel (recursion) | 0–9 % / +50–92 % |
| Instantiation (`InstancePre`) | 4.2 µs |

**New target:** ≤ 1 µs per batched hook call, ≤ 5 ns per event, a ban on per-voxel/per-entity calls across the component boundary. \[Estimate\] Without batches the hooks take ≈ 2 ms of the tick; with batches ≈ 0.1 ms.

| Risk | Experiment | Threshold |
| --- | --- | --- |
| Lifting `string`/`record` in a Rust guest | `list<record>` of 1000 elements | ≤ 20 ns per element |
| Mod cold start | `Component::new` for 1–5 MB, cache | ≤ 50 ms per mod with cache |
| Instance memory | 20 mods × 16 workers | ≤ 1 GB per node |
| Hung mod | Infinite loop | interrupted ≤ 1 ms after the deadline |

### 13.11 Rust level

- Build: LTO, `codegen-units = 1`, baseline target x86-64-v2, no `target-cpu=native` (v3 gave a gain within noise \[test\]). Hot kernels — via runtime dispatch (`multiversion`).
- Explicit SIMD — only when the profile shows a need; candidates — gather/masks and light BFS.
- `mimalloc` and PGO — only after an A/B on benchmarks (threshold: −10 % tick p99 for mimalloc, ≥ 10 % for PGO).
- Profiling from day one: Tracy, `dhat`, criterion in CI.

### 13.12 Summary table of targets

| Target | Status | Proposal |
| --- | --- | --- |
| 60 FPS iGPU/GTX 1050, 12 chunks | Not tested | Measure vertex throughput on iGPU first |
| 64+ chunks via LOD | Realistic \[estimate\] | Full data only ≤ 12–16 chunks |
| Client RAM ≤ 2 GB | Realistic \[test + estimate\] | Internal budget 1 GiB |
| Mesh ≤ 0.2 ms/chunk | On VM 0.27 ms | ≤ 0.3 ms `fast_mesh` / ≤ 0.5 ms pipeline per unit |
| Hook ≤ 1 µs | Achieved (0.40 µs) | ≤ 1 µs per batch + ≤ 5 ns per event |
| Tick ≤ 25 ms @ 500 | Not tested | Add region capacity; players across ≥ 8 regions |
| 5000+ players | Not tested | Only with an even distribution; hot spot policy |

### 13.13 "Do first" (stages 0–1)

1. Mesh unit + `fast_mesh`, incremental masks, skipping empty and solid units.
2. Fast gather from the palette (bulk unpacking): gather + masks ≤ 100 µs.
3. Palette without word crossing + Single + hash index: `set` ≤ 10 ns at any bit width.
4. 8 B quad, vertex pulling, draw per unit × direction; adapter capability probe; instanced attribute as a fallback path.
5. Vertex throughput bench on iGPU and GTX 1050 before the complex renderer.
6. Incremental lighting (add/remove BFS) with a per-tick limit: ≤ 100 µs p95 per edit.
7. WIT API with batched hooks; epoch on, fuel off; an instance per mod per worker.
8. Region = bevy `World` + stable global IDs; right away a bench of 500 bots on 1 and 8 regions.
9. One QUIC connection: datagrams for entities, priority streams for chunks.
10. Palette sections + zstd (3 — disk, 1 — network).
11. Tracy, `dhat`, criterion in CI with a fixed corpus of worlds and regression thresholds.
12. x86-64-v2 build, LTO; mimalloc and PGO — only after an A/B.

## 14. Decision priorities and review of old ideas

When an optimization conflicts with game feel, feel wins: the game must feel like Minecraft, not lag, have smart mobs and a long render distance. Of 11 old ideas, 5 are adopted, 4 partially, and 2 are deferred.

### Experience priorities

1. **Smoothness.** No frame spikes > 33 ms and no desyncs with the server. This matters more than peak FPS.
2. **Render distance.** Full data at least 8 chunks on the weakest profile (Low), 12 by default, 16 on High. Beyond that LOD up to 32–64 chunks. A distance shorter than 8 is not offered even in settings: shadows, effects, and LOD are reduced first.
3. **Smart mobs.** Full AI within ≈ 48 blocks of the player \[estimate, to be tuned by playtest\]. Pathfinding — asynchronous in the task pool, so smart AI does not eat tick time. Farther out, the AI decision rate drops, but a mob never "freezes" in a player's view.
4. **Minecraft feel.** Player physics, digging pace, 20 TPS block mechanics within the simulation distance — no compromises for optimization.

### Decision: 32³ mesh chunk

We mesh in 32³ chunks (2×2×2 sections): boundaries coincide with the data, and a block edit remeshes ≈ 7 times fewer voxels than with 62³. The cost is ≈ 7 times more units: at 12 chunks ≈ 1,900 units and up to ≈ 4,500 draw commands without indirect \[estimate\].

Compensation: the crate's quad encodes coordinates in 6 bits (0–63). So the quads of eight neighboring 32³ mesh chunks can be put into one 64³ batch with a +32 offset and drawn with one command. The number of draw commands becomes the same as with 62³ (≈ 600), while remeshing stays fine-grained. This is a design decision; from the stage 0 benchmark we expect: copying a batch after an edit ≤ 50 µs. On Vulkan/DX12 with `multi_draw_indirect` batches are not needed.

### Review of old ideas

| # | Idea | Verdict | How we adopt it | What to weigh |
| --- | --- | --- | --- | --- |
| 1 | LOD is downloaded from the server | Adopt (already in the plan) | The server sends LOD sections, the client caches them on disk | The client does not generate LOD itself: it would need the world seed, which opens up ore-finding cheats. Until LOD arrives — fog |
| 2 | Start with a complex way, then a better one; the old one is turned off and unloaded | Adopt as a principle | The fallback path works until there is something better (cache, server data, GPU features), then its data is freed | What needs unloading is data, assets, and WASM modules. The engine code itself is tens of MB, with almost no gain from unloading |
| 3 | At start only the base code; the rest by game stage (bosses, dungeons, items) | Partially | Heavy assets (models, textures, sounds) load lazily, by proximity and zone (see no. 11) | Not by player progress: another player may bring an item or lead a late-stage mob. Block and item registries are always full (kilobytes, needed for synchronization). Preloading in advance + a placeholder, otherwise a freeze |
| 4 | Generation by speed and height; do not generate the top underground; mobs in the air with AI off | Partially | A load queue with priorities: movement direction, visibility, height. High up — more LOD; underground — cave culling for rendering | Underground the top is only not drawn, the data is not discarded. Mobs in the air without AI — no: that is literally "dumb mobs". Instead, spawn only in a valid radius, as in Minecraft, and simulation LOD (no. 5) |
| 5 | Farther from the player — a smaller texture and slower simulation | Adopt | Textures — mipmaps and LOD (the GPU does it itself). Simulation — the entity AI rate decreases with distance | Block mechanics (farms, contraptions) within the simulation distance always at 20 TPS, otherwise farms break. A mod can mark a system "every tick" |
| 6 | Rewrite the core for the most popular server; one core for the maximum or evenly; a separate core for mods | Partially | Choose a target mode (e.g. SMP survival) and run bot benchmarks on exactly that. High-frequency CPU; the heaviest regions — on the fastest cores (pinning). Network, IO, and generation — on separate cores | The tick of a single dense region (a crowd, large contraptions, block update chains) splits poorly: core frequency decides here. A separate core for mods — no: every hook would become cross-thread synchronization and everything would slow down. Mods run in their region's tick with a budget |
| 7 | Buy our own servers | Later | At the start rent dedicated bare-metal servers (not cloud VMs). Buy and place in a data center when the load is stable (stage 6+) | Buying before there are players is frozen money and the wrong hardware. DDoS protection is mandatory |
| 8 | Optimization for our own hardware | Later | At stage 6: CPU pinning, huge pages, NUMA, NVMe, 10 Gbit, Linux kernel tuning. Before that — a regular x86-64-v2 build | The engine must not be tied to one server model |
| 9 | Fast start: the game opens right in the launcher window in a few seconds | Adopt | Target ≤ 5 s from button to menu: shader cache, pre-converted assets, mmap of registries, lazy loading. Instead of a Tauri launcher and a separate game: the launcher becomes the main menu of the game itself, and updates are done by a tiny separate updater | Changes section 7: login, mods, and catalog move into the game. The updater stays separate because a running binary cannot replace itself |
| 10 | The server logs everyone's actions; people can be hidden together with their buildings | Partially | An action log (who placed or broke what) — for griefing rollback and moderation. Hiding a player's avatar, chat, and voice — yes | Hiding buildings in a shared world is not possible: blocks are physical, and a hidden block becomes an invisible wall (desync with mobs, water, physics). Only possible in separate modes (creative plots). The log also takes disk — compression and a retention period are needed |
| 11 | World zones with biomes and difficulty; spawn and logic are computed only where they can occur | Adopt | "Game zones" with their own spawn, block, and dungeon tables: the spawn system does not check foreign lists, zone assets load on approach | Game zone ≠ the region from section 3: threading regions are dynamic and depend on players, zones are a static map layout. A mob can cross a zone border, so its behavior works everywhere — only spawning is restricted |

### What this changes in the plan

- **Section 7 (Launcher):** decided — an in-game menu + a small updater (no. 9); we do not use Tauri.
- **Stage 1:** add a load queue with priorities (no. 4), lazy asset loading (no. 3), and 2×2×2 mesh batches; exit criterion — launch to menu ≤ 5 s (no. 9).
- **Stage 2:** entity simulation LOD and asynchronous pathfinding (no. 5, priority 3).
- **Stage 4:** game zones (no. 11) and an action log with rollback (no. 10).
- **Stage 6:** own hardware and tuning for it (no. 7–8), pinning of heavy regions (no. 6).
