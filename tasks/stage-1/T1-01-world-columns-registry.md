# T1-01. World columns, block registry and ID table

- **Status:** done (2026-11-21; the map-access budget waits for the owner — see the report)
- **Depends on:** T0-03
- **Owner:** agent (cloud)

## Goal

Turn sections into a playable world store in `world`: 16×384×16 columns (24 sections + light), a world map of loaded columns owned by one thread (no `Mutex`/`RwLock` around the world), the block-state registry and the stable ID table that mods fill (the engine knows no blocks), and block access by `BlockPos` across columns.

## Specs

- [world-storage.md](../../docs/architecture/world-storage.md) — Column and loading, Registries and the ID table
- [ADR-0004](../../docs/adr/0004-sections-16-palette.md)
- [lighting.md](../../docs/architecture/lighting.md) — light stored apart under the same key

## Deliverables

- `world::Column`, `world::WorldMap` (insert/remove/get by `ColumnPos`, `get_block`/`set_block` by `BlockPos`, dirty-section tracking for meshing/light/saving).
- `world::Registry`: block states with properties needed by the engine (opacity, emission, transparency for meshing), filled at startup from data (fallback block set in `mods/core` data for now).
- Unit tests + proptest: random edits through `WorldMap` equal a naive `HashMap<BlockPos, BlockStateId>` reference.
- Benchmark: `get_block`/`set_block` through the map (budget: within 2× of section `get`/`set`).

## Acceptance criteria

- [x] Proptest against the naive reference passes.
- [x] Benchmark numbers recorded in performance-budgets (new rows), machine stated. The proposed "within 2×" is not met (get 2.8×, set 2.4×); the owner decides the budget.
- [x] No locks around the world; dependency direction as in overview.md.

## Out of scope

Saving (T1-02), generation (T1-03), networking.
