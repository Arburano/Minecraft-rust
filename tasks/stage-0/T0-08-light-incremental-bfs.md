# T0-08. Incremental light add/remove

- **Status:** done
- **Depends on:** T0-03
- **Owner:** agent
- **PR:** https://github.com/Arburano/Minecraft-rust/pull/17

## Goal

In the `light` crate: nibble arrays with a Single variant, BFS for additions, reverse BFS for removals, sky light from the heightmap, operating on a snapshot of 3×3×3 sections. Starting point — `research/bench/light`.

## Specs

- [docs/architecture/lighting.md](../../docs/architecture/lighting.md)
- [docs/adr/0011-client-light-server-lod.md](../../docs/adr/0011-client-light-server-lod.md)

## Deliverables

- API: `relight_add`, `relight_remove`, `full_relight` on `LightNeighborhood`.
- Reference — full recalculation; equality proptest.
- Benchmark: incremental add/remove, full recalculation of 3×3×3.

## Acceptance criteria

- [x] After any sequence of edits, incremental lighting is bitwise equal to a full recalculation (proptest).
- [x] Numbers recorded in performance-budgets.md; target of ≤ 100/200 µs p95 verified, or the deviation explained.

## Result

Implemented in `crates/light` (snapshot, add/remove BFS, sky run, `Nibbles` storage, naive reference, proptest, `light` bench on the T0-12 corpus). Targets met with a wide margin on a VM; see the [task report](../../task-reports/t0-08-light-incremental-bfs.md).

## Out of scope

No integration with regions.
