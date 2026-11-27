# mods/

Mods shipped together with the engine. The format is described in `docs/modding/`.

The base game is split into modules by responsibility ([ADR-0017](../docs/adr/0017-modular-base-game-and-builds.md), [builds](../docs/modding/builds.md)); a build (`../builds/<id>.toml`) picks the modules a game runs with:

- `core/` — what every build shares: the common block palette and default entity limits.
- `movement/` — the player movement profile.
- `overworld/` — the overworld dimension: its generation.
- `nether/`, `end/` — planned dimensions (manifests only, content with stage 4+).
- `mobs/` — planned creatures (manifest only, content with stage 4).

All of them are written only through the public API (`wit/engine-api.wit` + datapacks). If something is missing from the API, first write an ADR and extend the API, rather than working around it.

`examples/` holds example mods for the SDK; they are not part of any build.

Third-party and experimental mods are not stored here: they live in separate repositories and are installed through the mod registry.
