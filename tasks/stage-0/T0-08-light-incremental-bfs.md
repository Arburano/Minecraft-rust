# T0-08. Incremental light add/remove

- **Status:** todo
- **Depends on:** T0-03
- **Owner:** agent

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

- [ ] After any sequence of edits, incremental lighting is bitwise equal to a full recalculation (proptest).
- [ ] Numbers recorded in performance-budgets.md; target of ≤ 100/200 µs p95 verified, or the deviation explained.

## Out of scope

No integration with regions.
