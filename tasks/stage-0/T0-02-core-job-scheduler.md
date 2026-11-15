# T0-02. Job scheduler with priorities and cancellation

- **Status:** done
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

- [x] A high-priority job submitted behind 10 000 background jobs starts before them (test `interactive_starts_before_ten_thousand_background_jobs`).
- [x] A cancelled job does not invoke the apply callback (tests `cancelled_before_start_never_runs_and_never_applies`, `cancelled_after_start_discards_result`: cancelled jobs deliver nothing to `drain_completed`).
- [x] No `Mutex` on the hot submit path (lock-free crossbeam-channel queues, one per priority, plus a wake channel; `AtomicU64` job ids).

## Results

- Implementation: `engine_core::jobs` — `JobPool` on a dedicated `rayon::ThreadPool`, four priority queues, `CancelToken`, results via a completion channel, `submit(priority, token, FnOnce) -> JobHandle<R>` and `drain_completed()`.
- Tests: 13 unit tests + 1 doctest, green 10/10 repeated runs (priority order under load with all four priorities, cancellation before/after start, no lost results with 2 000 jobs and 4 workers, zero-thread rejection, drop with pending jobs).
- Benchmark `jobs` (criterion, VM 4 vCPU Xeon Platinum 2.5 GHz, rustc stable 1.100): ≈ 0.37 µs/job under load (`submit_complete_batch64`), ≈ 0.57 µs idle round trip (`submit_complete_single`) — target ≤ 2 µs met on the VM. Recorded in [performance-budgets.md](../../docs/process/performance-budgets.md) and [docs/research/job-scheduler-2026-11-15.md](../../docs/research/job-scheduler-2026-11-15.md); bare-metal confirmation pending as for all stage-0 VM numbers.

## Out of scope

Do not integrate with ECS — that is stage 1.
