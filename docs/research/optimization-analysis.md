## 13. Optimization analysis: recommendations

**Main conclusion.** The client memory target (≤ 2 GB) and the mod hook overhead target (≤ 1 µs) are realistic with margin, while the 0.2 ms meshing budget is achievable only for `fast_mesh` with maintained masks and only if a "chunk" is defined as a 62³ mesh unit. The real bottlenecks are not meshing or WASM, but data collection into the mesh unit (gather/masks ≈ 0.35 ms), incremental lighting, server memory at 500 players, and vertex throughput on an iGPU. The server targets (25 ms @ 500 players, 5000+ with sharding) are not yet backed by any test and have the status of a hypothesis.

### 13.0 How to read this section

- Sources: `optimization_tests.md` (benchmarks on a noisy VM: 2 vCPU Xeon 2.1 GHz, error ±30 %), `optimization_search.md` (web research), raw logs in `/home/claude/bench/results/`.
- Additionally, I ran `meshing` with `-C target-cpu=x86-64-v3` (2 runs against 2 baseline). Result: `fast_mesh` 262–291 µs vs 269–302 µs baseline. The difference is within noise.
- Also verified on docs.rs: wgpu 30.0.2 has `DownlevelFlags::VERTEX_STORAGE` ("storage buffers in vertex shaders") and `INDIRECT_EXECUTION` (absent on WebGL2, GLES 3.0, Apple1/2). quinn 0.11.12 has `SendStream::set_priority`.
- Tags: **[test]** — measured; **[source]** — a cited source with a URL; **[estimate]** — my arithmetic or assumption, not measured.
- Confidence: high / medium / low.

### 13.1 Where research and tests diverge

| Topic | Research | Tests | Why I trust it |
|---|---|---|---|
| Meshing time | 50–200 µs (Ryzen 3800X, no transparency); ~600 µs with transparency (Xeon E5) | `mesh()` 1.1 ms; `fast_mesh` 0.27 ms (VM, 8 % transparent) | The tests — for **relative** conclusions (`fast_mesh` is 4× faster than `mesh`). The research's absolute numbers are plausible for a faster CPU, but for an old crate version. |
| Crate version | 0.3.6 | 0.6.4 (has `fast_mesh`, `mesh_lod`) | The tests (Cargo.lock). |
| Epoch | ~10 % slowdown (Wasmtime documentation) | 0–9 % | Consistent. |
| Fuel | "more expensive than epoch", no numbers | +50–92 % on `fib(35)`, ~0–2 % on a tight loop | The tests: now there is a number. |
| Palette packing | "indices may cross a long" (wiki) | No crossing, as in MC 1.16+ | The test variant: simpler and faster access. The wiki describes the old format (to my knowledge, not verified). |
| zstd for regions | Linear: −50–60 % size (no methodology) | Synthetic data gave unrealistic 19–184× | Neither for absolute numbers. From the tests I take only the relative: levels, dictionary, speed. |

### 13.2 Meshing

**Decision:** `binary-greedy-meshing` 0.6.4, `fast_mesh` mode. Mesh unit 62³ padded to 64³. The opaque/transparent masks are maintained incrementally. Fallback: our own port of cgerikj v2 for a different unit size.

| Mesher | Median | p95 | Quads/unit | Bytes (8 B/quad) |
|---|---|---|---|---|
| `mesh()` (BTreeSet) | 1083–1178 µs | 1540–1990 µs | 4962 | 39 KB |
| `fast_mesh` (precomputed masks) | 269–278 µs | 457–556 µs | 4962 | 39 KB |
| naive (table) | 898 µs | 1425 µs | 11762 | 92 KB |
| building masks from scratch | 341–366 µs | 550–636 µs | — | — |
| empty / solid unit, `fast_mesh` | 52 µs | ~66 µs | 0 | 0 |

Source: [test]. Confidence: **high** for choosing `fast_mesh`; **medium** for the absolute time on a desktop.

**What "chunk" means in the 0.2 ms target.** This must be defined explicitly:
- A 62³ unit = 238,328 voxels ≈ 58 sections of 16³ ≈ 2.4 columns of 16×16×384. [estimate]
- `fast_mesh` ≈ 270 µs / 238,328 ≈ **1.13 ns/voxel** on the VM. [test + arithmetic]
- For a 16×16×384 column (98,304 voxels) that is ≈ 0.11 ms under linear scaling. But the crate works only with 62³, so this is a hypothetical number. [estimate]

**Target revision.** Instead of "≤ 0.2 ms/chunk" I propose:
- `fast_mesh` ≤ 0.3 ms p50 and ≤ 0.6 ms p95 per 62³ unit on a reference 4-core CPU;
- **the full pipeline** (gather from sections + masks + mesh) ≤ 0.5 ms p50 per unit;
- skipping empty and solid units by section metadata, without calling the mesher (even 52 µs for an empty unit is wasted work).

**Load arithmetic** [estimate, on VM numbers, full pipeline 0.27 + 0.35 ≈ 0.62 ms]:
- Radius of 12 chunks = 400 × 400 blocks. Horizontally (400/62)² ≈ 42 unit columns; vertically 384/62 ≈ 6.2. Total ≈ **260 units**.
- 2 meshing workers (a 4-core laptop): 2 × 16.7 ms / 0.62 ms ≈ **53 units per frame**. A full rebuild of 260 units ≈ 5 frames.
- Changing a single block on a border affects the neighbors' padding: up to 4 units (up to 8 with AO on diagonals). 4 × 0.62 = 2.5 ms on one thread. Fits in the same frame.
- Flying at 30 blocks/s [assumption]: 30 × 400 × 384 = 4.6 million voxels/s ≈ 19 units/s. That is a small thing.

Conclusion: meshing CPU time is not the bottleneck. The bottleneck is gather and masks.

**Risks and cheapest experiments:**

| Risk | Experiment | Threshold |
|---|---|---|
| Gather from palette sections into 64³ via `get()` = 262,144 × 3.8 ns ≈ **1 ms** [estimate from the palette test] | Bulk unpacking of a section (word-by-word) into a 64³ buffer and building masks directly from the palette (Single sections = memset) | gather + masks ≤ 100 µs p50 |
| Incremental masks are not implemented | Update mask bits on `set_block`; 10,000 random edits | ≤ 20 ns per edit |
| The 62 grid is not a multiple of 16 (LCM = 496) | Check the gather index math at section boundaries (property tests) | 0 mismatches with the reference naive mesher |
| AO not measured | Add AO bits to the quad, compare time | ≤ +30 % to `fast_mesh` |
| Desktop CPU not tested | The same bench on a Ryzen/Intel 4+ GHz | `fast_mesh` ≤ 0.2 ms p50 |

### 13.3 GPU rendering and a fallback path for weak GPUs

**Decision (stage 0–1):** 8 B quads, vertex pulling from a shared storage buffer (slab allocator). One draw per unit per face direction per pass (opaque, then transparent). CPU frustum culling. Discarding face directions facing away from the camera. The vertex is computed from `vertex_index` (quad = `vertex_index / 6`), so `first_instance` is not needed.

**Fallback path:** if there is no `DownlevelFlags::VERTEX_STORAGE`, the same 8 B are supplied as a per-instance attribute (`Uint32x2`, 4 vertices per instance). The shader logic is the same.

**Next option (stage 2+):** compute culling that writes indirect arguments, + `multi_draw_indirect(_count)` where `INDIRECT_EXECUTION` / `MULTI_DRAW_INDIRECT_COUNT` are available. Hi-Z (as in Aokana) — only after measurements.

Confidence: **medium**. The GPU was not tested at all.

Arguments:
- The storage buffer binding limit is 128 MiB [source: docs.rs wgpu Limits]. That is 16.7 million quads per binding. The need estimate is below. The limit does not interfere.
- `INDIRECT_EXECUTION` is absent on GLES 3.0/WebGL2 [source: docs.rs DownlevelFlags]. So the GPU-driven path cannot be the only one.
- Large 62³ units yield few draw calls: ≈ 260 units, of which non-empty ≈ 100 [estimate]. 100 units × ≤ 3 visible directions × 2 passes ≈ **600 draws** without indirect. For DX12/Vulkan that is acceptable [estimate, not measured].

**Vertex arithmetic** [estimate]:
- ≈ 100 non-empty units × 4962 quads ≈ 0.5 million quads = 3 million vertices (6 per quad) per frame.
- At 60 FPS that is 180 million vertices/s. Discarding face directions reduces this by about half (≤ 3 of 6 directions visible).
- For old iGPUs this may be borderline. **This is the main unmeasured rendering risk.**

| Risk | Experiment | Threshold |
|---|---|---|
| Vertex throughput on an iGPU | wgpu scene of 0.5 million quads, vertex pulling vs instanced vs indexed buffer; Intel UHD + GTX 1050 | ≤ 8 ms GPU time per frame (half of the 60 FPS budget) |
| Instanced quads (4 vertices per instance) are slow on some GPUs | The same test, compare with `draw(0..6n)` | instanced no worse than 1.3× |
| Feature support | Dump `adapter.features()`, `limits()`, `downlevel_capabilities()` on target GPUs (Vulkan and DX12) | `VERTEX_STORAGE` on both; otherwise the fallback path by default |
| Draw call cost | 600 vs 2000 draws on an iGPU | command recording CPU time ≤ 1 ms |
| Cave culling | A visibility graph between unit faces; count units discarded underground | ≥ 50 % of units discarded in a cave |

### 13.4 LOD (up to 64 chunks)

**Decision:** LOD as voxel downsampling 2× per level. Each 62³ LOD unit covers 62·2^L blocks and is meshed by the same mesher (`mesh_lod` exists in 0.6.4 but was not tested). The server sends the base LOD level (the Distant Horizons pattern [source]). Seams — skirts. LOD lighting — sky only, baked into the vertex.

**Fallback:** client-side LOD generation from its own data without a server stream.

**Rejected for stage 0–1:** Aokana SVDAG + ray marching [source: arXiv 2505.02017]. Reasons: no runtime editing, no transparency, needs compute (does not work on the fallback path), Unity only. Transvoxel is for marching cubes, not for a block world.

Confidence: **medium** (choice), **low** (numbers).

**Arithmetic** [estimate; LOD0 ≈ 39 KB per 62×62 = 3844 m² ≈ 10 B/m², quads at level L are 4^L fewer]:

| Ring | Area, m² | Divisor | Mesh, MB | Units (horiz. × vert.) |
|---|---|---|---|---|
| L0: 0–12 chunks | 160,000 | 1 | 1.6 | 42 × 6.2 ≈ 260 |
| L1: 12–24 | 430,000 | 4 | 1.1 | 28 × 3.1 ≈ 87 |
| L2: 24–48 | 1,770,000 | 16 | 1.1 | 29 × 1.5 ≈ 44 |
| L3: 48–64 | 1,830,000 | 64 | 0.3 | 7 × 1 ≈ 7 |
| **Total** | 4,190,000 | — | **≈ 4 (×5 margin = 20)** | **≈ 400** |

Conclusion: 64 chunks via LOD is cheap both in memory and in draw count. But **full data for 64 chunks is impossible**: 129² × 24 = 399,384 sections × 4 KB of light = 1.52 GiB for light alone. Full voxel data only within a radius of 12–16.

| Risk | Experiment | Threshold |
|---|---|---|
| LOD update after edits (no source describes it) | Dirty regions: edit → recompute ancestors L1..L3 | ≤ 1 ms per edit total across levels |
| Cracks at level boundaries | Screenshot test with skirts | 0 visible holes from 8 viewpoints |
| `mesh_lod` behaves differently than expected | Bench `mesh_lod` vs downsampling + `fast_mesh` | Time ≤ `fast_mesh`; quads ≤ 1/3 of L0 |

### 13.5 Lighting

**Decision:** per-section nibbles (block + sky, 4 KB per section), a Single variant for uniform sections (all sky 15 / all dark 0). **Incremental add/remove BFS queues** for edits. Two-pass light in 2×2 groups (the dktapps specification [source]) — for initial generation. Light for rendering is computed by the client. The server computes light only where it is needed for gameplay (mob spawn), lazily.

**Fallback:** server light sent to the client (as in MC). More expensive for server CPU and traffic.

Confidence: **high** that incremental light is needed; **medium** about client-side computation.

| Scenario [test] | 16³ | 48³ (27 sections) |
|---|---|---|
| 1 torch, open air | 24 µs | 36 µs |
| 8 torches/section, open | 46 µs | 1415 µs |
| 26–45 % opaque, 8 torches | 25–27 µs | 2062–2181 µs |
| Sky (heightmap + roofs) | not indicative | 554 µs |

Cost: 10–13 ns per pop in open space, 27–35 ns with obstacles [test].

**Arithmetic** [estimate]:
- One new torch incrementally ≈ 3303 pops × 11–30 ns ≈ **36–100 µs**. Removal ≈ 2× (two queues).
- Server, 50 light edits per tick: 2–5 ms, spread across region threads. Acceptable.
- 1000 edits per tick (TNT, mass building): 36–100 ms. **Does not fit** → a per-tick limit on light updates with deferral is needed.
- Client initial sky: 554 µs / 27 ≈ 20 µs/section × 15,000 = 0.3 s on one thread. On a pool during loading — acceptable.

| Risk | Experiment | Threshold |
|---|---|---|
| Incremental light removal was not tested | Remove-BFS: remove a torch, break a block under the sky | p95 ≤ 100 µs per edit |
| Section boundaries in BFS | A 3×3×3 test with a torch on a border; compare with a full recompute | Bit-for-bit equality |
| Client/server determinism (if the server also computes) | Same inputs → light hash | Hashes match |
| Smooth lighting / AO in the shader | Not researched; measure after the baseline renderer | — |

### 13.6 World storage and memory

**Decision:** 16³ sections with a palette without word crossing (1→2→…→8 bits, then direct 12 bits), a Single variant, **a reverse index (hash) for palettes > 16 entries**. Disk and network: palette-packed sections + zstd. Level 3 for saving, 1 for network, 9 for backups. A dictionary — only for single sections on the network and only after a test on real worlds. One file per region (the Linear pattern [source, no methodology]).

**Fallback:** rkyv + mmap for fast region loading (not verified, search only).

Confidence: **high** (palette), **medium** (zstd levels).

Key numbers [test]:
- Size: Single 56 B; surface (8 ids, 3 bit) 1640 B; dungeon (10 ids, 4 bit) 2124 B; worst case 6616 B (81 % of 8192).
- `get` ≈ 3.8 ns (vs 1.0 ns for a flat array); `set` 6–7 ns with a palette ≤ 16; **66.7 ns with a palette of 256** (linear search).
- zstd: palette-packed columns vs raw u16 at level 3 — 1081 B vs 1361 B, compression 26 µs vs 73 µs per column. The palette wins both in size and speed (relative; the data is synthetic).
- Level 9 gives −13–30 % size for 4–6× compression time; decompression is equally fast.
- The dictionary helped only on a single 8 KB section (+3–5 %). On columns it is neutral or harmful (144× → 100×).

**Client memory budget** (everything except rows tagged [test] is an [estimate]):

| Component | 12 chunks | 64 chunks (via LOD) |
|---|---|---|
| Voxel sections (15,000) | 17 MiB [test, assumed mix]; worst case 95 MiB | The same (full data only ≤ 12) |
| Light, 4 KB/section | 58.6 MiB uncompressed [arithmetic from the test]; ≈ 15–30 MiB with Single | The same |
| CPU mesh copies | < 10 MiB [arithmetic from the test]; free after upload to GPU | — |
| GPU meshes L0 | ≈ 2–10 MiB | + LOD ≈ 4–20 MiB |
| Textures (16 px atlas, ~1000 blocks, mips) | ≈ 1.5 MiB; 64 px packs ≈ 25 MiB | The same |
| 1080p framebuffers (3 color + depth) | ≈ 32 MiB (on an iGPU — from shared RAM) | The same |
| ECS, entities, UI, audio | ≈ 100–200 MiB | ≈ 100–200 MiB |
| WASM mods on the client (10 × 10–20 MiB) | ≈ 100–200 MiB | The same |
| Binary, driver, wgpu staging, fragmentation | ≈ 200–300 MiB | ≈ 200–300 MiB |
| **Total** | **≈ 0.5–0.9 GiB** | **≈ 0.55–1.0 GiB** |

Conclusion: the 2 GB target is **realistic** with margin. I propose an internal budget of 1 GiB at 12 chunks to leave room for mods and packs.

**The server is a separate problem** [estimate]: 500 players scattered without overlap × 15,000 sections = 7.5 million sections × 1.19 KB ≈ **8.9 GB of voxels alone** (with 4 KB light — another +30 GB). So the server does not keep light for all sections, uses a simulation distance of 6–8 chunks, and an LRU cache of compressed columns for the rest of the view radius.

| Risk | Experiment | Threshold |
|---|---|---|
| The real section mix is unknown (17 MiB is an assumption) | Generate/import a real world, histogram of section bit widths | ≤ 40 MiB per 15,000 sections |
| `set` 66 ns on large palettes | Add a `HashMap<u16,u8>`/sorted index for > 16 entries | ≤ 10 ns per `set` |
| Real zstd ratios | Compress real columns (with light and block entities) at levels 1/3/9, with and without a dictionary | Choose a level giving ≤ 50 µs/column compression |
| Actual client RSS | `dhat` + RSS after 10 min of flight at 12 chunks | ≤ 1 GiB |

### 13.7 Network

**Decision:** one QUIC connection (quinn 0.11). Unreliable datagrams — for entity state (delta from the last acknowledged snapshot, 20 Hz). Reliable prioritized streams (`SendStream::set_priority` [verified on docs.rs]) — for chunks sorted by distance to the player. Chunks go as palette sections + zstd level 1. Model: authoritative server + client prediction + interpolation (Photon, Gambetta [source]).

**Fallback:** `bevy_quinnet` as a ready wrapper (not verified).

Confidence: **medium**. Nothing about the network was tested.

**Arithmetic** [estimate]:
- Player login: 625 columns × 5–20 KB (real size unknown; a synthetic 1 KB is unrealistic) = 3–12 MB. Over 10 s that is 2.5–10 Mbit/s per player.
- Entities: 50 visible × 16 B × 20 Hz = 16 KB/s ≈ 128 kbit/s per player. × 500 = **≈ 64 Mbit/s** of node outbound traffic plus chunks.
- Packets: 500 players × 20 Hz = 10,000 datagrams/s per node.

| Risk | Experiment | Threshold |
|---|---|---|
| quinn CPU at 500 connections | 500 bot clients, 20 Hz × 1 KB datagrams, one server process | ≤ 1 core per 10,000 packets/s |
| Head-of-line blocking between chunk streams | 2 % loss + RTT 100 ms (`tc netem`); time to visibility of the nearest chunks | Nearest 3×3 columns ≤ 1 s |
| Size of real chunk packets | See 13.6 | ≤ 20 KB/column |

### 13.8 Server region threads (Folia style)

**Decision:** a region = a separate bevy `World` owned by a single worker per tick. Stable global entity IDs (our own `NetId → Entity`) so that merging/splitting regions is a copy. A global region — for chat, commands, tables. I interpret the 25 ms target as ≤ 25 ms p99 per region at 20 TPS (tick budget 50 ms → 50 % margin).

**Fallback:** one `World` per node with `par_iter` over a region component. Simpler, but isolates worse and complicates parallel mutations across systems.

Confidence: **low–medium**. Not tested.

Arguments and arithmetic:
- Folia: ≥ 16 cores preferred, a test server of ≈ 330 players at peak [source: Folia README]. Our 500 is 1.5× more ambitious, though it is Java vs Rust.
- Budget [estimate]: 16 cores × 25 ms / 500 = 0.8 ms CPU per player per tick (before overhead). Sufficient, **if players are spread across at least 8 regions**. A crowd of 500 players in one place yields one region and one thread, and the target is not met.
- So a "single region capacity" target and a hard limit (with instances/queue) are needed — the number is unknown.
- WASM on regions: a `Store` cannot be shared between threads. An instance per mod per region = 20 mods × 16 regions × 2–10 MiB ≈ 0.6–3.2 GiB [estimate]. So an instance per mod **per worker thread**, and mod state on the host side (ECS resources).

| Risk | Experiment | Threshold |
|---|---|---|
| Tick with 500 players | bevy_ecs: 500 bots (movement, inventory, 10 entities nearby), 8 and 1 region, 16 cores | p99 ≤ 25 ms on 8 regions; find the maximum players per 1 region |
| Moving entities between `World`s | Merging/splitting regions with 1000 entities | ≤ 5 µs per entity |
| Node memory | RSS after 500 bots scattered over 10 km | ≤ 16 GB |

### 13.9 Sharding across nodes and entity handoff

**Decision:** a static grid of regions across nodes at the start + a gateway (QUIC proxy) holding the client connection so that handoff does not require reconnecting. An overlap zone (ghost zone) of width ≥ AOI radius. A two-phase handoff (prepare → ghost → commit) with entity versions and idempotent operations (the Photon pattern [source]). Dynamic re-partitioning — later.

**Fallback:** a MultiPaper-like chunk ownership master [source]. It is in public beta and does not describe moving entities between servers.

Confidence: **low**. There are no tests; Photon's "sub-millisecond handoff" is a claim without methodology.

**Arithmetic** [estimate]:
- 5000 players / 500 = 10 nodes **only with an even distribution**.
- A node region of 512 blocks, running ≈ 5.6 blocks/s → a crossing every ≈ 90 s per player → ≈ 55 handoffs/s per cluster. The load is small.
- Handoff target: ≤ 1 tick (50 ms) and unnoticeable to the player, not "< 1 ms".

| Risk | Experiment | Threshold |
|---|---|---|
| Duplicates/losses on handoff | Two bevy_ecs nodes + quinn, RTT 100 ms, 1000 round-trip crossings | 0 duplicates, 0 losses, p99 ≤ 50 ms |
| Crowding on a node border | 200 bots standing on the border | Tick of both nodes ≤ 25 ms |
| Hot spot (an event with 2000 players) | Not solved by sharding for now → an instance policy is needed | A design-level decision |

### 13.10 WASM modding (wasmtime, Component Model)

**Decision:** Component Model (WIT) for the mod API, but **coarse batched hooks**: one call per mod per event type per tick, with `list<event>`. Epoch interruption always on. Fuel off (only for deterministic replays/anti-cheat). `InstancePre` + pooling allocator. An instance per mod per worker thread.

**Fallback:** core modules with a custom ABI via shared memory. A 15–20× cheaper call, but without type safety and WIT versioning.

Confidence: **high** (call numbers, epoch vs fuel), **medium** (Rust guest).

| Measurement [test, wasmtime 50.0.1] | Value |
|---|---|
| Core: empty call | 17–30 ns |
| Core: 1000 updates by separate calls | 18–35 ns/update |
| Core: 1000 updates in one call | 0.9–2.9 ns/update |
| Guest → host (import) | 3.7–4.9 ns |
| **Component: empty call** | **≈ 0.40 µs** |
| Component: 1000 separate calls | 423–523 µs (≈ 0.5 µs/update) |
| Component: one call with `list<u32>` of 4000 | 2.1–3.8 µs (≈ 3.6 ns/update) |
| Epoch | 0–9 % |
| Fuel, tight loop / recursion | ~0–2 % / **+50–92 %** |
| Instantiation (`InstancePre`) | 4.2 µs |

**Target revision.** "≤ 1 µs per hook" is met (0.40 µs), but with only a 2.5× margin on a slow VM. The new target:
- ≤ 1 µs per hook **dispatch** (one batched call);
- ≤ 5 ns per **event** inside a batch;
- a ban on per-voxel/per-entity calls across the component boundary in the API.

**Tick arithmetic** [estimate]:
- Without batches: 500 players × 10 hooks = 5000 calls × 0.4 µs = **2 ms** (8 % of the 25 ms budget) — just for crossing the boundary, with no mod work.
- With batches: 20 mods × 10 event types = 200 calls × 0.4 µs = 80 µs + 5000 events × ~4 ns = 20 µs → **≈ 0.1 ms**. 20× cheaper.

| Risk | Experiment | Threshold |
|---|---|---|
| Lifting `string`/`record`/`resource` in a Rust guest (cargo-component + wit-bindgen) not measured | Rust guest with `list<record{id:u32,pos:vec3,name:string}>` of 1000 elements | ≤ 20 ns per element |
| Cold start of real mods | `Component::new` for a 1–5 MB mod; cache of compiled modules | With cache ≤ 50 ms per mod |
| Instance memory | 20 mods × 16 workers, RSS | ≤ 1 GB per node |
| Async yield on epoch | A mod with an infinite loop must not break the tick | Interrupted ≤ 1 ms after the deadline |

### 13.11 Rust level: allocators, SIMD, PGO, profiling

**Decision:**
- Build: `lto = "fat"` or `"thin"`, `codegen-units = 1` for release. Baseline target x86-64-v2. **Not** `target-cpu=native` for distribution. My v3 vs baseline test: `fast_mesh` 262–291 µs vs 269–302 µs — within noise [test]. Hot kernels if needed — via runtime dispatch (the `multiversion` crate).
- SIMD: do not add explicit SIMD until the profile shows a need. Binary meshing is already bit-parallel [source + test]. Candidates — gather/masks and light BFS.
- Allocator: `mimalloc` behind a feature flag on the server, enable only after A/B. The numbers in the research are unverified.
- PGO: stage 2+, after the test workload stabilizes. The source (rustc book) promises only "should be faster", without numbers.
- Profiling from day one: Tracy (`tracy-client`) for frame and tick zones, `dhat` for memory, criterion for microbenchmarks in CI.

Confidence: **medium**.

| Risk | Experiment | Acceptance threshold |
|---|---|---|
| mimalloc | Server tick bench (13.8) with system vs mimalloc | tick p99 −10 % or more |
| PGO | Meshing + tick, regular build vs PGO | ≥ 10 % gain, otherwise do not complicate CI |
| Measurement noise | Repeat the benches on bare metal with a fixed frequency | Spread ≤ 5 % between runs |

### 13.12 Summary table of targets

| Target | Status | Proposal |
|---|---|---|
| 60 FPS iGPU/GTX 1050, 12 chunks | Not tested (no GPU) | Keep; measure vertex throughput on an iGPU first |
| 64+ chunks via LOD | Realistic for memory and draws [estimate] | Keep; full data only ≤ 12–16 chunks |
| Client RAM ≤ 2 GB | Realistic [test + estimate] | Internal budget 1 GiB at 12 chunks |
| Mesh ≤ 0.2 ms/chunk | Undefined; 0.27 ms on the VM | ≤ 0.3 ms `fast_mesh` / ≤ 0.5 ms full pipeline per 62³ unit |
| Hook ≤ 1 µs | Met (0.40 µs component) | ≤ 1 µs per batched call + ≤ 5 ns per event |
| Tick ≤ 25 ms @ 500 | Not tested | Add single-region capacity; players across ≥ 8 regions |
| 5000+ players | Not tested | Only with an even distribution; a hot-spot policy is needed |

### 13.13 "Do first" (stages 0–1)

1. **62³ mesh unit + `fast_mesh`.** 16³ storage sections. Maintain opaque/transparent masks on every edit. Skip empty and solid units.
2. **Fast gather from the palette.** Bulk unpacking instead of `get()`. Target: gather + masks ≤ 100 µs. This is the biggest known reserve in the pipeline.
3. **Palette without word crossing + Single + hash index for > 16 entries.** Target: `set` ≤ 10 ns at any bit width.
4. **8 B quad, vertex pulling, draw per unit × direction.** An adapter capability probe at startup. An instanced attribute as the fallback path without `VERTEX_STORAGE`.
5. **Vertex throughput bench on an iGPU and GTX 1050** before writing a complex renderer. This decides whether indirect/Hi-Z is needed.
6. **Incremental lighting (add/remove BFS)** with a per-tick update limit. Target: ≤ 100 µs p95 per edit. The client computes light.
7. **WIT API with batched hooks** (`list<event>`). Epoch on, fuel off. An instance per mod per worker. A ban on per-voxel calls in the API.
8. **Region = bevy `World` + stable global IDs.** Immediately a bench of 500 bots on 1 and 8 regions.
9. **Network: one QUIC connection.** Datagrams for entities with deltas, prioritized streams for chunks by distance.
10. **On-disk and network format:** palette sections + zstd (3 for disk, 1 for network). A dictionary — only after a test on real worlds.
11. **Tools from day one:** Tracy, `dhat`, a criterion suite in CI with a fixed world corpus and regression thresholds.
12. **Build:** x86-64-v2, LTO, `codegen-units = 1`. mimalloc and PGO — only after A/B on the benches from items 8 and 11.
