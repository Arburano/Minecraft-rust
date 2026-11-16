//! Per-job overhead of the job scheduler (`engine_core::jobs`).
//!
//! Budget row: "Job scheduler: submit + complete overhead" in
//! `docs/process/performance-budgets.md` (target ≤ 2 µs per job).
//!
//! `submit_complete_single` measures a full submit → complete round trip from
//! an idle pool, including the cost of waking a parked worker.
//! `submit_complete_batch64` amortizes the round trip over 64 in-flight jobs:
//! divide the reported time by 64 to get the per-job overhead under load.

use std::time::{Duration, Instant};

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use engine_core::jobs::{CancelToken, JobPool, Priority};

const TIMEOUT: Duration = Duration::from_secs(10);

/// Submits one trivial job and busy-waits for its result.
fn round_trip(pool: &JobPool) -> u64 {
    let handle = pool.submit(Priority::Interactive, CancelToken::new(), |_| 1u64);
    let deadline = Instant::now() + TIMEOUT;
    loop {
        for job in pool.drain_completed() {
            if job.id() == handle.id() {
                return job.downcast::<u64>().expect("u64 result");
            }
        }
        assert!(Instant::now() < deadline, "job did not complete");
        std::hint::spin_loop();
    }
}

fn submit_complete(c: &mut Criterion) {
    let pool = JobPool::new(2).expect("pool workers");
    let mut group = c.benchmark_group("jobs");

    group.bench_function("submit_complete_single", |b| {
        b.iter(|| round_trip(&pool));
    });

    group.bench_function("submit_complete_batch64", |b| {
        b.iter_batched(
            || Vec::with_capacity(64),
            |mut ids| {
                for _ in 0..64 {
                    let handle = pool.submit(Priority::Background, CancelToken::new(), |_| 1u64);
                    ids.push(handle.id());
                }
                let mut done = 0;
                let deadline = Instant::now() + TIMEOUT;
                while done < 64 {
                    for job in pool.drain_completed() {
                        let _ = job.downcast::<u64>();
                        done += 1;
                    }
                    assert!(Instant::now() < deadline, "jobs did not complete");
                    std::hint::spin_loop();
                }
                done
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

criterion_group!(benches, submit_complete);
criterion_main!(benches);
