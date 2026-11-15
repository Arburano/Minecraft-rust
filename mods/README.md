# mods/

Mods shipped together with the engine. The format is described in `docs/modding/`.

- `core/` — base gameplay (blocks, items, mobs, crafting, generation). Written only through the public API (`wit/engine-api.wit` + datapacks). If something is missing from the API, first write an ADR and extend the API, rather than working around it.

Third-party and experimental mods are not stored here: they live in separate repositories and are installed through the mod registry.
