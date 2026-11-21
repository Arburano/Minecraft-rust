# Meshing and rendering

Crates: `mesh`, `render`. ADR: 0003 (own renderer on wgpu), 0005 (32³ mesh chunk + 2×2×2 batches).

## Pipeline

```text
block edit ─► masks (incremental) ─► re-mesh queue (Interactive priority)
new chunk  ─► gather 34³ + masks  ─► queue (Visible / Prefetch)
                        │
                        ▼
              fast_mesh 32³ (binary-greedy-meshing 0.6.4)
                        │  u64 quads, 6 lists by face direction
                        ▼
        copy into the 64³ render batch (offset +0/+32 on each axis)
                        │
                        ▼
         slab allocator in a large GPU buffer ─► draw
```

## Meshing

- Crate: `binary-greedy-meshing` 0.6.4, `Mesher<32>`, the `fast_mesh` method with precomputed masks.
- Input: a 34³ buffer (32³ + 1 block of padding on each side from neighbors). The 8 core sections are gathered word by word (`Section::unpack_into`, see world-storage.md); the 1-block-thick padding shell (6 536 cells) is read with per-cell `get` — unpacking whole 4 096-cell sections for a ≤ 256-cell sliver measured ≈ 3× slower (T0-04). Uniform sections are filled without unpacking, with whole bit-range mask updates.
- Opacity and transparency masks are maintained incrementally on `set_block` (target ≤ 20 ns per edit). Building from scratch happens only on chunk load.
- Empty and solid mesh chunks are skipped by section flags; the mesher is not called. "Solid" means every section in the 4×4×4 neighborhood is fully opaque (a solid core with air padding still meshes its boundary faces).
- An edit on a mesh chunk border affects neighbors (padding): up to 4 mesh chunks (up to 8 with diagonal AO). `mesh::affected_mesh_chunks` computes the dirty set.
- Non-cubic models (stairs, fences, plants) take a separate path: precomputed model templates, instances. No greedy merging.

Measured (VM 2 vCPU, ±30%, 62³ unit, pre-code research): `fast_mesh` 0.27 ms, masks from scratch 0.35 ms, `mesh()` 1.1 ms.

Measured (VM, T0-04, 32³ mesh chunk, seeded terrain corpus 4×2×4 chunks, 8.1% transparent; `mesh` bench `mesh_chunk`): gather + masks ≈ 60 µs, `fast_mesh` ≈ 42 µs, full build ≈ 99 µs, re-mesh after an edit ≈ 30 µs, mask update per edit ≈ 2.6 ns, mask rebuild ≈ 41 µs; ≈ 535 quads/chunk (4.2 KB). Bare-metal confirmation pending.

## Render batches (ADR-0005 decision)

- A 32³ mesh chunk produces quads with coordinates 0..31 (anchor planes and max corners reach 32). When copying into a 64³ batch we add an offset of 0 or 32 on each axis — a flip of bit 5 of each 6-bit coordinate field. Coordinates are stored modulo 64: a max-corner anchor field (positive face plane, or the far corner of Down/Right/Front) reading 0 means 64; such fields are always ≥ 1, so this is unambiguous. Min-corner fields never wrap.
- A batch = 8 mesh chunks, one range in the GPU buffer per face direction. Re-meshing one mesh chunk → re-copying the whole batch (`mesh::RenderBatch::pack_into`). Decided by the T0-05 benchmark (open question No. 8): full re-copy ≈ 1.6 µs vs ≈ 0.2 µs for the 8-sub-range variant (VM, 4 976-quad batch), both far under the 50 µs budget — the ~1.4 µs saving does not justify 48 ranges with per-slot counters. Bare-metal confirmation pending.
- Number of draw commands: ≈ as for a 62³ unit (≈ 600 at 12 chunks without indirect), not ≈ 4,500 for separate 32³ ones.

## Quad format (8 bytes)

| Bits | Field |
| --- | --- |
| 0–5, 6–11, 12–17 | x, y, z in the batch (0..63, modulo 64: a max-corner field of 0 means 64, see above) |
| 18–23, 24–29 | width, height − 1 |
| 32–47 | block type / texture index (16 bits) |
| 48–55 | flat face light (reserve, if the 3D light texture is unavailable) |
| rest | reserved (AO flags, animation) |

The face direction is not stored in the quad — it is given by the list (6 ranges). Smooth light and AO are not put into the quad: greedy merging would break quads on every light change. The shader reads light from the sections' 3D light texture.

## Capability probe (at client start)

We read `adapter.features()`, `limits()`, `get_downlevel_capabilities()` and choose a path:

| Path | Condition | What we do |
| --- | --- | --- |
| A. GPU-driven | `INDIRECT_EXECUTION` + compute + `MULTI_DRAW_INDIRECT_COUNT` (DX12, Vulkan 1.2+) | compute culling writes indirect arguments, 1–3 calls per pass |
| B. Vertex pulling | `DownlevelFlags::VERTEX_STORAGE` | CPU culling, a draw per render batch × direction, quads from a storage buffer, vertex from `vertex_index` (quad = `vertex_index / 6`), `first_instance` not needed |
| C. Fallback | no storage in the vertex shader (GL ES 3.0, old drivers) | the same 8 bytes as a per-instance `Uint32x2` attribute, 4 vertices per instance |

Stages 0–1 build B and C; A comes from stage 2, only after measurements (analysis §13.3). Capability dump on an iGPU and GTX 1050 (Vulkan, DX12, GL) — task T0-06; stage 0 evidence is taken on Radeon RX Vega 10 + GTX 1660 Ti instead (open question 12, T0-14), so GTX 1050 remains unverified. The [2026-11-19 laptop probe](../research/gpu/laptop-probe-2026-11-19.json) shows `VERTEX_STORAGE` and `TIMESTAMP_QUERY` on both adapters on all three backends, so both select path B; B/C measurements and the [summary](../research/gpu/summary-2026-11-19.md) are in T0-14.

Platform findings from T0-14 that the stage 1 renderer must handle:

- **Path choice:** on the Vega 10 iGPU path C was 25–32% faster than B on all backends (synthetic unit quads, non-indexed B). The default stays B until re-measured — [open question 13](../open-questions.md).
- **Vulkan on AMD switchable-graphics laptops:** `VK_LAYER_AMD_switchable_graphics` made Vulkan enumerate zero adapters; it worked only with `VK_LOADER_LAYERS_DISABLE=VK_LAYER_AMD_switchable_graphics`. Players cannot be asked to set environment variables, so backend selection must fall back (DX12 on Windows, then GL) when Vulkan returns no adapters, and must never assume Vulkan exists.
- **Selection (T1-05, `render::selection`):** `attempt_order` is a pure function over the enumerated adapters: backends by the user's choice, then `Vulkan → DX12 → GL` on Windows (`Vulkan → GL` on Linux, `Metal` on macOS), then any other backend that listed adapters; inside, adapters whose name matches the user's setting, then by GPU class (`HighPerformance`: discrete first — the default; `LowPower`: integrated first), then by name. Software adapters come last of all. `select_device` enumerates each backend separately (an empty Vulkan does not hide DX12), tries device creation along that order and reports failed attempts. The user preference (`GpuPreference`: backend, adapter name, power) is serde-serializable for the client settings (T1-13). `gpu-select` prints the decision on a real machine.
- **Adapter order is not stable:** the NVIDIA "preferred GPU" program setting swapped DX12/Vulkan indices and made GL expose only the iGPU. Select adapters by type/capability/name, never by index.
- **GL on the AMD iGPU:** `gpu-probe` crashed at process exit (segfault, after valid output) while GL was on the Vega 10 ([note](../research/gpu/probe-crash-note-2026-11-19.txt)). The renderer's GL shutdown path needs a test on this driver.

## Culling

1. Frustum: CPU, by render batches (AABB), then by mesh chunks.
2. Face directions: draw only ≤ 3 directions facing the camera (per batch).
3. Cave culling: a visibility graph between mesh chunk faces (which faces are connected by air); BFS traversal from the camera. Target ≥ 50% of units discarded in a cave.
4. Hi-Z occlusion — only on path A and only after measurements; with a second pass for what was rejected by the previous frame.

## Textures

- Texture arrays (`texture_2d_array`) instead of an atlas; a separate array for each texture size (16, 32, 64…). The default wgpu limit is 256 layers; more — several arrays or a higher limit if the adapter allows.
- Mipmaps are generated during asset conversion (not at runtime). This is the "smaller texture farther from the player" (PRD §14, No. 5). The 2×2 box filter weights color by alpha, so transparent texels (leaves, glass) do not darken edges (`assets::Rgba8Image::mip_chain`).
- `assets::TextureSet` keeps one array per size, splits at 256 layers, and loads lazily: `require(name, load)` converts a texture on first use and reports new layers (`take_added`) for upload.
- BC7 compression — only for HD packs.

## Render graph and profiles

Passes: opaque → transparent (sorted by batches from farthest) → water → entities → particles → UI. Optional: shadows, SSAO, clouds, fog. The Low profile turns optional passes off first; render distance is not below 8 chunks on any profile (PRD §14).

## Budgets

- 60 FPS on an iGPU / GTX 1050 at 12 chunks; GPU frame ≤ 8 ms for geometry (T0-06).
- Command recording CPU ≤ 1 ms per frame.
- The main unmeasured risk: vertex throughput on an iGPU (≈ 0.5 million quads = 3 million vertices per frame, estimate).
