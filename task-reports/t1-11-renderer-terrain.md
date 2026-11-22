# T1-11 — Terrain renderer: upload, culling, texture arrays, profiles

Status: cloud part done; the GPU frame time on the Vega 10 and the decisions on open questions 13 and 14 wait for the owner's laptop run (commands in the task file).

## What was done

**`render::terrain`** (new module, `crates/render/src/terrain/`):

- `arena.rs` — `RangeArena`, a first-fit range allocator with a coalescing free list for the shared quad buffer. One contiguous range per render batch; steady-state updates do not allocate. Tests: alloc/free/coalesce, 5,000 random operations without overlap and with exact accounting.
- `culling.rs` — `Frustum` (planes from a wgpu view-projection, conservative AABB test), `visible_face_mask` (≤ 3 directions when the camera is outside a box), `cave_visible` (BFS over mesh chunks through connected faces, never stepping back against a direction already taken, bounded by the frustum and the radius). Tests for all three.
- `light_atlas.rs` — slot bookkeeping for the GPU light: 8,192 bricks of 16³ in a 512 × 512 × 128 `R8Uint` atlas plus a toroidal 64 × 32 × 64 indirection volume; uniform sections (open sky, dark rock) need no brick.
- `profiles.rs` — `QualityProfile::{Low, Default, High}` = 8/12/16 chunks, fog start and mipmaps; never below 8 (PRD §14).
- `camera.rs` — perspective and look-to matrices (the workspace glam has no projection helpers).
- `renderer.rs` — `TerrainRenderer` on wgpu: quad buffer + arena, 4,096 batch slots with world origins in a uniform array, the block texture array with mips, a `state × direction → layer` table, light atlas + indirection textures, three draw variants (`TerrainPath::{Pulling, PullingIndexed, Instanced}` = B, indexed B, C), `prepare` (frustum + face culling, neighboring directions merged into one draw, a reused draw list) and `render`. A test validates the three WGSL variants with naga.
- Shaders `terrain-common.wgsl` + one entry file per variant: batch-quad decoding (the T0-05 max-corner rule), world-space UVs, alpha test, flat light from the cell in front of the face (`max(block, sky × daylight)`), fixed per-direction shade, distance fog.

**`mesh`:** `visibility.rs` — `ChunkVisibility`, the 6 × 6 face connectivity of a mesh chunk from a flood fill over non-opaque cells (test: empty, solid, tunnel).

**`client`:** mesh jobs compute `ChunkVisibility` next to the quads; `Loader::visibility()` exposes it (all-air chunks have no entry = fully open). New example `screenshot`: generates and loads a world, uploads batches and light, prints culling statistics for a surface and a cave camera, renders offscreen and saves a BMP; `FRAMES=n` adds timed frames with GPU timestamp queries; `ADAPTER_NAME` and `WGPU_BACKEND` pick the GPU.

**Docs:** `meshing-and-rendering.md` (new "Terrain renderer" section; quad table fixed: width/height are stored as is, not minus one; bits 61–63 carry the direction in the GPU copy; path B uses `first_instance` for the batch slot), `performance-budgets.md` (two rows), `open-questions.md` 13 and 14 (data so far), task file (status, criteria, laptop commands), `tasks/README.md`, a cloud screenshot `docs/research/t1-11-terrain-screenshot.png` (our fallback textures only).

### Numbers (cloud VM: 4 vCPU Intel Xeon 2.1 GHz, software Vulkan llvmpipe — GPU times are not representative)

World seed 7, camera at (8, 95, 8), 1280 × 720.

| What | Result |
| --- | --- |
| Cave culling, camera in a cave at (−23.5, −49.5, −17.5) | 165 → 40 mesh chunks (76% discarded) at radius 8; 325 → 74 (77%) at radius 12 |
| Cave culling at the surface | 21–26% discarded (sealed rock below the surface) |
| Cave traversal CPU, radius 12 | 18–85 µs over 4 runs |
| `prepare` (frustum + faces + draw list), 147 batches | 2.3–5.3 µs median |
| Pass recording, 161 draws | 11–38 µs |
| `FRAMES=60`, radius 12, after the first frame: prepare + record | median 0.057–0.067 ms, p95 ≤ 0.105 ms, p99 ≤ 0.110 ms (B, Bi, C) |
| GPU pass median (llvmpipe), radius 12, 277k of 1.09M quads drawn | B 190 ms, Bi 94 ms, C 118 ms |
| Upload, radius 12 | 147 batches + 17,496 light sections in 96 ms |

B and Bi pictures are pixel-identical; C differs in 146 of 921,600 pixels (max delta 49/255) at triangle edges.

Checks: `cargo fmt`, `cargo clippy --workspace --all-targets` with `-D warnings`, `cargo test --workspace`, `cargo bench --workspace --no-run` — all pass.

## What went wrong

- The workspace glam (0.34.3) has no `perspective_rh`/`look_at_rh`; added `terrain::camera` instead.
- The first cave-culling test used a wall smaller than the search radius, so the traversal went around it; the wall now covers the radius.
- The first "underground" measurement put the camera inside solid stone at y −20 (58% discarded, but not a cave); the example now searches for an air cell in a real cave.
- Two `FRAMES` runs showed one 10–24 ms CPU outlier (prepare + record). It was not reproduced in later runs; on this VM llvmpipe renders on all 4 vCPUs at the same time, so the recording thread gets preempted. The first frame is now reported separately; the laptop run will show whether outliers exist on real hardware.
- A clippy lint (`single_range_in_vec_init`) in the arena; fixed.

## Issues encountered

- **Open for the owner:** GPU frame time at 12 chunks on the Vega 10 and the B / Bi / C decision (open question 13); the CPU recording statistic (open question 14). The cloud data says the CPU side is far inside 1 ms at both p95 and p99.
- **Spec deviations (documented in the spec):** the GPU copy of a quad carries the face direction in bits 61–63 (the CPU format is unchanged), and path B uses `first_instance` (direct draws) for the batch slot, where the spec said it was not needed.
- **Not done here:** the transparent and water passes (water is drawn with the opaque alpha test for now); smooth light and AO; a separate grass top texture (the fallback set has one `grass_block` texture, so tops look like sides); defragmenting the quad buffer (a full buffer returns `QuadBufferFull`); settings UI hooks for profiles come with the client app (T1-13). Cave culling runs on mesh chunks and its result is passed to `prepare` as a set of batches; the client app has to wire it per frame.
- The software Vulkan driver (`mesa-vulkan-drivers`) is installed by hand in each cloud session; it is not in the environment setup script.
