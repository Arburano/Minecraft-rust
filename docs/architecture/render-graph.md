# Render graph and shader packs, hook version 2

**Status: accepted; part 1 implemented** — [ADR-0015](../adr/0015-programmable-render-graph.md) (accepted 2026-11-27), task T1-22. See "Implementation (T1-22 part 1)" at the end for what runs today. Hook version 1 ([`../modding/packs.md`](../modding/packs.md), ADR-0014) keeps working. Crate: `render`. Related: [meshing-and-rendering.md](meshing-and-rendering.md), [ADR-0016](../adr/0016-geometry-agnostic-meshing.md) (render buckets).

## Frame

```text
 setup (once / on resize)
   │
 shadow ──────────────► shadowtex0..1 (+ shadowcolor0..1)
   │
 gbuffer_opaque ──────► colortex* (MRT) + depth0
   │   sky · terrain buckets · entities · block entities
   │
 deferred_0..N ───────► colortex* (full-screen or compute)
   │
 gbuffer_translucent ─► colortex* + depth1   (water, glass, ice, weather; reads depth0)
   │
 composite_0..N ──────► colortex* (ping-pong)
   │
 final ───────────────► swap chain ──► UI (engine, never touched by packs)
```

A stage the pack does not declare runs the engine's built-in passes. The engine's own High profile (built-in shadows, ambient occlusion) is expressed as a built-in graph of the same kind.

## What the engine keeps

| Engine | Pack |
| --- | --- |
| Quad decoding and vertex pulling (B/Bi/C), model instances, all buckets | Vertex hook: displacement and values for its fragment code |
| Frustum, face-direction and cave culling (boxes grown by `max_displacement`); shadow-caster culling from the light's frustum | — |
| Draw submission, bindings, barriers, target allocation and aliasing | Which targets exist, their formats and sizes; which passes read and write them |
| Uniforms (below), the light atlas, block textures, normal/specular textures from resource packs | Fragment hooks, full-screen and compute passes |
| GPU timestamps per stage, budget notice, safe mode | Options shown in the menu |

## `shaderpack.toml` (version 2)

```toml
name = "Example"
hook_version = 2
max_displacement = 0.5             # blocks; culling boxes grow by this

[targets]                          # up to 16
colortex0 = { format = "rgba16f" }                  # lit color
colortex1 = { format = "rgba8" }                    # albedo
colortex2 = { format = "rg16f" }                    # encoded normal
colortex3 = { format = "rgba8" }                    # roughness, metallic, emission, AO
colortex4 = { format = "rg16f", history = true }    # motion vectors + previous frame
colortex5 = { format = "rgba16f", scale = 0.5 }     # half-resolution clouds

[shadow]
resolution = 2048
cascades = 3
color = true                       # shadowcolor for tinted glass

[[pass]]                           # order inside a stage = file order
stage = "gbuffer_opaque"
file = "gbuffer_terrain.wgsl"      # pack_vertex + pack_fragment
buckets = ["voxel_quads", "model_instances"]
writes = ["colortex1", "colortex2", "colortex3", "colortex4"]

[[pass]]
stage = "deferred"
file = "lighting.wgsl"             # pack_fullscreen or pack_compute
reads = ["colortex1", "colortex2", "colortex3", "depth0", "shadowtex0"]
writes = ["colortex0"]

[[pass]]
stage = "composite"
file = "taa.wgsl"
reads = ["colortex0", "colortex4", "depth1"]
writes = ["colortex0"]

[[option]]                         # as in version 1, up to 32 in version 2
id = "wave_strength"
label = "Wave strength"
min = 0.0
max = 1.0
default = 0.5
```

The engine checks the graph at load: every read target is written earlier in the frame or has history, formats are on the allowed list (`rgba8`, `rgba16f`, `rg16f`, `r11g11b10f`, `r32f`, `rgba32f`), the total target memory fits the adapter's limits for the current resolution. Any error → the built-in look with a message naming the pass.

## Hooks

```wgsl
// geometry stages (shadow, gbuffer_opaque, gbuffer_translucent)
fn pack_vertex(v: PackVertex) -> PackVaryings
fn pack_fragment(f: PackFragment) -> PackTargets      // the targets this pass writes

// deferred and composite
fn pack_fullscreen(uv: vec2<f32>) -> PackTargets
// or a compute entry point with an engine-chosen workgroup size
fn pack_compute(pixel: vec2<u32>)

// setup
fn pack_setup(cell: vec3<u32>)                        // writes the pack's lookup textures
```

- `PackVertex`: world position, normal, tangent, UV, layer (texture), block id, bucket, block and sky light, ambient occlusion corner value.
- `PackVaryings`: displaced world position (the engine projects it), plus up to 8 `vec4` values for the fragment hook.
- Version 1 hooks (`pack_surface`, `pack_light_curve`, `pack_fog`, `pack_post`) are still accepted in a version 2 pack and run where they did before.

## Uniforms (`PackFrame`, one block for every pass)

`view`, `proj`, `view_proj` and their inverses; the same for the previous frame; camera position and previous position; TAA jitter; viewport size; frame index; sun and moon direction; shadow matrices per cascade; time of day, world time, real time; weather (rain, thunder, strength); biome temperature and humidity at the camera; eye height above the ground and an "eye in water / lava" flag; render distance; fog start, end and color; sky color; pack options.

## Profiles and budgets

| Profile | Graph | Use |
| --- | --- | --- |
| Low | forward only, hook version 1 hooks | iGPUs, the stage 1 exit target (60 FPS at 12 chunks on Vega 10) |
| Default | full graph, pack or built-in | discrete GPUs |
| High | full graph + heavier built-in passes | later: mesh shaders and ray queries only after a separate ADR with measurements |

Budgets: the frame budget does not change (`performance-budgets.md`). The built-in Default graph must meet it on the GTX 1660 Ti; a pack's cost is the pack author's, but it is measured per stage and shown. Render distance never drops below 8 chunks because of a pack.

## Testing

- Graph validation and target allocation are pure functions, tested without a GPU.
- Screenshot tests on the reference scene ([T1-21](../../tasks/stage-1/T1-21-render-reference-scene.md)): the built-in graph must reproduce the hook version 1 image within a tolerance; a sample version 2 pack (waving leaves, shadows, simple water) has golden images on software Vulkan.
- Manual check on hardware with RenderDoc for any new stage.

## Iris-format packs

Not supported. A possible later adapter (open question 17) would read the public `shaders.properties` description and GLSL programs, translate GLSL to WGSL with naga, and map Iris program names onto the stages above. It is written from the public format description only, with no code from Iris or OptiFine (`docs/legal.md`).

## Implementation (T1-22 part 1)

**What runs:**

- **`render::graph`** (pure, unit-tested): the version 2 manifest, validation and target allocation.
  - Validation checks:
    - stage names and file paths;
    - reads after writes, with history and engine targets;
    - allowed formats, equal scales per pass, full size for geometry passes;
    - attachment, target, option and pass limits;
    - memory at the current resolution.
  - Every error names the pass (`stage/file`).
  - Targets are allocated as *versions* (each write starts one), so aliasing and ping-pong fall out of lifetimes: a chain of 6 composite passes over `colortex0` uses 2 textures.
  - History targets get two persistent textures that swap each frame.
- **`render::graph::exec::GraphRenderer`**:
  - builds the passes and records a frame: the pack's `gbuffer_opaque` passes (or the built-in forward pass into `colortex0`), then `deferred`, `composite`, and `final` (the pack's, or the built-in copy of `colortex0` to the screen);
  - sets the terrain's culling margin to `max_displacement`.
- **Geometry hooks** on paths B, Bi and C:
  - `pack_vertex(PackVertex) -> PackVaryings`: displaced world position plus 4 `vec4` values (the spec allows up to 8; 4 for now);
  - `pack_fragment(PackFragment) -> PackTargets`: one output per written target;
  - the version 1 hooks (`pack_surface`, `pack_light_curve`, `pack_fog`) are callable from them.
- **Full-screen passes:**
  - `pack_fullscreen(uv) -> PackTargets`;
  - read targets are bound by name (`colortexN` as `texture_2d<f32>` with `pack_sampler`, `depth0` as `texture_depth_2d`);
  - helpers `pack_pixel(uv)` and `pack_world_at(uv, depth)`.
- **`PackFrame`** (full-screen passes):
  - view-projection and its inverse, the previous frame's view-projection and camera;
  - viewport, frame index, time, fog, sky, sun direction;
  - 32 options.

  Geometry passes see `camera` and `pack_option(0..32)`.
- **Built-in graphs:** Low is the forward path (unchanged). Default (`BUILTIN_DEFAULT`) is the built-in opaque pass into an `rgba16f` `colortex0` plus the built-in final pass.
- **Client:**
  - a `hook_version = 2` pack is loaded with all its pass files and built for the window size (rebuilt on resize);
  - any error falls back to the built-in look with a notice, and the console names the pass.

**Measured** (software Vulkan, reference scene, 320×180):

- The built-in Default graph matches the forward golden images on B, Bi and C: at most 1 level of difference, 4–10 on path C as before.
- The example pack `sunlit-deferred` has its own golden images (`sunlit-deferred-<camera>.png`) and matches them on all three paths.
- Lifting all geometry by 0.5 blocks through `pack_vertex` changes 39% of the `chunk_border` image.

**Not yet (part 2):**

- shadow maps and cascades, `setup`, `gbuffer_translucent` (water and glass), `depth1`;
- TAA jitter;
- more than 4 varyings;
- block ids and tags in `PackVertex` (waving leaves need them);
- GPU timestamps per stage in the shader-pack menu, and the over-budget notice;
- entity boxes drawn through a graph (they are not drawn while a version 2 pack is active);
- laptop numbers for the frame budget.
