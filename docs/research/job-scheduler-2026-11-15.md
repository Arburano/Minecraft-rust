# Job scheduler overhead (T0-02, 2026-11-15)

First measurement of `engine_core::jobs` (rayon pool + four lock-free priority
queues, crossbeam-channel). Benchmark: `crates/engine-core/benches/jobs.rs`,
criterion group `jobs`. Budget row: "Job scheduler: submit + complete
overhead" in [`../process/performance-budgets.md`](../process/performance-budgets.md).

## Machine and versions

- Cloud VM (hypervisor flag set), **4 vCPU Intel Xeon Platinum @ 2.5 GHz**,
  33 MiB L3, 7 GiB RAM. Not bare metal; treat numbers as ±30% like the other
  VM results in [optimization-tests.md](optimization-tests.md).
- rustc/cargo **stable 1.100.0** — the pinned 1.97 (`rust-toolchain.toml`)
  could not be downloaded in the sandbox; the pin file was not changed.
- rayon 1.12.0, crossbeam-channel 0.5.17, thiserror 2.0.21, criterion 0.8.2
  (from `Cargo.lock`).
- Release profile of the workspace: `lto = "thin"`, `codegen-units = 1`,
  no `target-cpu=native`.

## Method

`cargo bench -p engine-core`, one run, 100 samples per benchmark (criterion
defaults: 3 s warm-up, 5 s measurement). Pool of 2 workers; job body returns
a `u64` constant; the measured thread busy-waits on `drain_completed()`.

- `jobs/submit_complete_single` — one full submit → complete round trip from
  an idle pool, including waking a parked worker.
- `jobs/submit_complete_batch64` — 64 jobs in flight, round trip amortized
  over the batch; per-job overhead under load = reported time / 64.

## Results

| Benchmark | Time (95% CI) | Per job | Outliers |
| --- | --- | --- | --- |
| `submit_complete_single` | 547–591 ns (point 568 ns) | ≈ 0.57 µs | 18/100 (15 high severe) |
| `submit_complete_batch64` | 23.20–23.64 µs (point 23.43 µs) | ≈ 0.37 µs | 1/100 |

## Verdict

Target ≤ 2 µs per submit+complete: **met on this VM** with ~5x margin under
load and ~3.5x margin for an idle round trip. The idle figure includes the
worker wake-up; the scheduler's own queue/token/channel work is the smaller
batch figure. Final confirmation on bare metal with a fixed frequency is
still pending (stage 0 exit criterion), as for all VM numbers.