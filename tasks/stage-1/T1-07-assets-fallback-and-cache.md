# T1-07. Fallback asset set, asset cache and texture arrays

- **Status:** done (2026-11-21)
- **Depends on:** T0-01
- **Owner:** agent (cloud)

## Goal

The game must look like a game without any Mojang files: our own fallback textures for the `mods/core` block set, the local asset cache, and conversion to GPU texture arrays with mipmaps. Lazy loading: only what the menu and nearest blocks need at start.

## Specs

- [auth-and-assets.md](../../docs/architecture/auth-and-assets.md) — Assets
- [legal.md](../../docs/legal.md) — rules 2 and 3
- [meshing-and-rendering.md](../../docs/architecture/meshing-and-rendering.md) — Textures

## Deliverables

- Fallback textures for every block in the core block set (generated procedurally or drawn by us; license recorded), in `mods/core` assets.
- `assets` cache: key = source hash + converter version, location `<data>/cache/assets/<version>/`.
- Converter: textures → texture array layers with mipmaps; golden tests on fallback assets.
- Lazy loading API used by the client (T1-13).

## Acceptance criteria

- [x] Golden tests pass; CI uses fallback assets only.
- [x] Cache invalidates on converter version change (test).
- [x] Conversion time and memory of the full fallback set measured.

## Out of scope

Mojang asset import (T1-14), sounds beyond a placeholder, models for mobs (stage 4).
