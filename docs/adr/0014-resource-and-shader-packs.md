# 0014. Resource, texture and shader packs

Status: Accepted (owner, 2026-11-22: shader packs change the look, not geometry — option 1). Decision 2 (the shader-pack boundary) superseded by [ADR-0015](0015-programmable-render-graph.md) (2026-11-27); hook version 1 stays supported. Implemented in T1-19 (formats and hook reference: `docs/modding/packs.md`).

## Context

The owner wants players to install their own shaders, resource packs and texture packs, all optional and replaceable like movement and world generation (ADR-0013). Choosing another file changes the look, and nothing unused ships.

Constraints:

- **Performance:** paths B/Bi/C, CPU culling, the 8-byte quad format and the ≤ 8 ms geometry budget (ADR-0003, ADR-0005, T1-11) must hold whatever pack is installed. PRD §14: feel wins, so a pack must not be able to make the game stutter by accident, and there has to be a way back.
- **Safety:** packs come from strangers. GPU code cannot read files or the network, but it can hang the GPU, which the driver then resets.
- **Legal (`docs/legal.md`):** players may use packs they own, including packs based on Mojang textures. Packs never enter the repository, CI or our builds; tests use generated packs.
- **Existing pieces:** fallback assets and the texture-array cache (T1-07), the local-installation import (T1-14), and the terrain shader with its fixed bindings (T1-11).

## Decision

1. **Resource packs (textures included).**
   - **Format:** a folder or `.zip` in `<data>/resourcepacks/`, with a `pack.toml` (id, name, version, engine format version, description) and files under `assets/<namespace>/...`: textures (PNG), block models, sounds, translations, fonts. A texture pack is a resource pack that only has textures.
   - **Stacking:** the player enables and orders packs. A file in a higher pack replaces the same path in lower ones. The bottom of the stack is always our fallback assets, then the player's own licensed assets (T1-14), then enabled packs. A missing file falls through to the layer below, so a partial pack works.
   - **Loading:** packs are converted into the T1-07 cache, as texture arrays per size with mips. Changing packs in the menu rebuilds the arrays and swaps them in without a restart. Sizes may differ (16, 32, 64… with one array per size); BC7 is used only for HD packs (spec).
   - **Formats read:** our own layout, plus the common community layout (`assets/minecraft/textures/block/*.png`) mapped to our block ids through the mods' texture references. That second layout is read only from files the player supplies.
2. **Shader packs.** The proposed boundary is replacing how things look, not how geometry is produced.
   - **Format:** a folder or `.zip` in `<data>/shaderpacks/` with a `shaderpack.toml` and WGSL files that implement defined **hook functions** with fixed signatures and inputs. The engine provides the camera, time of day, light levels, the block's textures, world position and normal.
   - **Hooks, version 1:**
     - terrain surface shading (`shade(surface) -> color`), which replaces the built-in fragment code;
     - sky and fog color;
     - the light curve (level → brightness);
     - up to N full-screen **post passes** (color grading, bloom, vignette…), each reading the previous result plus depth;
     - the pack's own options (sliders and toggles in `shaderpack.toml`, passed as uniforms).
   - **Fixed by the engine:** vertex pulling and quad decoding, culling, draw submission and all bindings. So paths B/Bi/C, the batch layout and the CPU budget hold for every pack. Shadows, reflections and volumetrics come later as further hooks or passes with explicit budgets (a later ADR), not as raw pipeline access.
   - **Safety:** every file is validated with naga at load. On any error the engine falls back to the built-in shaders with a message. Holding a key (Shift) at startup or `--safe-mode` starts without packs. The menu notes that shader packs run code on the GPU and should come from trusted sources.
   - **Exactly one shader pack, or none.** The built-in look is the default and is always present (small, part of the engine).
3. **Optional and replaceable**, the same as ADR-0013. Packs are files the player (or later a server manifest, stage 3) chooses. Our builds ship only the fallback assets and the built-in shaders. A server can list required or allowed packs; the client downloads them into its cache (stage 3, with mod sync). Selections are stored in `settings.toml` (client-wide); a world or server may ask for others.

## Consequences

- **Simpler:** HD textures, other styles and visual "shaders" without engine changes; the same pipeline also serves the T1-14 own-asset import.
- **Harder:**
  - a stable, versioned shader hook interface, documented like the WIT contract;
  - texture hot-swap;
  - PNG decoding, which needs a dependency (`png`, small) to be justified in the PR;
  - pack format versioning.
- **Not possible in version 1:** custom vertex shaders, a different meshing or a full custom pipeline. If the owner wants full pipeline replacement, the cost is losing the guarantees above; a separate ADR would decide that.
- **Tasks:** T1-19 (resource packs and shader packs, version 1).

## Alternatives considered

- **Shader packs replace whole pipelines (raw WGSL for everything).** This is the most freedom, but any pack can break culling or paths B/C, or blow the frame budget, and every engine change breaks every pack.
- **Packs only as mods (WASM).** Packs are data and GPU code; a WASM sandbox adds nothing for them and needs programming.
- **Bundle popular packs.** That is a legal risk, and we would ship what is unused (against the owner's request).
