# T1-10. Chunk loading queue with priorities

- **Status:** in-progress (2026-11-22: done in the cloud; the 3×3 ≤ 0.5 s number needs the owner's confirmation and a laptop run)
- **Depends on:** T1-01, T1-03, T0-02
- **Owner:** agent (cloud)

## Goal

Load the world around the player so the nearest chunks appear first and movement never waits: a priority queue (distance, direction of movement, height, speed) feeding generation/load → light → mesh jobs on the T0-02 scheduler, with cancellation when the player turns away.

## Specs

- [engine-core.md](../../docs/architecture/engine-core.md) — Task scheduler
- [world-storage.md](../../docs/architecture/world-storage.md) — Column and loading
- [roadmap](../../docs/process/roadmap.md) — stage 1 additions

## Deliverables

- `client::loading::Queue` (or a shared crate if the server needs it) with the priority function and unit tests.
- Pipeline stages as jobs; results applied on the main thread in budgeted batches (no frame spikes).
- Simulation test: a scripted fly-through measures time until the 3×3 / 12-radius area is ready and the number of cancelled jobs.

## Acceptance criteria

- [ ] Nearest 3×3 columns ready ≤ 0.5 s after spawn on the owner's laptop (proposed number; confirm with the owner). VM: 0.47–0.49 s; laptop run pending.
- [x] No main-thread apply batch > 2 ms (measured: ≤ 1.46 ms on a VM).
- [x] Cancellation test passes.

## Out of scope

Network column streaming (stage 2).
