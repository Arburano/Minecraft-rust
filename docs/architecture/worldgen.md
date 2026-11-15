# World generation

Crate: `worldgen`.

## Pipeline

```text
heightmap+biomes (cheap, 2D) ─► terrain ─► caves ─► ores ─► structures ─► decorations ─► initial light
```

- Each stage is a function `fn(&GenContext, &mut ProtoColumn)`; stages are registered by mods (`core` registers vanilla-like ones). A mod can add a stage or replace a stage by id.
- Stages that touch neighbors (structures, trees) require neighboring columns to have passed the previous stage — like chunk "statuses" in Minecraft. The generation scheduler holds a dependency graph.
- Generation is deterministic from seed and coordinates; task execution order does not affect the result.
- Noise: `noise` for the start; hot functions get a SIMD implementation after profiling.

## Game zones (PRD §14, No. 11)

- Static map layout: zone = (biomes, difficulty, spawn tables, dungeon and structure tables).
- Zone tables are compiled into fast lists when the world loads. The spawn system takes the list of the zone under a position and does not check others.
- Zone assets (mob models, sounds) are loaded by the client on approaching the zone.
- Zone ≠ threading region. A mob can cross a zone border, so its behavior works everywhere; only spawning is restricted.

## Generation range

- Only what is needed is generated: players' simulation distance + the LOD request. A priority queue (movement direction, height, speed — PRD §14, No. 4).
- World pre-generation is a standard server command; recommended in production, because on-the-fly generation is the most expensive part of the load.
- A cheap heightmap+biomes pass provides data for far LOD before full chunks are generated.

## Tests

- Determinism: one column generated in different orders and with different thread counts gives the same hash.
- Speed: a column ≤ N ms per core (the number comes after the stage 1 benchmark).
