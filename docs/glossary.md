# Glossary

| Term | Meaning |
| --- | --- |
| Section | A 16³ cube of blocks, the unit of storage, lighting and network transfer. |
| Column | A stack of sections over the full world height (24 sections at height 384). What Minecraft calls a "chunk". |
| Chunk (in render-distance metrics) | A horizontal 16×16 block cell. "12 chunks of render distance" = a radius of 12 columns. |
| Mesh chunk | A 32³ block cube (2×2×2 sections), the unit of meshing. ADR-0005. |
| Render batch | 2×2×2 mesh chunks (64³ blocks), drawn with a single command. ADR-0005. |
| Quad | A rectangular face after greedy meshing, packed into a `u64` (8 bytes). |
| Render bucket | One geometry type inside a render batch with its own pipeline: voxel quads, model instances, later smooth meshes and water surfaces. ADR-0016. |
| Render graph | The ordered stages of a frame (setup, shadow, G-buffer, deferred, translucent, composite, final) and the targets they read and write. ADR-0015, `render-graph.md`. |
| G-buffer, MRT | Several render targets written in one geometry pass (albedo, normal, material, motion vectors), read later by deferred passes. |
| Generation backpressure | Holding or slowing a player at the edge of loaded terrain instead of letting them enter unloaded columns. T2-10. |
| Masks | Bit masks of opaque and transparent voxels of a mesh chunk; input for `fast_mesh`. |
| Gather | Unpacking sections into a flat 34³ buffer (with padding) before meshing. |
| Region (threading region) | A dynamic group of sections around players that is ticked by one thread. ADR-0008. Not to be confused with a game zone. |
| Game zone | A static map partition (biome, difficulty, own spawn tables). PRD §14, no. 11. |
| Global region | A separate region for world state (time, weather, tables); others read its snapshot from the previous tick. |
| Region buffer | Empty sections around occupied ones; two regions never touch directly. |
| Tick phases | `PreTick → Simulate → Exchange → PostTick`. |
| Node | One server process on one machine. |
| Shard / cell | A part of the world (e.g. 512×512 blocks) owned by one node. |
| Gateway | A process that holds the client's QUIC connection and routes it to the owning node. |
| Ownership number | A counter that grows on each entity handoff between nodes; protects against dupes. ADR-0009. |
| LOD n | A 16³ voxel section where a voxel = 2ⁿ blocks. |
| Datapack | A level 1 mod: TOML/JSON + assets, no code. |
| WASM mod | A level 2 mod: a WebAssembly component following the `wit/engine-api.wit` contract. |
| Native plugin | A level 3 mod: a Rust crate built together with the engine. Project team only. |
| Epoch | A wasmtime mechanism for interrupting a guest by time; a watchdog against hangs, 0–9% cost. |
| Fuel | A deterministic wasmtime instruction counter; +50–90% cost, for tests and replays only. |
| Hook | A call from the host into the guest (`on-events`, `run-system`). Always batched. |
| Quality profile | Low / Medium / High / Ultra; determines distance, shadows, effects. |
| In-game launcher menu | The game's main menu that acts as the launcher. ADR-0010. |
| Updater | A separate tiny binary that updates the game and launches it. ADR-0010. |
| ADR | Architecture Decision Record, a file in `docs/adr/`. |
