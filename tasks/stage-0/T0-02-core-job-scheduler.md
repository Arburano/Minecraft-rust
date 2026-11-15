# T0-02. Job scheduler with priorities and cancellation

- **Status:** todo
- **Depends on:** T0-01
- **Owner:** agent

## Goal

Implement in `engine-core` a job pool on top of `rayon::ThreadPool` with four priorities (`Interactive`, `Visible`, `Prefetch`, `Background`), cancellation tokens, and returning results via a channel. Jobs operate only on data snapshots.

## Specs

- [docs/architecture/engine-core.md](../../docs/architecture/engine-core.md)

## Deliverables

- `engine_core::jobs` with API: `submit(priority, cancel_token, FnOnce) -> JobHandle`, `drain_completed()`.
- Tests: priority order under load, cancellation before/after start, no lost results.
- Benchmark: per-job overhead (target ≤ 2 µs for submit+complete; hypothesis: record the measured number in performance-budgets.md).

## Acceptance criteria

- [ ] A high-priority job submitted behind 10 000 background jobs starts before them (test).
- [ ] A cancelled job does not invoke the apply callback.
- [ ] No `Mutex` on the hot submit path (lock-free queues such as crossbeam are fine).

## Out of scope

Do not integrate with ECS — that is stage 1.
