# 0015. Programmable render graph for shader packs (hook version 2)

Status: Proposed (2026-11-26). If accepted, it supersedes the shader-pack boundary of ADR-0014, decision 2 ("look, not geometry"); resource packs (ADR-0014, decision 1) do not change.

## Context

- ADR-0014 (accepted 2026-11-22, option 1) lets a shader pack change shading, the light curve, fog and post passes. The engine keeps all geometry and all passes.
- On 2026-11-26 the owner asked for packs that "change everything, as Iris does": waving plants and water, a curved world, shadows, PBR materials, deferred lighting, refraction and caustics, TAA, bloom, tone mapping. In Iris and OptiFine a pack controls the whole frame graph: shadow pass, G-buffer passes with several render targets, deferred passes, translucent passes, composite chain, final.
- What must still hold (ADR-0003, ADR-0005, T1-11, PRD §14): paths B/Bi/C on the same 8-byte quads, CPU culling, command recording ≤ 1 ms (p95), geometry ≤ 8 ms on the iGPU, render distance ≥ 8, a way back from a broken pack.
- Legal: no code and no shader files from Iris, OptiFine or Horizon (`docs/legal.md`). Compatibility with their pack format is a separate question (open question 17).
- Research: [external-projects-review-2026-11-26.md](../research/external-projects-review-2026-11-26.md).

## Decision

1. **The frame is a render graph described by the pack.** The engine owns a fixed list of **stages** in a fixed order; a pack (hook version 2) declares, in `shaderpack.toml`, which stages it uses, how many passes each one has, and which render targets they read and write. Stages, in order:
   1. `setup` — compute passes run once at load and on resize (lookup tables: sky scattering, 3D noise for clouds);
   2. `shadow` — scene depth (and optional color, for tinted glass) from the sun or moon, up to 4 cascades;
   3. `gbuffer_opaque` — sky, terrain, entities, block entities into **multiple render targets** (MRT): albedo, normal, material (roughness / metallic / emission, labPBR-style channels), light map, block id, motion vectors;
   4. `deferred` — up to N full-screen or compute passes after opaque geometry and before water (lighting, ambient occlusion, clouds, fog, reflections, global illumination);
   5. `gbuffer_translucent` — water, ice, glass and weather on top of the lit scene, with access to the opaque depth (refraction, absorption, caustics);
   6. `composite` — up to N full-screen passes over ping-pong color targets (bloom, exposure, TAA, tone mapping);
   7. `final` — writes the swap-chain image, before the UI.

   A stage the pack does not use runs the engine's built-in version. The built-in look of hook version 1 is one valid graph.
2. **The engine keeps geometry production.** Quad decoding, culling and draw submission stay in the engine, so B/Bi/C and the CPU budget hold for every pack. The pack plugs **vertex hooks** into the engine's vertex shader for each geometry stage (`shadow`, `gbuffer_opaque`, `gbuffer_translucent`): it receives the decoded vertex (world position, normal, UV, block id, light) and returns a displaced world position plus values passed to its fragment hook. The pack declares `max_displacement` (blocks) in the manifest; the engine grows culling boxes by it. Large values (a curved world) are allowed and cost culling efficiency, which the menu shows.
3. **Render targets are declared, not created by the pack.** Up to 16 color targets (`colortex0..15`), formats from an allowed list (`rgba8`, `rgba16f`, `rg16f`, `r11g11b10f`, `r32f`, `rgba32f`), full or fractional resolution, optional history (previous frame) for TAA. The engine allocates, aliases and clears them.
4. **Engine-provided inputs** in one uniform block for every pass: view and projection of the current and previous frame, camera position and its previous value, sun and moon direction, time of day, world time, weather, the biome at the camera, eye height and whether the eye is under water, frame index, viewport, TAA jitter, render distance and fog, pack options. The set is versioned with the hook version.
5. **Budgets are measured, not trusted.** GPU timestamps per stage are always on (TIMESTAMP_QUERY is present on the target hardware, T0-14). The pack options menu shows each stage's cost. If the frame stays over budget, the engine shows a notice and offers the built-in look; it does not switch silently (PRD §14: the player decides, but feel wins by default on the Low profile).
6. **Profiles.** Low: hook version 1 hooks only (forward, no MRT, no shadow pass) — the fast path for iGPUs. Default and High: the full graph. A later High-end option (mesh shaders, ray queries) is a separate ADR after wgpu support is measured.
7. **Safety and versioning as in ADR-0014.** Every file validated with naga at load; any error → built-in shaders with a message; `--safe-mode` and Shift at start disable packs. Hook version 1 packs keep working unchanged. The language is WGSL only; GLSL packs would go through a separate adapter (open question 17).

Specification: [`docs/architecture/render-graph.md`](../architecture/render-graph.md).

## Consequences

- **Simpler:** "full" shader packs (shadows, PBR, water, TAA) without engine changes; the same graph is the base for the engine's own High profile, so built-in shadows and SSAO are just default passes.
- **Harder:**
  - the renderer becomes a graph with transient targets and barriers instead of fixed passes (`render` crate, larger than T1-19);
  - a terrain G-buffer costs memory bandwidth on iGPUs — this is why Low stays forward;
  - motion vectors need the previous frame's transforms for entities;
  - the hook interface grows; every change needs a version bump and documentation like the WIT contract;
  - visual correctness cannot be proven by unit tests alone — a reference scene with screenshot tests is a prerequisite ([T1-21](../../tasks/stage-1/T1-21-render-reference-scene.md)).
- **Not possible even with this ADR:** replacing culling, meshing or draw submission; reading the world outside what the engine uploads; reading files or the network.
- **Tasks:** T1-21 (reference scene, can start now), T1-22 (render graph and hook version 2, starts after acceptance).

## Alternatives considered

- **Keep ADR-0014 (look only) and add passes one by one.** Safe and cheap, but each new effect (shadows, water) waits for an engine change, and packs can never deform geometry. This is what the owner asked to move away from.
- **Raw pipeline access (packs write their own vertex pulling and draws).** Maximum freedom, but any pack can break paths B/C, culling and the CPU budget, and every engine change breaks every pack (rejected in ADR-0014 for the same reasons).
- **Copy Iris's pipeline and program names one to one.** Easier for pack authors who know Iris, but ties us to GLSL and OpenGL-era conventions and is a legal risk if it drifts into copying code. The stage names above follow the same ideas so that an adapter can map Iris programs to them later.
