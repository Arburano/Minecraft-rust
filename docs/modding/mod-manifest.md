# Mod manifest (`mod.toml`)

```toml
[mod]
id = "copper_pipes"          # [a-z0-9_], unique; namespace of all the mod's IDs
version = "0.3.1"            # semver
api = "^0.1"                 # engine:api version (WIT package)
side = "both"                # server | client | both
authors = ["Hi"]
provides = ["movement"]      # gameplay slots this mod fills (ADR-0013): "movement", "worldgen:<dimension>"
description = "Copper pipes for fluids"
license = "MIT"

[dependencies]
core = "^0.1"
fluids = { version = "^0.5", optional = true }

[permissions]                # no entry — no access
filesystem = "mod_data"      # only the mod's own data folder
network = false              # client mods: forbidden by default

[budget]
tick_ms = 0.5                # expected time per tick per region (soft); hard = 4× or the server limit
memory_mb = 32               # linear memory limit

[wasm]                       # if there is code
server = "build/server.wasm"
client = "build/client.wasm"

[[component]]                # mod components with replication (only side = both)
name = "copper_pipes:flow"
replicate = true
schema = "u16"               # schema description for the handshake hash
```

## Validation rules (`modhost` at load time, `engine-cli` at build time)

- `id` does not coincide with another mod; all IDs in a datapack have the prefix `id:` (except explicit overrides with `override = true`, only for dependencies).
- `api` is compatible with the engine (current or previous major via an adapter).
- The dependency graph has no cycles; load order is topological, ties are broken by `id`. Implemented for datapacks by `world::modset` (T1-24): a build's mods and their dependencies are resolved from the mods folder; requirements are `^a.b.c` (default), `=a.b.c` or `*`; `optional = true` dependencies are checked only when the build has that mod ([builds.md](builds.md)).
- A WASM component imports only the interfaces allowed by `[permissions]`; otherwise — a linking error with an explanation.
- `side = client` cannot have `[wasm].server`, and vice versa.
