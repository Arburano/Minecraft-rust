# Voxel Engine Optimization Research (Rust, bevy_ecs, wgpu, QUIC, wasmtime)

Scope: concrete techniques, libraries, papers and projects for a Minecraft-like Rust voxel engine.
Targets from the brief: 60 FPS at 12-chunk render distance on iGPU / GTX 1050, client RAM <= 2 GB, server tick <= 25 ms at 500 players per node, 5000+ players across shards.

Conventions:
- "URL opened" = a page I actually fetched with WebFetch. Items marked "search only" came from search result listings and were not opened.
- "not stated" = the opened page did not give the figure. Nothing below is estimated.
- "unverified" = claim or number came only from a summary, a search snippet, or a project's own marketing.
- Test ideas are my suggestions for benchmarking; they were not checked against the crates' current APIs.

---

## Summary of the most useful findings

1. Binary greedy meshing (cgerikj, Rust port in `binary-greedy-meshing`) is the best-documented meshing approach found. It reports 50-200 us per chunk, and 8-byte packed quads with vertex pulling.
2. Aokana (arXiv 2505.02017) is the strongest recent paper on GPU-driven voxel LOD with SVDAG + Hi-Z. Its memory numbers are good but it does not support runtime voxel edits or transparency, so it fits a static-world LOD layer only.
3. wgpu limits matter: storage buffer binding is capped at 128 MiB by default, and the WebGL2 downlevel preset disables storage buffers and compute entirely. A GTX 1050 can use the Vulkan/DX12 path, but the iGPU and any WebGL target need a fallback path.
4. Epoch interruption is the cheap choice for wasm plugins (about 10% slowdown per Wasmtime docs); fuel is deterministic but costs more and has no number. Veloren's plugin page does not confirm Wasmtime, so do not treat Veloren as a Wasmtime reference.
5. Folia's model (regions that tick in parallel and do not share data) is the reference for region threading. Its own docs list about 16 cores and about 330 peak players on a test server. The MultiPaper design (a Master that owns chunks) is an alternative for cross-process sharding, but it is in public beta.
6. Photon's MMO backend article gives a concrete sharding design: dynamic partitioning, an overlap zone between neighbouring nodes, exactly-once entity operations, a 30 Hz simulation tick per node, and a sub-millisecond handoff claim.
7. Hytale: only secondary sources were available. Its server is said to be Java and to run plugins as .jar files, but its internal architecture is not confirmed by Hypixel.

---

## 1. Chunk meshing

### 1.1 Binary greedy meshing (cgerikj/binary-greedy-meshing)
- Description: Builds a 64x64 array of 64-bit occupancy masks, computes visible faces 64 at a time with bitwise ops, then greedy-merges quads with bit operations. Each quad is 8 bytes (6-bit x, y, z, width, height plus 8-bit voxel type), rendered with vertex pulling.
- Numbers: Meshing time per chunk typically 50-200 us; screenshot chunks averaged 74 us single-threaded and 108 us in a thread pool on a Ryzen 3800X (per the page). The page says v2 is "several times faster" than v1, with no exact speedup stated.
- Maturity: MIT license, active C/C++ reference by the author; used as a base for Rust ports.
- Engine fit: `chunk meshing` worker jobs on rayon; mesh output feeds the vertex-pulling buffer in the wgpu renderer.
- URL opened: https://github.com/cgerikj/binary-greedy-meshing
- Test: Criterion bench in the workspace. Generate random and real-terrain 62^3 chunks, measure us/chunk single-threaded and under rayon, and also measure vertices and bytes per chunk.

### 1.2 `binary-greedy-meshing` Rust crate (with transparency support)
- Description: Rust port of 1.1. Chunks must be 62^3 with 1-voxel padding (64^3 total); other sizes are not supported. Transparent block IDs are passed in a `BTreeSet`. Output is `[Vec<u64>; 6]`, one vector per face direction. Usable via instancing or converted to an index mesh.
- Numbers: Meshing with transparency about 600 us on an Intel Xeon E5-1650 v3 (per README as fetched). The README says this is in line with the 50-200 us range of the C version without transparency; transparency cost comes mainly from the hidden-face culling phase. About 7x faster than `block-mesh-rs` (about 4.5 ms per chunk, per the README).
- Maturity: v0.3.6, MIT, no runtime deps, last release Oct 18, 2024 (per the lib.rs listing). Used by the Bevy voxel game Riverbed (per README).
- Engine fit: Drop-in for chunk meshing; the padded 62^3 layout must match the engine's chunk storage or copy into it.
- URLs opened: https://lib.rs/crates/binary-greedy-meshing ; https://github.com/cgerikj/binary-greedy-meshing
- Test: `cargo bench` comparing this crate with `block-mesh` on the same data; measure transparency on vs off.

### 1.3 Vertex pulling with packed 8-byte quads
- Description: Quad data lives in a storage buffer; the vertex shader derives corners from `vertex_index` and the quad word. Removes per-vertex data and index buffers.
- Numbers: Vertex size reduced to 8 bytes per quad (per 1.1). No frame-time numbers stated on the page.
- Maturity: Used in the cgerikj design; wgpu support requires storage buffers (see section 2 limits).
- Engine fit: Mesh buffers in the renderer; requires `STORAGE` usage and a shader path without vertex buffers.
- URL opened: https://github.com/cgerikj/binary-greedy-meshing
- Test: Compare GPU memory per chunk and draw time for vertex-buffer quads vs vertex-pulled quads on the target GPU.

### 1.4 Transparency handling (sorted separate pass)
- Description: The binary greedy crate treats transparent voxels as a separate set during culling. The general pattern is to render opaque chunks first and transparent faces in a separate pass, which Aokana also uses (opaque then transparent pass in its forward renderer).
- Numbers: The 600 us vs 50-200 us gap in 1.2 is the only measured cost of transparency found (unverified for other hardware).
- Maturity: Standard practice; the crate's support is the only verified implementation here.
- Engine fit: Renderer pass ordering; chunk mesh split into opaque and transparent streams.
- URLs opened: https://lib.rs/crates/binary-greedy-meshing ; https://arxiv.org/html/2505.02017v1
- Test: Count transparent faces in typical terrain and time the two passes separately.

### 1.5 Baked ambient occlusion in the mesh pass (TCraft)
- Description: TCraft bakes ambient occlusion into chunk meshes during greedy meshing; meshing and terrain generation run on a thread pool off the render thread.
- Numbers: Not stated.
- Maturity: Hobby project (161 commits, 1 star per the page); OpenGL 4.6 not wgpu.
- Engine fit: Mesh worker stage; a cheap alternative to per-pixel smooth lighting.
- URL opened: https://github.com/tonadr1022/TCraft
- Test: Measure meshing time and vertex count with AO baked in vs off.

---

## 2. GPU-driven rendering

### 2.1 Aokana: SVDAG chunks, GPU culling and Hi-Z visibility buffer
- Description: Chunks are stored as sparse voxel DAGs (SVDAG). A compute pass selects chunks by frustum culling against precomputed AABBs; a tile pass runs Hi-Z occlusion culling from the previous frame on 8x8-pixel tiles; a ray-marching pass writes a 64-bit visibility buffer using InterlockedMax, then a color resolve shades it.
- Numbers (from the paper, measured on Ryzen 5 5600X, 64 GB, RTX 3060 Ti, Unity 6 Vulkan):
  - 6 ms average frame time at 64K (about ten billion voxels).
  - San Miguel 64K VRAM 424.11 MB (Aokana) vs 3475.05 MB (HashDAG), about 12% of HashDAG.
  - Abstract claims up to 9x memory reduction and up to 4.8x speedup vs prior state of the art; exact baseline details not stated.
  - About 5% of full voxel data must be in VRAM while navigating.
  - Limitations stated: no runtime voxel modification; transparent voxels not handled. Greenfield 8K uses more memory than plain SVDAG (140.8 MB vs 46.14 MB).
- Maturity: Academic paper (arXiv 2505.02017, 2025); Unity implementation, no public Rust port found.
- Engine fit: Best for a read-only far LOD / distant terrain layer on the GPU, not for editable near chunks.
- URLs opened: https://arxiv.org/html/2505.02017v1
- Test: Port the Hi-Z culling step on its own first. Measure cull time and draw count per frame for a static 12-chunk-radius scene on the target GPU.

### 2.2 Multi-draw indirect with compute-generated draw commands (TCraft)
- Description: TCraft uses `glMultidrawElementsIndirect` for all chunks that share a vertex format and shader, with a compute shader that does frustum culling and writes the draw buffer on the GPU.
- Numbers: Not stated; the README calls the approach a "massive performance win" with no figures.
- Maturity: OpenGL 4.6 implementation; hobby status.
- Engine fit: Maps to wgpu `multi_draw_indirect` (see 2.3 for the feature flags).
- URL opened: https://github.com/tonadr1022/TCraft
- Test: Benchmark N chunk draws as separate `draw_indexed` calls vs one indirect call; measure CPU time per frame.

### 2.3 wgpu support for indirect draw and bindless features
- Description: wgpu exposes indirect count, first-instance and binding-array features, with different backend support.
- Numbers and details (docs.rs wgpu 30.0.2 page):
  - `MULTI_DRAW_INDIRECT_COUNT`: draw count from a buffer; native only; DX12 and Vulkan 1.2+ only. Without it, `multi_draw_indirect` may be emulated.
  - `INDIRECT_FIRST_INSTANCE`: not on OpenGL ES / WebGL; Metal needs Apple3+ or Mac1+.
  - `TEXTURE_BINDING_ARRAY`: native only (DX12, Metal, Vulkan).
  - `BUFFER_BINDING_ARRAY`: Vulkan only; native only.
  - The page renames `PUSH_CONSTANTS` to `IMMEDIATES` in this version.
  - Requesting an unsupported feature panics at device creation.
- Maturity: Stable wgpu feature set; check the version your workspace uses.
- Engine fit: Renderer capability detection and fallback path (per-chunk draw calls when indirect count is missing, for example on the iGPU or WebGL).
- URL opened: https://docs.rs/wgpu/latest/wgpu/struct.Features.html
- Test: Enumerate adapter features on the iGPU and GTX 1050 (Vulkan and DX12) and log which path is used.

### 2.4 wgpu limits on weak and downlevel adapters
- Description: Default limits on the docs.rs page, with downlevel presets for weaker targets.
- Numbers:
  - `max_storage_buffer_binding_size`: 128 MiB default and in `downlevel_defaults()`.
  - `max_buffer_size`: 256 MiB in all three presets.
  - `max_storage_buffers_per_shader_stage`: 8 default, 4 downlevel, 0 in WebGL2 downlevel.
  - `max_compute_workgroup_size` X/Y: 256, Z: 64, invocations per workgroup 256 (0 in WebGL2 downlevel).
  - `downlevel_webgl2_defaults()` sets storage buffers, storage textures and all compute limits to 0.
- Maturity: Stable docs.
- Engine fit: Split large voxel data across several storage bindings (the 128 MiB per-binding cap); keep compute workgroups within 256 invocations; have a non-compute path for WebGL.
- URL opened: https://docs.rs/wgpu/latest/wgpu/struct.Limits.html
- Test: Query `adapter.limits()` on each target adapter and assert against the minimum you need.

### 2.5 Hi-Z occlusion on previous-frame depth
- Description: Aokana uses the previous frame's Hi-Z pyramid for tile occlusion and retests culled tiles against the current frame's Hi-Z.
- Numbers: Only overall 6 ms at 64K (see 2.1); no isolated Hi-Z cost stated.
- Maturity: Standard technique; the Aokana details are the verified specifics here.
- Engine fit: Depth pyramid pass in the renderer.
- URL opened: https://arxiv.org/html/2505.02017v1
- Test: Time the Hi-Z build compute pass and the culling pass separately on the target GPU.

---

## 3. LOD for voxel worlds

### 3.1 Aokana LOD: chunk merge with density threshold
- Description: Eight LOD n chunks merge into one LOD n+1 chunk at the same 256^3 resolution; a merged voxel exists when at least two of its eight children are non-empty and takes their average color. The CPU walks an implicit octree and subdivides when `(ChunkSize x StreamingFactor) - distance` is positive.
- Numbers: StreamingFactor 2.0 in the experiments; SSIM 0.888 (San Miguel, 341 active chunks), 0.8 (Sponza, 473), 0.866 (Greenfield, 130).
- Maturity: Paper (2025).
- Engine fit: Build LOD from the chunk store; works with server-supplied chunks if the server sends the base level.
- URL opened: https://arxiv.org/html/2505.02017v1
- Test: Build LOD n+1 from random chunks; measure time per merge and the error (SSIM or voxel-count delta) against distance.

### 3.2 Distant Horizons: background LOD generation, server-supplied LOD
- Description: Records explored terrain, generates simplified LOD chunks on background threads, and can accept LOD data from a compatible server mod or plugin. The client mod is still required.
- Numbers: Background threads configurable from 1 to 16+; the page gives no LOD level count, memory or per-thread CPU figures.
- Maturity: Popular Minecraft mod (Java); design is "traditional LOD" per the page. How LOD is updated after block edits is not stated.
- Engine fit: Pattern for server-sent LOD: the server computes and streams a coarse layer; the client only stores and draws it.
- URL opened: https://distant-horizons-guide.pages.dev/about
- Test: Measure LOD generation time per chunk-region on a background pool and bytes per LOD region.

### 3.3 Voxy (Minecraft LOD mod)
- Description: The GitHub page only says it is "an LoD rendering mod for minecraft"; no internals (storage, GPU path, update method) are on the page.
- Numbers: Not stated.
- Maturity: Actively used Minecraft mod (Modrinth listing not opened).
- Engine fit: Reference only; internals need the Modrinth page, source files or its README.
- URL opened: https://github.com/Vrgrwh/voxy
- Test: Not applicable without internals; list as "needs deeper read".

### 3.4 Transvoxel seam stitching
- Description: Transition cells are inserted where a voxel mesh at one resolution meets a neighbour at exactly half that resolution. Each transition cell uses nine samples of the high-resolution data; 512 cases collapse into 73 equivalence classes with a triangle pattern per class.
- Numbers: Qualitative only; the page says transition cells need only local voxel data. No timing or frame figures.
- Maturity: Algorithm is free of patent claims (per page); full C4 Engine implementation and lookup tables exist; no Rust port checked.
- Engine fit: Seams between LOD levels of the marching-cubes or surface-nets terrain.
- URL opened: https://transvoxel.org/
- Test: Mesh two adjacent blocks at 1x and 2x resolution, count cracks or holes, measure retriangulation time per changed chunk.

### 3.5 SVDAG LOD for server-side distant terrain
- Description: Aokana's chunk-level SVDAG (see 2.1) is also a LOD representation: coarser levels are the same data structure with merged voxels.
- Numbers: See 2.1.
- Maturity: Paper; read-only data.
- Engine fit: Distant layer streamed from server (see 3.2 pattern).
- URL opened: https://arxiv.org/html/2505.02017v1
- Test: Compare SVDAG size and query time vs plain octree for the same LOD level.

### 3.6 Terrain-change LOD update
- Gap: No verified source for how Aokana, Voxy or Distant Horizons update LOD after block edits. Aokana states runtime voxel modification is not supported. Plan a dirty-region approach and test it.

---

## 4. Lighting

### 4.1 BFS flood fill with per-channel light (0fps)
- Description: Light spreads by BFS along the six faces of each voxel. Each voxel stores R, G, B (4 bytes total per voxel per the post), with sky light sampled along several directions. The post describes bit-packing light channels into machine words for parallel propagation.
- Numbers: Not stated.
- Maturity: Long-running blog post (2018); a reader comment warns that some links now redirect to a phishing page, so do not follow them.
- Engine fit: Per-chunk light arrays inside the chunk job; sky light column handling.
- URL opened: https://0fps.net/2018/02/21/voxel-lighting/
- Test: Time BFS over a 16^3 section and a full 12-chunk radius; measure light memory in bytes per voxel.

### 4.2 Two-pass cross-border lighting with 2x2 chunk groups (dktapps spec)
- Description: Pass 1 propagates inside one chunk column and does not touch other chunks. Pass 2 handles borders in 2x2 groups of chunks; each chunk is in four groups and is fully lit after all four. Pass 2 can be skipped when pass 1 produced no border light. Block light sources are skipped for sub-chunks whose palette has no emitting block.
- Numbers: Not stated.
- Maturity: Specification from a Minecraft-protocol project; describes the Java behaviour.
- Engine fit: Parallel light jobs on rayon with per-group locks; skip-empty-palette check ties into palette storage (see 5.1).
- URL opened: https://github.com/dktapps/lighting-algorithm-spec
- Test: Time pass 1 and pass 2 separately across a 12-chunk radius on a rayon pool; count border updates.

### 4.3 Invalidation granularity (spec)
- Description: The spec invalidates light only at chunk granularity: replacing a chunk invalidates its own light and that of the surrounding 8 chunks. It does not describe incremental removal of light.
- Numbers: Not stated.
- Maturity: Spec.
- Engine fit: If the engine needs single-block edit responsiveness, this granularity is too coarse; plan an incremental remove/add queue (not covered by a verified source).
- URL opened: https://github.com/dktapps/lighting-algorithm-spec
- Test: Measure time to relight after one block edit using chunk-level recompute vs an incremental queue.

### 4.4 Smooth lighting and GPU lighting
- Gap: No verified source with numbers for smooth lighting or GPU lighting was found in the opened pages. The 0fps post and TCraft's baked AO (see 1.5) are the only leads. Treat as "needs search".

---

## 5. World storage and memory

### 5.1 Minecraft section palette with bit-packed indices
- Description: Each 16x16x16 section stores a palette of block states plus a packed long array of indices. Index bit width is uniform per section, minimum 4 bits, and indices can span longs. Empty sections are not saved.
- Numbers: 4 bits minimum per index. At that width, 4096 indices = 16384 bits = 2048 bytes of packed data (256 longs) per section, plus the palette. Derived arithmetic, not measured. Direct-palette threshold not stated on the page.
- Maturity: Vanilla Java format (per the wiki page); widely reimplemented.
- Engine fit: Core chunk storage for the client and server. This lets chunks with few block types use very little RAM.
- URL opened: https://minecraft.miraheze.org/wiki/Chunk_format
- Test: Build a section from realistic terrain and measure bytes per section for palette vs flat `u16` arrays; check against the 2 GB client budget.

### 5.2 Linear region format (single-file Zstd per region)
- Description: Writes a whole 32x32 chunk region as one contiguous file, compressed with Zstandard at a tunable level; replaces `.mca`/`.mcc` files.
- Numbers: The page says "reducing world save size by 50-60%"; methodology not given (unverified).
- Maturity: Early-development Minecraft mod; format version and checksums not stated; compatibility with vanilla not stated.
- Engine fit: Pattern for the engine's own region file: per-region Zstd, one file per region.
- URL opened: https://github.com/memesgmm/Linear/
- Test: Save 1000 random-but-realistic regions with `zstd` at levels 1, 3, 9 and measure size and load time.

### 5.3 rkyv zero-copy and mmap
- Description: rkyv gives zero-copy deserialization of archived data, which can be memory-mapped for region loads.
- Numbers: Not verified (search only).
- Maturity: Stable crate; versions 0.7 and 0.8 release candidates seen in listings.
- Engine fit: Region file payloads for fast load; pair with 5.2.
- URLs: search only (https://rkyv.org/print.html was in the results, not opened).
- Test: Load 1000 regions via rkyv archive + mmap vs bincode + zstd; measure load time and peak RSS.

### 5.4 Memory budget note
- The 2 GB client budget should be checked with `dhat` (heap profiling) and the palette test (5.1). No verified numbers for total client RAM were found for any engine above.

---

## 6. Server scaling

### 6.1 Folia regionized threading
- Description: Groups nearby loaded chunks into independent regions that each tick at 20 TPS on a thread pool. Regions do not share data. Entity and player commands run on the region that owns the entity; console commands run on the global region.
- Numbers (Folia docs): Ideally at least 16 cores; test server about 330 peak players; thread guidance (rough estimate): Netty IO about 4 threads and chunk IO about 3 per 200-300 players; keep total threads under 80% of CPUs. No benchmark or speedup numbers. Restricted APIs (scoreboards, teleport, world loading).
- Maturity: Production Minecraft fork (PaperMC).
- Engine fit: Direct model for the server's region threads on bevy_ecs; a region is a bevy `World` or sub-schedule owned by a worker.
- URL opened: https://github.com/PaperMC/Folia
- Test: Simulate 500 players spread across regions in a bevy_ecs world; measure tick time per region and total tick vs region count.

### 6.2 MultiPaper: chunk ownership across processes
- Description: Several server processes run one world; a MultiPaper-Master stores world data and decides chunk ownership ("first in first served"). Servers cache chunks they need and subscribe to changes. A built-in proxy sends players to the least loaded server.
- Numbers: None stated. Aspiration "upwards of 1000 players" is not a measured result.
- Maturity: Public beta; Java 1.20.1 Purpur fork; features "can occasionally break" (per page). Entities are not described as moving between servers.
- Engine fit: Pattern for chunk ownership with a coordinator; a Rust version would need its own entity handoff.
- URL opened: https://github.com/PureGero/MultiPaper
- Test: Model a coordinator that grants chunk ownership and measure ownership-transfer latency under load.

### 6.3 Photon seamless MMO backend: dynamic partitioning, overlap zones, exactly-once ops
- Description: Simulation spread across nodes with dynamic repartitioning instead of a fixed grid; neighbouring nodes share an overlap zone where each node predicts and reconciles entities it does not own; a delivery layer aims for exactly-once entity operations during transfers; clients connect through a gateway.
- Numbers: Simulation tick 30 Hz per node; gateway state updates 30 per second per player; handoff "sub-millisecond" (Q&A section, claim); simulated ping 100 ms; about 7 years and about 10 architecture iterations. Players per node not stated.
- Maturity: Company blog post; production status is partly stated.
- Engine fit: Overlap zones and gateway model for the 5000+ player target across shards.
- URL opened: https://blog.photonengine.com/building-a-seamless-mmo-backend-across-machines/
- Test: Two bevy_ecs nodes exchanging entities over quinn; measure handoff time and missed or duplicated updates over a 100 ms simulated link.

### 6.4 Interest management / AOI
- Gap: The SpatialOS-style search returned only a Medium article and forum threads, which were not opened. Use the Photon overlap-zone design (6.3) as the verified pattern and measure AOI cost separately.

---

## 7. Networking

### 7.1 Authoritative server with client prediction and interpolation (Photon)
- Description: Clients send inputs; the server decides outcomes. The client predicts its own entity and corrects when authoritative state arrives; remote objects are interpolated from snapshots; clients only see the authoritative node's state.
- Numbers: 30 state updates per second per player (Photon). No bandwidth figures.
- Maturity: Standard pattern; described in Photon's article.
- Engine fit: Chunk streaming for the world, entity snapshots at 30 Hz for players.
- URL opened: https://blog.photonengine.com/building-a-seamless-mmo-backend-across-machines/
- Test: Simulate 100 ms RTT with a link shaper and measure correction size on the client.

### 7.2 Lag compensation (Gambetta)
- Description: The client sends shot timestamp and aim; the server reconstructs the world at that past instant, processes the shot, then updates clients. Tradeoff: a target that has just moved behind cover can still be hit for a few milliseconds.
- Numbers: "A few milliseconds" window (qualitative).
- Maturity: Well-known article series; this page is the lag-compensation part only.
- Engine fit: Only needed for combat in shows or games; keep a per-tick snapshot ring buffer on the server.
- URL opened: https://gabrielgambetta.com/lag-compensation.html
- Test: Keep 500 ms of per-tick snapshots and measure memory and rewind time.

### 7.3 Client prediction and reconciliation: details not verified
- Gap: The Gambetta series page points to earlier parts for the rewind-and-replay rules, which I did not open. Search results listed similar material (not opened).

### 7.4 QUIC datagrams with quinn
- Description: quinn is a Rust QUIC implementation; bevy_quinnet wraps it for Bevy. Datagrams (unreliable) suit state updates.
- Numbers: Not verified (search results only).
- Maturity: quinn 0.11.x latest on docs.rs listing; bevy_quinnet 0.20.0 listed.
- Engine fit: Transport for player state (datagrams) and chunk streams (reliable streams with priority).
- URLs: search only (https://docs.rs/crate/quinn/latest, https://docs.rs/crate/bevy_quinnet/latest/source/README.md; not opened).
- Test: Send 30 Hz position datagrams for 500 players per node; measure goodput and loss under a shaped link.

### 7.5 Delta compression and chunk streaming priority
- Gap: No verified source with numbers was opened for delta compression or chunk priority. Plan: diff against last acknowledged chunk palette, send changed sections only, and order by distance to the player.

---

## 8. Modding performance

### 8.1 Wasmtime epoch vs fuel interruption
- Description: Epoch-based interruption checks a global epoch from another thread; fuel metering counts instructions and is deterministic.
- Numbers (Wasmtime docs): Epochs "measured at around a 10% slowdown"; fuel "imposes more overhead ... than epochs do" with no figure. Fuel traps with `Trap::OutOfFuel`. Epoch deadline default traps; can async-yield.
- Maturity: Stable Wasmtime features.
- Engine fit: Epochs for plugin time-slicing on the server tick; fuel only where determinism matters (replay or anti-cheat).
- URL opened: https://docs.wasmtime.dev/examples-interrupting-wasm.html
- Test: Run a no-op and a 1 M-iteration loop plugin with epochs on vs off and fuel on vs off; measure ns per call.

### 8.2 Wasmtime Component Model cold start (academic data)
- Description: Measured Wasmtime cold start (from `Component::new()` including JIT) and execution for three workloads on one machine.
- Numbers (arXiv 2509.09400, AMD Ryzen 7 5700U, 16 GB): no-op cold start 5.6 ms average (lowest); Mandelbrot cold start 16.9 ms; image processing cold start 188.0 ms; Mandelbrot execution 161.84 ms and image processing 149.09 ms without I/O. Per-call latency not stated.
- Maturity: Academic study on serverless/edge; not a plugin benchmark.
- Engine fit: Plugin load time at server start and on hot-reload.
- URL opened: https://arxiv.org/pdf/2509.09400
- Test: Measure `Component::new` and first call for your plugin size; measure ns per host call with a typed Component Model interface.

### 8.3 Scripting call overhead numbers (not Wasmtime)
- Description: Author's measurements on a non-Wasmtime scripting backend and a threadpool task.
- Numbers: Threadpool task median 5364 ns (micro-benchmark), 11152 ns with ondemand frequency scaling; author estimates live systems at 2-5x the micro-benchmark. LuaJIT call overhead 55 ns to 80 ns with instruction counting. Syscall-like host call 19-21 ns (the author's own RISC-V emulator). Author derives about 44 calls per microsecond (derived, not measured).
- Maturity: Blog post; hardware and CPU model not stated.
- Engine fit: Rough order-of-magnitude for host-call cost in an interpreter; use only as a sanity check.
- URL opened: https://fwsgonzo.medium.com/time-to-first-instruction-53a041da0ce5
- Test: Use the same host-call pattern in Wasmtime and compare.

### 8.4 Veloren plugin system
- Description: Plugins compile to `wasm32-wasi`, package as a `*.plugin.tar` with `plugin.toml` and `.wasm` modules. Interaction is event-driven (`#[event_handler]`, `emit_action`), because the page says WASM has no well-defined host ABI yet.
- Numbers: None on performance.
- Maturity: Experimental. The page says APIs are unstable, functionality is limited, and it requires nightly Rust. The page does not name the runtime, so it does not confirm Wasmtime.
- Engine fit: Useful as a model for event/action plugin APIs and sandbox boundaries, not as Wasmtime performance evidence.
- URL opened: https://book.veloren.net/contributors/modders/writing-a-plugin.html
- Test: Not applicable directly; prototype the event/action model with Wasmtime Component Model.

### 8.5 Luanti (Minetest) modding lessons
- Description: Mods use buffer arguments for VoxelManip and Perlin noise; reuse Perlin objects; apply mapgen memory optimizations because badly coded mapgens are "a likely cause of OOM errors"; guard delayed callbacks; persist entity data with staticdata.
- Numbers: None stated.
- Maturity: Mature engine (Luanti, formerly Minetest).
- Engine fit: Plugin API design: bulk-buffer access instead of per-voxel calls; explicit memory rules for plugin-generated data.
- URL opened: https://docs.luanti.org/for-creators/modding-tips/
- Test: Compare per-voxel host-call vs bulk-buffer host-call in Wasmtime.

### 8.6 Vintage Story server/client mod split
- Description: The game creates a ModSystem on the server and one on each client; `ModSystem.Start` runs on both. Mod-side classification is marked TBA on the page. Singleplayer shares a dataPath between server and client.
- Numbers: None; sync and performance guidance not covered by the page.
- Maturity: Commercial game, stable.
- Engine fit: Warning: keep server-only state separate from client state to avoid the shared-path pitfall.
- URL opened: https://wiki.vintagestory.at/index.php/Modding:Server-Client_Considerations
- Test: Run a dedicated server and a client with separate data paths.

### 8.7 Hytale plugin model (secondary sources)
- Confirmed by Hypixel (per a secondary page citing Hypixel posts): plugins are Java `.jar`; server source to be released; "server-side first" modding; no client mods. Dedicated servers available at launch.
- Not confirmed by Hypixel: Java server, sequential world generation/AI/physics, chunk model details.
- URL opened: https://winternode.com/blog/hytale/what-we-know-about-hytale-server-hosting

---

## 9. Rust-specific: SIMD, allocators, PGO/BOLT, profiling

### 9.1 PGO workflow (rustc)
- Description: Four steps: instrumented build with `-Cprofile-generate`, run workloads to produce `.profraw`, merge with `llvm-profdata`, rebuild with `-Cprofile-use`. Use `--target` in Cargo to keep `RUSTFLAGS` out of build scripts; use absolute paths; use Cargo 1.39+. Sampling PGO is also supported via `-Cprofile-sample-use`.
- Numbers: None; the chapter says only that the result "should" be faster.
- Maturity: Stable rustc feature.
- Engine fit: Release builds of the server and client, trained on a representative workload (region ticks, meshing, networking).
- URL opened: https://doc.rust-lang.org/rustc/profile-guided-optimization.html
- Test: Compare tick time and mesh throughput for a plain release build vs PGO build on the same workload; keep the training workload fixed.

### 9.2 BOLT (post-link optimization)
- Gap: Not opened. Search did not return a primary BOLT source. Treat as "needs search".

### 9.3 Tracy profiler with Rust bindings
- Description: Tracy is a real-time, nanosecond-resolution telemetry profiler that covers CPU zones, memory allocations, locks, and GPU work across OpenGL, Vulkan, D3D11/12, Metal, OpenCL, CUDA and WebGPU.
- Numbers: No overhead figure stated on the page.
- Maturity: Mature; `rust_tracy_client` is the Rust binding named on the page.
- Engine fit: Frame profiling for meshing, GPU passes (wgpu uses Vulkan/DX12/Metal backends) and server ticks.
- URL opened: https://github.com/wolfpld/tracy
- Test: Add `rust_tracy_client` zones around meshing, the server tick and render passes; measure overhead with the feature on and off.

### 9.4 Allocators: mimalloc
- Description: mimalloc is a general-purpose allocator; the Rust `mimalloc` crate can be set as `#[global_allocator]`.
- Numbers: Not verified (search results only); no game-engine benchmark was opened.
- Maturity: Stable; crate versions 0.1.x listed.
- Engine fit: Global allocator for the server to reduce per-chunk and per-entity allocation cost.
- URLs: search only (https://crates-io.ganymede-static.progval.net/crates/mimalloc/0.1.38 not opened).
- Test: Run the same server tick benchmark with the system allocator vs `mimalloc`; measure p99 tick time.

### 9.5 SIMD (std::simd / wide)
- Gap: No source opened for SIMD speedups in voxel meshing or lighting. The binary greedy approach (1.1) already uses 64-bit bitwise parallelism; measure before adding explicit SIMD.

### 9.6 Heap profiling
- Gap: `dhat` was not opened; it is the usual Rust heap profiler for checking the 2 GB budget (section 5.4).

---

## Recommended test order (cheap first)

1. Meshing: bench `binary-greedy-meshing` vs `block-mesh` on real chunks (1.2). Hours of work.
2. Storage: palette vs flat array bytes per section (5.1); region zstd level sweep (5.2). Hours.
3. Wasm plugins: epoch vs fuel overhead on a no-op and a loop plugin (8.1). A day.
4. wgpu capability dump on iGPU and GTX 1050 (2.3, 2.4). A day.
5. Server: 500-player region tick with bevy_ecs regions (6.1). Several days.
6. Networking: 30 Hz datagrams over quinn with a link shaper (7.4). Several days.
7. GPU LOD: port the Aokana chunk-cull and Hi-Z passes only if the distant layer is needed (2.1, 2.5). Weeks.

## Not covered or unverified (explicit list)

- Voxy internals: not stated on the opened page.
- Distant Horizons memory and LOD update behaviour: not stated.
- Hytale server internals beyond the Hypixel-confirmed points in 8.7.
- SpatialOS / interest management details: only search results, not opened.
- mimalloc, rkyv, quinn, BOLT, dhat, smooth/GPU lighting: search results only or not found.
- Client prediction rewind rules: referenced but not opened (7.3).
- Valence and ferrumc claims of "thousands of players" and "highly efficient memory": project claims, no benchmarks (unverified).
