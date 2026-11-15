# LOD

Crate: `lod`. ADR: 0011 (light on the client, LOD from the server).

## Model

- LOD n is a 16³ voxel section where a voxel = 2ⁿ blocks. A LOD n section covers (16·2ⁿ)³ blocks.
- A LOD voxel is the dominant opaque state in the 2ⁿ cube (with priority for surface blocks so grass does not become dirt). The format is the same palette section, so the mesher, gather, and rendering are the same.
- Rings (clipmap) around the player:

| Ring | Range, chunks | Level |
| --- | --- | --- |
| L0 | 0–12 (full data; 8 on Low, 16 on High) | full sections |
| L1 | 12–24 | LOD1 |
| L2 | 24–48 | LOD2 |
| L3 | 48–64 | LOD3 |

For a 64-chunk target, LOD0–LOD3 are enough. Beyond that is optional on High/Ultra.

## Where LOD comes from

- **The server computes and sends LOD** (PRD §14, No. 1). The client does not generate LOD itself: it would need the world seed, and the seed enables ore-finding cheats.
- The server keeps a LOD pyramid for the generated part of the world (+≈ 14% volume). After a block change, the L1..L3 ancestors are marked dirty and recomputed in batches in `PostTick`/background (target ≤ 1 ms total per edit).
- For the not-yet-generated part of the world, the server can serve LOD from the cheap height and biome map (see worldgen.md) without generating full chunks.
- Until LOD arrives — fog at the edge of the render distance, no holes.

## Client

- Disk LOD cache (key: world + position + level + node version). Re-entering does not re-download.
- LOD meshing with the same mesher (`mesh_lod` 0.6.4 — compare with downsampling + `fast_mesh` in T1).
- LOD light is sky only, baked into the vertex; block light is not visible far away.
- Seams between rings are "skirts" (vertical strips down along the edge). Target: 0 visible holes from 8 viewpoints (screenshot test).
- Dynamic far effects (seasons, snow) are shader parameters, not new geometry.

## Budget

LOD up to 64 chunks ≈ 4 MB of meshes (×5 margin = 20 MB) and ≈ 400 units (estimate, analysis §13.4). We do not keep full data beyond 12–16 chunks: light for 64 chunks alone would take 1.52 GiB.

## Deferred

SVDAG + raymarching (Aokana) — R&D after stage 5: no runtime editing and no transparency, needs compute.
