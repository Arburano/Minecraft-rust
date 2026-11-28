# Resource, texture and shader packs

[ADR-0014](../adr/0014-resource-and-shader-packs.md); implemented in T1-19.

Packs are optional files the player puts into their data folder: `%APPDATA%\voxel-engine`, `~/Library/Application Support/voxel-engine` or `~/.local/share/voxel-engine` (the folder is named after the working title, open question 15). The game ships no packs. In the menu, **Settings → Resource packs… / Shader pack…** (also on the pause screen in a world) enables and orders them; changes apply at once, without a restart. Start with `--safe-mode` to turn all packs off.

## Resource and texture packs

`<data>/resourcepacks/<pack>/` (a folder) or `<data>/resourcepacks/<pack>.zip`:

```text
pack.toml                     name = "…", description = "…"
assets/<namespace>/textures/<path>.png
```

- **The community format works as is:** a `pack.mcmeta` instead of `pack.toml`, with `assets/minecraft/textures/block/*.png`. A texture pack is a resource pack that only has textures.
- **The stack:** enabled packs in the player's order (the top one wins), then the player's own licensed installation (T1-14), then our fallback textures. A file missing in a pack falls through to the layer below, so partial packs work.
- **HD:** all layers take the size of the largest texture found (a power of two, 16–256 px). Smaller ones are scaled up without smoothing, so pixel art stays sharp.
- **Block textures:** the block files name the textures per face (`textures = { top, bottom, side, north/south/east/west, side_overlay, all }`) and the gray textures' tints (`tint = { … = "grass" | "foliage" | "water" }`); see [datapacks.md](datapacks.md).
- **Not read yet:** models, sounds, translations, fonts; animated textures use their first frame.

## Shader packs (hook interface version 1)

`<data>/shaderpacks/<pack>/` or `.zip`:

```text
shaderpack.toml               name, description, hook_version = 1, [[option]] sliders
shade.wgsl                    terrain hooks (any subset)
post.wgsl                     full-screen post pass (optional)
```

At least one of `shade.wgsl` and `post.wgsl` is required.

A shader pack changes how the world **looks**, not how geometry is built. Quad decoding, culling and draw submission stay the engine's, so render paths B, Bi and C and the frame budgets hold with any pack. Every file is validated when the pack is applied. On an error the game prints it (with the line) to the console, shows a notice and keeps the built-in look.

### `shaderpack.toml`

```toml
name = "Warm glow"
description = "…"
hook_version = 1          # this game: 1; another version is refused
[[option]]                # up to 8; pack_option(0) is the first
id = "warmth"
label = "Warmth"
min = 0.0
max = 1.0
default = 0.5
```

The values the player sets are stored in `settings.toml` per pack.

### Terrain hooks (`shade.wgsl`)

Define any of these; the engine's built-in version is used for the rest. The built-in versions reproduce the default look exactly.

```wgsl
fn pack_light_curve(level: f32) -> f32                       // light level 0..15 → brightness
fn pack_surface(s: Surface) -> vec3<f32>                     // the lit color of a block face
fn pack_fog(rgb: vec3<f32>, world: vec3<f32>, eye: vec3<f32>,
            sky: vec3<f32>, start: f32, end: f32) -> vec3<f32>
```

Available to pack code:

- `struct Surface { albedo: vec4<f32>, normal: vec3<f32>, world: vec3<f32>, block_light: f32, sky_light: f32, daylight: f32 }`. Light levels are 0–15; `daylight` is 0–1.
- `pack_option(i: u32) -> f32` and `pack_time() -> f32` (seconds since the world opened).
- `camera.view_proj`, `camera.position.xyz` (the eye), `camera.fog.xy` (start, end) and `camera.sky.rgb`.
- Your own helper functions, named so they do not clash with the engine's: prefix them with your pack's name.

### Post pass (`post.wgsl`)

```wgsl
fn pack_post(color: vec3<f32>, uv: vec2<f32>, depth: f32) -> vec3<f32>
```

It runs once per pixel over the rendered world, before the UI. `depth` is the raw depth buffer value (0 near … 1 far). `pack_option` and `pack_time` work here too.

### Example

[`docs/examples/shaderpacks/warm-glow/`](../examples/shaderpacks/warm-glow/): a warmer light curve, saturation, smoother fog and a vignette with three sliders. Copy the folder into `<data>/shaderpacks/` and choose it under Settings → Shader pack.

## Versioning

The hook interface has a version (`hook_version`). A change that breaks packs raises it, and packs for an older version are refused with a clear message rather than drawn wrongly. New hooks (for example shadows, reflections or water) are added in later versions with their own budgets. Hook version 2 — a full render graph with shadow, G-buffer, deferred, translucent and composite stages and vertex hooks — is proposed in [ADR-0015](../adr/0015-programmable-render-graph.md) and specified in [render-graph.md](../architecture/render-graph.md); it is not implemented yet.
