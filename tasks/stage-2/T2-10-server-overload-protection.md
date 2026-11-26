# T2-10. Server overload protection: generation backpressure and entity limits

- **Status:** todo
- **Depends on:** T2-02, T2-03
- **Owner:** agent (cloud)

## Goal

Two failure modes seen in other Rust servers under load ([research](../../docs/research/external-projects-review-2026-11-26.md), section 1) must not happen here:

1. **Players outrunning generation.** When fast players (flight, later elytra) reach columns that are not generated or not sent yet, the server keeps its tick rate by queueing generation — and the players fly into empty space and fall below the world. Instead the server applies **backpressure to movement**: the authoritative position never enters a column that is not loaded on the server, and the client's prediction already treats unloaded cells as solid (T1-12). The player is slowed or held at the edge and sees a "loading terrain" hint; generation for that player gets the highest priority.
2. **Lag machines from cheap entities.** Falling blocks, dropped items and similar entities get per-region limits and merging (items of the same kind close together merge; falling blocks over the limit settle as blocks immediately), so a sand contraption cannot stall a region's tick.

## Deliverables

- Server-side movement clamp at loaded-column boundaries (in the movement validation of T2-03), a client notice, and a priority boost for generation in the player's movement direction.
- Per-region limits and merge rules for falling blocks and item entities, numbers in the `core` datapack (the engine knows no specific entities), defaults measured.
- `server-regions.md` and `simulation-and-ai.md` updated with the final numbers.

## Acceptance criteria

- [ ] 50 bots flying at maximum fly speed in straight lines away from spawn over an ungenerated world: none ever has a position inside an unloaded column or below the world; tick p99 ≤ 25 ms (laptop, 8 cores); the time players are held is reported.
- [ ] A scripted contraption dropping 10,000 falling blocks in one region keeps that region's tick ≤ 25 ms p99.
- [ ] Limits are visible in the server metrics (how often they were hit).

## Out of scope

Elytra and other gameplay items (stage 4); anti-cheat (stage 7).
