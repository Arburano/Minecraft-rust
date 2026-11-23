# T2-07. Entity simulation LOD and asynchronous pathfinding

- **Status:** todo
- **Depends on:** T2-06
- **Owner:** agent (cloud)

## Goal

From the review of old ideas (roadmap, PRD §14):

- **Simulation LOD:** full AI within ≈ 48 blocks of a player, and decision frequency every 2nd/4th/8th tick farther away, with physics and animation kept.
- **Pathfinding:** asynchronous A* on the job pool (`Visible` priority near players), applied next tick, with a per-region job budget.

Test entities only; mobs are stage 4 gameplay in `mods/core`.

## Acceptance criteria

- [ ] 500 test entities pathing in one region keep the tick ≤ 25 ms p99, with the job budget respected (measured).
- [ ] No entity "freezes" visibly at LOD boundaries (positions stay continuous in a test).

## Out of scope

Mob behaviors, spawning tables (stage 4).
