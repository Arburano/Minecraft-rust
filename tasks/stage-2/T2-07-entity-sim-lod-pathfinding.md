# T2-07. Entity simulation LOD and asynchronous pathfinding

- **Status:** done ([report](../../task-reports/t2-07-entity-sim-lod-pathfinding.md))
- **Depends on:** T2-06
- **Owner:** agent (cloud)

## Goal

From the review of old ideas (roadmap, PRD §14):

- **Simulation LOD:** full AI within ≈ 48 blocks of a player, and decision frequency every 2nd/4th/8th tick farther away, with physics and animation kept.
- **Pathfinding:** asynchronous A* on the job pool (`Visible` priority near players), applied next tick, with a per-region job budget.

Test entities only; mobs are stage 4 gameplay in `mods/core`.

## Acceptance criteria

- [x] 500 test entities pathing in one region keep the tick ≤ 25 ms p99, with the job budget respected (measured). Cloud VM: p99 3.09 ms; ≤ 4 requests per region per tick.
- [x] No entity "freezes" visibly at LOD boundaries (positions stay continuous in a test). Largest step 0.22 blocks per tick across all bands.

## Out of scope

Mob behaviors, spawning tables (stage 4).
