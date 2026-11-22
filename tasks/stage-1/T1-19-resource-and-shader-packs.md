# T1-19. Resource, texture and shader packs

- **Status:** todo (waits for the owner's approval of ADR-0014's shader-pack boundary)
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

- [ ] With no packs the game looks exactly as before (screenshot diff = 0).
- [ ] A test texture pack changes the textures it contains and nothing else; removing it restores the old look without a restart.
- [ ] A test shader pack (for example color grading plus a different light curve) works on paths B, Bi and C; its GPU cost is measured on the laptop and shown.
- [ ] A broken shader pack shows a message and the game keeps running with the built-in look.

## Out of scope

Server-required packs and downloads (stage 3, with mod sync); shadows, reflections and volumetric hooks (a later ADR); custom vertex shaders or meshing.
