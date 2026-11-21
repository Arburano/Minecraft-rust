# T1-07: fallback asset set, asset cache and texture arrays

## What was done

- **`assets::fallback`:** 16×16 textures generated procedurally from their key: stone, dirt, grass block, sand, gravel, oak log and planks, coal and iron ore, cobblestone, water (alpha 180), oak leaves (with holes), glass (frame), torch, glowstone, plus a magenta/black "missing" checkerboard. They come from our own deterministic code (FNV-1a seed + xorshift), so the license is the repository's and there are no image files. `fallback_key` maps references such as `core:fallback/stone` and block ids to keys.
- **`assets::image::Rgba8Image`:** mip chains down to 1×1 with an alpha-weighted 2×2 box filter, so transparent texels do not darken edges.
- **`assets::texture_array`:**
  - `TextureArray`: one size, square power-of-two layers, all mips, at most 256 layers, laid out per mip level for upload.
  - `TextureSet`: one array per size, split at 256 layers. Lazy `require(name, load)` converts a texture once on first use, and `take_added` reports new layers for GPU upload.
- **`assets::cache`:**
  - The `.vxta` format: magic, format and converter versions, names, mip levels.
  - `AssetCache`: `<root>/<CONVERTER_VERSION>/<sha256>.vxta`, atomic write via a temporary file and rename. `get_or_convert` treats stale or other-version entries as a miss.
- **`examples/fallback_preview`:** writes all fallback textures to a BMP sheet.
- **Docs:** `auth-and-assets.md`, `meshing-and-rendering.md` (Textures), and a new `performance-budgets.md` row.

**Tests:**
- 3 unit tests: mip sizes and alpha weighting, determinism and distinctness of the fallback textures, key mapping.
- 7 integration tests:
  - the converted fallback set is byte-identical to the golden file `tests/golden/fallback-16.vxta` (22,067 bytes, sha256 `4df9a84d…ab4f9`);
  - every block in `mods/core/data/blocks` has its own look;
  - a cache round trip and hit;
  - another converter version is not read and gets reconverted;
  - truncated or extended files are rejected;
  - lazy loading runs once and reports new layers;
  - the split at 256 layers.
- All tests run on fallback assets only.

**Benchmark** (`cargo bench -p assets --bench assets`): cloud **VM**, 4 vCPU Xeon 2.1 GHz, 3 runs, 30 samples.
- Full fallback set generated and converted (16 textures, 1 array, 5 mip levels, 21,824 bytes): 39.0–42.0 µs.
- Cache entry encode: 0.45–1.36 µs (noisy).
- Decode: 1.20–1.48 µs.

## What went wrong

- The first `cargo bench -p assets -- --sample-size …` call failed because Criterion options also went to the lib test harness; `--bench assets` fixed it.

## Issues encountered

- **Fallback looks are simple placeholders** (rule 3 asks for a complete own asset set before the public release; this is the minimal set for development).
- **Sounds and block models are not covered**; the task's scope was textures (models for mobs are stage 4).
- **The renderer has not uploaded these arrays yet:** T1-11 binds `TextureSet` to wgpu `texture_2d_array`s.
- **The cache is not wired into the client yet** (T1-13 decides `<data>`), and nothing reads Mojang assets (T1-14).
- VM numbers only.
