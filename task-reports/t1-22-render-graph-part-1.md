# T1-22 (part 1): render graph and shader-pack hook version 2

## What was done

**`render::graph`** (pure, `crates/render/src/graph/mod.rs`):

- The version 2 `shaderpack.toml` (`GraphManifest`): targets (`colortex0`…`15`, 6 formats, scale, history), `[shadow]`, `[[pass]]` (stage, file, reads, writes, buckets) and up to 32 options.
- `validate(&GraphLimits)` checks:
  - the hook version, displacement ≤ 4 blocks, limits on targets, options and passes;
  - target names and scales;
  - stages this engine version runs;
  - plain file paths;
  - writes per pass ≤ the device's attachments, equal scales within a pass, full size for geometry passes;
  - every read written earlier in the frame, or a history target, or an engine target (`depth0` after the opaque stage);
  - history targets written at most once per frame;
  - memory at the current size.

  Every error names the pass.
- Allocation works on *versions* of targets (each write starts one; a version lives until its last read). Versions of the same format and scale whose lifetimes do not overlap share a physical texture. A pass that reads and writes the same target gets two textures, and a chain of 6 composite passes over one target uses 2 textures. History targets get two persistent slots that swap each frame. `BUILTIN_DEFAULT` is the built-in Default graph.

**`render::graph::exec::GraphRenderer`:**

- Builds the passes:
  - geometry passes become terrain pipelines with the pack's hooks;
  - full-screen passes get their own bind group layouts: `PackFrame`, the sampler, the read targets by name.
- Records a frame:
  - the pack's `gbuffer_opaque` passes into their targets (or the built-in forward pass into `colortex0`);
  - `deferred`, `composite`;
  - `final` (the pack's, or a built-in copy of `colortex0` to the output);
  - the history swap.
- Sets the terrain's culling margin to `max_displacement`.

**`render::terrain`:**

- `hooks::compose_geometry_v2`: the vertex pulling of each path calls `pack_vertex` (identity if absent). `fs_pack` builds a `PackFragment` and returns the pack's `PackTargets`. The version 1 hooks stay callable.
- `hooks::compose_fullscreen`: the full-screen shader with `pack_pixel` and `pack_world_at`.
- `TerrainRenderer` gains `geometry_pipeline`, `builtin_pipeline`, `render_with` and `set_cull_margin`.
- The pack uniform now holds 32 options; the version 1 layout is unchanged.

**Client:**

- `packs::load_shader_pack` loads a `hook_version = 2` pack with all its pass files.
- `WorldView::build_graph`, `render_graph` and `drop_graph`.
- The app draws the world through the graph when one is active and resizes it with the window. Any error falls back to the built-in look with the usual notice; the console line names the pass.

**Example pack** `docs/examples/shaderpacks/sunlit-deferred`:

- a G-buffer pass (albedo, normal + sky light, block light);
- deferred sun lighting with ambient, warm block light and fog from `depth0`;
- a composite with soft tone mapping, warmth and a vignette;
- 4 options.

**Docs:** `render-graph.md` ("Implementation (T1-22 part 1)"), `packs.md` (versioning).

**Tests:**

- `graph` unit tests:
  - a deferred graph with ping-pong;
  - aliasing (6 passes → 2 textures);
  - errors naming the pass (read before write, unknown format, unimplemented stage, shadows, bad name, displacement, empty writes, path escape, version);
  - history;
  - memory;
  - a post-only pack on the built-in color;
  - the built-in Default graph.
- `hooks`: version 2 geometry on all 3 paths and a full-screen shader validate with naga; an invalid pack does not.
- `reference_scene.rs` (software Vulkan, cloud VM):
  - the built-in Default graph matches the T1-21 golden images on B, Bi and C (max channel difference 1, path C 4–10 as for the forward pass, ≤ 0.003% of pixels);
  - `sunlit-deferred` golden images (7 cameras, written from Bi, checked on B, Bi and C);
  - a `pack_vertex` lifting geometry by 0.5 blocks changes 39.2% of the `chunk_border` image;
  - a broken shader and a missing file are refused, naming `composite/c.wgsl`.
- `client`: the example version 2 pack loads with its graph and options, and the version 1 example is unchanged.
- `cargo fmt`, clippy with `-D warnings`, `cargo test --workspace`.

## What went wrong

- A first check compared a custom G-buffer pass without displacement to the forward golden images and failed on 11% of the pixels. That is the sky: G-buffer targets are cleared to transparent black, and the forward pass clears to the sky color. This is correct for a G-buffer, so the comparison was removed.
- My first T2-11 test run shared the working folder with this work in progress and compiled half-written code. T2-11 was then checked in a separate worktree.

## Issues encountered

- **Part 2 is not done:**
  - shadow maps and cascades;
  - `gbuffer_translucent` (water, glass) and `depth1`;
  - `setup`;
  - TAA jitter;
  - 8 varyings (4 for now);
  - block ids and tags in `PackVertex`;
  - GPU timestamps per stage and the over-budget notice;
  - entity boxes through graphs (not drawn while a version 2 pack is active).

  A pack using any of the stages is refused with a clear message, and the built-in look stays.
- **The sample pack has no waving leaves or shadows** (the task asked for them). Waving leaves need block tags in the vertex hook, and shadows need part 2.
- **No laptop numbers:** the frame budget on the GTX 1660 Ti with the Default graph, and command recording ≤ 1 ms p95 with the sample pack. Owner: choose "Sunlit (deferred)" in Settings → Shader pack and run `scripts\stage1_exit.ps1 -Seconds 300 -Runs 1`.
- `PackFrame` has `view_proj` and its inverse but not `view` and `proj` separately, because the camera only provides the product. Weather, biome values and eye-in-water from the spec are not there yet.
