# T1-01: world columns, block registry and ID table

## What was done

- **`world::registry`:**
  - `BlockDef` is parsed from datapack TOML (`docs/modding/datapacks.md` schema; unknown fields are errors; `light_opacity` is new, defaulting to 15 for opaque blocks and 0 otherwise). `blocks_from_dir` reads a mod's `data/blocks/*.toml` in file-name order.
  - `RegistryBuilder` produces a deterministic numbering: `core:air` = 0, `core:missing` = 1, then mods sorted by id, blocks in declaration order, and states in canonical order (properties sorted by name, the last varying fastest).
  - `Registry` is immutable and shared through `Arc`. It offers `props`, `name`, `state_id`, `id_table` and `remap` (saved table → current IDs, unknown → `core:missing`). Errors: bad names, duplicates, light values out of range, more than 65,536 states, I/O.
- **`world::column`:**
  - `WorldBounds`, default y `-64..320` (24 sections).
  - `ColumnPos` and `ColumnState` (`Requested → … → Ready → Unloading`).
  - `Column`: sections plus a sky heightmap (topmost block with `light_opacity > 0`, plus one), kept current on every `set` and recomputed by `from_sections`.
- **`world::map::WorldMap`:**
  - Owned by one thread with no locks. Columns live in a direct-mapped 64×64 slot grid, with an overflow hash map (multiplicative `PosHasher`) for colliding columns; a freed slot is refilled from the overflow map.
  - `get_block`, `set_block` (validates the state, the loaded column and the height) and `height`.
  - Per-consumer dirty tracking (`Mesh` / `Light` / `Save`): each edited section is queued once; `take_dirty` drains it, and marks of removed columns are dropped.
- **`mods/core/data/blocks`:** the fallback block set used by later tasks — stone, dirt, grass, sand, gravel, oak log and planks, coal and iron ore, cobblestone, water, oak leaves, glass, torch, glowstone.
- **Docs:** `world-storage.md` (registry, columns, map, bounds, heightmap, light kept separate), `datapacks.md` (`light_opacity`), and a new `performance-budgets.md` row.

**Tests:**
- 4 registry unit tests: deterministic order independent of the order mods are added, state expansion, `remap`, error cases, and datapack parsing with the full example from `datapacks.md`.
- A proptest (256 cases, up to 300 ops) of `WorldMap` against a naive `HashMap<BlockPos, BlockStateId>` reference. Ops are random sets (including a tight cluster for overwrites and removals), loads and unloads of columns, and unknown states. It checks every block, the heightmap and the dirty lists per consumer (each section once, drained exactly).
- Unit tests: incremental heightmap equal to the recomputed one, bounds mismatch, colliding columns with overflow and promotion, and building a registry from the real `mods/core` datapack.
- **Mutation checks:**
  - Disabling the dirty de-duplication is caught.
  - Disabling the heightmap drop on removal was **not** caught by the first proptest version; the clustered set ops were added, and then it is caught.

**Benchmark** (`cargo bench -p world --bench map`): cloud **VM**, 4 vCPU Xeon 2.1 GHz, 3 runs, 30 samples. Each iteration accesses 4,096 random cells of the T0-12 corpus (seed 0, 8×8 columns × 8 sections).

| Path | Before (HashMap) | After (slot grid, inline) | Per op after |
| --- | --- | --- | --- |
| section `get` (reference) | 14.0–16.5 µs | 13.4–14.2 µs | ≈ 3.3–3.5 ns |
| map `get_block` | 80.4–85.0 µs | 37.4–39.7 µs | ≈ 9.1–9.7 ns (2.8×) |
| section `set` (reference) | 83–101 µs | 76–81 µs | ≈ 18.5–19.8 ns |
| map `set_block` (incl. heightmap and dirty) | 213–230 µs | 182–204 µs | ≈ 44–50 ns (2.4×) |

## What went wrong

- The first `WorldMap` used a `HashMap` per lookup: `get_block` was 5.3× the section `get`. The direct-mapped slot grid halved it, and storing the slots inline (no `Box`) gave a little more. The proposed relative budget ("within 2×", written by me in T1-01 as a proposal) is still not met: the remaining cost is the column lookup, the bounds and section-index checks, and for `set` the heightmap and dirty bookkeeping.
- The first state-expansion code used a hand-written odometer loop that was hard to read. It was replaced by a mixed-radix decode with an overflow check before allocating.
- The heightmap mutation was not caught at first (see the mutation checks above).

## Issues encountered

- **Budget decision for the owner:** keep "within 2×" as a target with a later optimization, or set an absolute budget (for example `get_block` ≤ 10 ns, `set_block` ≤ 60 ns on the reference laptop). Hot loops (meshing, lighting, generation) work on sections and snapshots, not on `get_block`, so the map path is used by physics, gameplay and edits.
- **Light is not in `Column`** (the task text said "24 sections + light"). `world` cannot depend on `light`; light lives under the same `SectionPos` key in the `light` crate, as `world-storage.md` says. T1-09 connects them.
- **`light_opacity`** is a new datapack field, documented in `datapacks.md`.
- **Default bounds y `-64..320`** are an engine default, configurable per dimension.
- **Two corpus conventions:** the corpus uses its own numeric test IDs; the benchmark registry pads ten states so they are valid. T1-03 replaces the corpus IDs with registry lookups.
- VM numbers only.
