# Datapacks

A level 1 mod: everything that can be described by data is described by data. The engine compiles datapacks into registries at load time; at runtime they cost nothing.

## Structure

```text
<mod>/data/
  blocks/*.toml      items/*.toml      entities/*.toml
  recipes/*.toml     loot/*.toml       tags/*.toml
  worldgen/biomes/*.toml   worldgen/structures/*.toml   zones/*.toml
  physics/player.toml      # movement parameters (core)
<mod>/assets/
  textures/   models/   sounds/   lang/<locale>.json   shaders/*.wgsl
```

## Block

```toml
id = "copper_pipes:copper_pipe"
model = "copper_pipes:models/pipe"
textures = { all = "copper_pipes:block/copper_pipe" }
# Per face instead of `all`: top, bottom, side, north, south, east, west; `side_overlay` is drawn over the sides.
# tint = { top = "grass" }   # multiply a gray texture by a color: grass, foliage, water
hardness = 2.5
tool = "pickaxe"
opaque = false
transparent = true                      # goes into the transparent mask
light_emission = 0
light_opacity = 1                       # 0..=15, attenuation of light entering the block; default 15 if opaque, else 0
collision = "model"                     # full | none | model (physics reads only this, never the mesh)
mesher = "cubic"                        # chunk mesher, ADR-0016; only "cubic" exists (the default)
states = { facing = ["north", "south", "east", "west", "up", "down"], waterlogged = [false, true] }
components = ["copper_pipes:flow"]      # behavior from WASM
tick = "none"                           # none | random | scheduled
```

Every combination of `states` is a separate `BlockStateId`. Keep the number of states small: it multiplies.

## Zone (PRD §14, no. 11)

Needs the engine capability `zones`. Full format, rule resolution and the world's own zones: [zones.md](zones.md).

```toml
id = "core:frozen_peaks"
priority = 0
difficulty = 3
areas = [{ x = [-2048, 2047], z = [4096, 6143] }]
rules = { pvp = true }
spawns = [
  { entity = "core:stray", weight = 80, group = [2, 4], light = [0, 7] },
  { entity = "core:goat",  weight = 20, group = [1, 3] },
]
```

Zones by biome and zone structure or dungeon tables come later.

## Overrides

A mod may change another mod's entry only if it declared it as a dependency and set `override = true`. A conflict of two overrides of the same ID is a load error listing the mods.

## Validation

`engine-cli check` and the loader verify: the schema of each file, references to textures/models (with fallback assets), ID uniqueness, number ranges. An error shows the file and line.
