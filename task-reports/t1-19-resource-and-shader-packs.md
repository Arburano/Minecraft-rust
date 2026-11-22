# T1-19 — Resource, texture and shader packs

Status: the cloud part is done. Still pending: the owner tries packs on the laptop, and the GPU cost of a shader pack is measured there.

## What was done

Per ADR-0014 (owner: option 1, shader packs change the look, not geometry). Packs are files the player adds; none ship with the game. Choosing other files changes the look, as the owner asked.

**`render`:**

- `terrain::hooks`:
  - the terrain fragment shader now calls `pack_light_curve`, `pack_surface` and `pack_fog`;
  - their built-in versions keep the exact T1-11 arithmetic;
  - `compose(path, pack)` adds a pack's code and the built-in version of every hook the pack lacks;
  - `validate` runs naga, which is now a normal dependency; wgpu already uses the same naga version.
  
  `HOOK_VERSION = 1`, `MAX_OPTIONS = 8`.
- **Pack uniform** at binding 7: options and time (`pack_option`, `pack_time`).
- **`TerrainRenderer` hot swaps:** `set_textures` replaces the block textures and layer table, `set_shader_pack` validates the pack and rebuilds the pipeline, `set_pack_params` sets options and time. All three keep every uploaded batch and all light. The constructor was split into `create_block_textures`, `make_bind0` and `create_pipeline`.
- **`terrain::post::PostPass`:** a full-screen pass around the pack's `pack_post(color, uv, depth)`, reading the world rendered off screen plus its depth.

**`client`:**

- `packs`:
  - lists packs (folders or `.zip`, `pack.toml` or the community `pack.mcmeta`, `shaderpack.toml`);
  - `PackFiles` reads from either; the `TextureSource` trait covers packs and the own installation;
  - `load_shader_pack` reads options and checks the hook version and the option count.
- **`textures::build`** takes a stack of sources (packs top first, then the installation) and falls back per face. The layer size is the largest texture found (power of two, 16–256), so HD packs keep their detail. It reports per-source counts.
- **Settings:** `resource_packs` (top first), `shader_pack`, and `shader_options` per pack.
- **Menu:**
  - "Resource packs" (enable, reorder, disable; shows the folder);
  - "Shader pack" (choose, description, sliders, the GPU-code warning).
  
  Both open from Settings and from the pause screen, and apply at once in a running world.
- **App:**
  - textures are built from the stack, and the F3 line shows the count per source, the fallback count, the size and the time;
  - the shader pack is applied on world open and on change;
  - with a post pass the world renders to an off-screen target and the depth buffer is kept;
  - a failed pack prints its error, shows a notice for 8 s and keeps the built-in look;
  - `--safe-mode` turns all packs off.

**Example pack:** `docs/examples/shaderpacks/warm-glow/` (our own code): a softer light curve, warm torch light, saturation, smoother fog and a vignette, with three sliders.

**Docs:**

- `docs/modding/packs.md` (formats, stack, hook reference, versioning);
- ADR-0014 points to the implementation;
- `meshing-and-rendering.md`;
- the task file and `tasks/README.md`.

### Tests

- **`render` hooks:** the built-in hooks validate on B, Bi and C; a pack replacing two hooks keeps the third built-in; packs with type and syntax errors are rejected with text.
- **`render` post:** valid post code passes; a wrong signature or a missing `pack_post` fails.
- **`render/tests/hot_swap.rs`** (GPU; skips without an adapter, so it runs here on llvmpipe and is skipped in CI):
  - a red block, swapped to blue textures and back, gives a bit-identical frame;
  - a broken shader pack changes nothing; a valid pack draws;
  - switching back to the built-in look is bit-identical.
- **`client` packs:** folders, zips, `pack.mcmeta`, shader options, the wrong hook version.
- **`client` own_textures:** two packs stack top first, a missing texture falls through to the lower pack, a 64 px texture sets the layer size, and without packs the size is 16 px.

### Screenshots (cloud, Xvfb with software Vulkan, world "Demo", seed 7, frame 200 after the world became playable)

| Case | Result |
| --- | --- |
| No packs, against a build from before T1-19 | **0 differing pixels** (two baseline runs were also identical, so the comparison is deterministic) |
| `warm-glow` shader pack | warmer, more saturated, vignette, smoother fog |
| A generated 32 px texture pack (sand, stone, dirt) | those blocks change; grass, leaves and the rest fall through to lower layers |
| A broken shader pack | console error with the line and reason; the frame equals the built-in look |

## What went wrong

- clippy: `let_and_return` in a test, and `unnecessary_sort_by`.
- A closure lifetime problem in the GPU test; it now uses a helper function.

## Issues encountered

- **Not done in version 1:**
  - more than one post pass (ADR-0014 says "up to N"; version 1 has one);
  - linear depth or camera near/far in the post pass;
  - sky rendering as a separate hook (sky is the fog color);
  - models, sounds, translations and fonts from resource packs;
  - animated textures (first frame only);
  - BC7 compression for HD packs;
  - the "hold Shift at startup" safe mode from ADR-0014. `--safe-mode` exists; winit cannot read key state before the window exists.
- **GPU cost of shader packs** is not measured on real hardware yet (laptop step). The off-screen target and the post pass add one full-screen pass; the terrain hooks add only arithmetic.
- **In-game notice:** it shows for 8 s; the full error is printed to the console. An in-game log view would help pack authors (later).
- **Server-required packs** come in stage 3, with mod sync.
