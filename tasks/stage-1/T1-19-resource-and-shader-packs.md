# T1-19. Resource, texture and shader packs

- **Status:** in progress (cloud part done; owner: try packs on the laptop, GPU cost of a shader pack)
- **Depends on:** T1-11, T1-13; shares the texture path with T1-14
- **Owner:** agent (cloud) + owner's laptop for a visual check

## Goal

Players install their own resource packs (textures, models, sounds), texture packs and shader packs. Everything is optional and replaceable by choosing other files; nothing unused ships ([ADR-0014](../../docs/adr/0014-resource-and-shader-packs.md)).

## Deliverables

- **Resource packs:**
  - `pack.toml`, folder or `.zip` packs in `<data>/resourcepacks/`;
  - a layered stack (fallback → own assets → packs in the player's order) with per-file fallthrough;
  - PNG decoding (a new dependency, justified in the PR);
  - conversion into the T1-07 cache;
  - texture arrays per size, rebuilt and hot-swapped in the renderer;
  - the community layout mapped through the mods' texture references.
- **Shader packs, version 1:**
  - `shaderpack.toml` and WGSL hook functions: surface shading, sky/fog, light curve, up to N post passes, pack options as uniforms;
  - naga validation; fallback to the built-in shaders with a message;
  - `--safe-mode`, and Shift held at startup.
- **Menu:** a "Resource packs" screen (enable, order, apply) and a "Shader pack" screen (choose, options); the selection is saved in `settings.toml`.
- **Tests:**
  - generated test packs (no third-party files): stacking order, fallthrough, a broken pack falls back, hot-swap;
  - every hook validates with naga;
  - a shader pack that fails validation does not stop the game.
- **Docs:** `docs/modding/` (pack formats, hook reference with a versioned interface), `meshing-and-rendering.md`, `auth-and-assets.md`.

## Acceptance criteria

- [x] With no packs the game looks exactly as before: the screenshot diff against a pre-T1-19 build is 0 pixels (cloud, Xvfb).
- [x] A test texture pack changes the textures it contains and nothing else (cloud screenshot: a generated 32 px pack replaces sand, stone and dirt; everything else falls through). Removing it restores the old look without a restart: `render/tests/hot_swap.rs` swaps textures live, and swapping back gives a bit-identical frame.
- [ ] A test shader pack works on paths B, Bi and C; its GPU cost is measured on the laptop and shown. Cloud: the example `warm-glow` pack renders (screenshot), every hook validates on all three paths (unit test), and live switching to a pack and back to the built-in look is bit-identical (`hot_swap`). The laptop cost is pending.
- [x] A broken shader pack shows a message and the game keeps running with the built-in look (cloud run: a console message with the line and reason, an on-screen notice, the frame identical to the built-in look).

## Laptop run (owner)

1. Copy `docs/examples/shaderpacks/warm-glow` into `%APPDATA%\voxel-engine\shaderpacks\`.
2. In the game, Esc → Shader pack… → Warm glow → Done. Compare FPS with F3, with and without the pack, and move the sliders.
3. Put any resource or texture pack you own (a folder or `.zip`) into `%APPDATA%\voxel-engine\resourcepacks\`, then Esc → Resource packs… → Enable → Done.

## Out of scope

Server-required packs and downloads (stage 3, with mod sync); shadows, reflections and volumetric hooks (a later ADR); custom vertex shaders or meshing.
