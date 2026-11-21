//! Job pool with priorities and cancellation.
//!
//! Spec: `docs/architecture/engine-core.md` ("Task scheduler"), task T0-02.
//!
//! Design: a dedicated `rayon::ThreadPool` whose workers pull from four
//! lock-free priority queues (`crossbeam-channel`, one per [`Priority`]).
//! Individual jobs are not dispatched through `rayon::spawn` because rayon
//! has no priorities; instead every worker runs a loop that always takes a
//! job from the highest non-empty queue. A separate wake channel parks idle
//! workers, so the submit path takes no `Mutex`.
//!
//! Jobs work on data snapshots and never touch the live world. A job body
//! returns a result which travels back through a completion channel; the
//! owning thread takes results with [`JobPool::drain_completed`] and applies
//! them in the appropriate tick phase. Cancellation through [`CancelToken`]:
//! if the token is cancelled before the job starts, the body never runs; if
//! it is cancelled while the body runs, the result is discarded. In both
//! cases nothing is delivered, so no apply callback can fire.

use std::any::Any;
use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crossbeam_channel::{Receiver, Sender};

/// Job priorities, highest first.
///
/// See `docs/architecture/engine-core.md`: `Interactive` — re-meshing after a
/// player edit, lighting near the player; `Visible` — new chunks in frame;
/// `Prefetch` — along the movement direction; `Background` — LOD, saving.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Priority {
    /// Latency-critical work visible to the player. Runs before everything else.
    Interactive = 0,
    /// Chunks currently in the camera frame.
    Visible = 1,
    /// Chunks ahead of the movement direction.
    Prefetch = 2,
    /// Everything else: LOD, saving, maintenance.
    Background = 3,
}

impl Priority {
    /// All priorities, highest first.
    pub const ALL: [Priority; 4] = [
        Priority::Interactive,
        Priority::Visible,
        Priority::Prefetch,
        Priority::Background,
    ];
}

/// Shared cancellation flag of a job.
///
/// Clones observe the same state. Cancellation is cooperative: a worker checks
/// the token before starting the body and again before delivering the result;
/// a long-running body should poll [`CancelToken::is_cancelled`] itself.
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    /// A fresh token, not cancelled.
    #[must_use]
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    /// Cancels the job. Idempotent, never blocks.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    /// Whether [`CancelToken::cancel`] has been called on any clone.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// Unique identifier of a submitted job, assigned in submission order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JobId(u64);

/// Typed handle returned by [`JobPool::submit`].
///
/// Match [`JobHandle::id`] against [`CompletedJob::id`] when draining results,
/// then take the value with [`CompletedJob::downcast`] using the same `R`.
pub struct JobHandle<R> {
    id: JobId,
    _result: PhantomData<fn() -> R>,
}

impl<R> JobHandle<R> {
    /// Identifier of the submitted job.
    #[must_use]
    pub fn id(&self) -> JobId {
        self.id
    }
}

impl<R> Clone for JobHandle<R> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<R> Copy for JobHandle<R> {}

impl<R> std::fmt::Debug for JobHandle<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JobHandle").field("id", &self.id).finish()
    }
}

/// A finished, non-cancelled job result awaiting application.
///
/// Delivered by [`JobPool::drain_completed`] on the draining thread, never on
/// a worker. Cancelled jobs never produce a `CompletedJob`.
pub struct CompletedJob {
    id: JobId,
    result: Box<dyn Any + Send>,
}

impl CompletedJob {
    /// Identifier of the job that produced this result.
    #[must_use]
    pub fn id(&self) -> JobId {
        self.id
    }

    /// Takes the result as the type produced by the job body.
    ///
    /// Returns `None` (dropping the result) if `R` is not that type.
    #[must_use]
    pub fn downcast<R: Send + 'static>(self) -> Option<R> {
        self.result.downcast::<R>().ok().map(|boxed| *boxed)
    }
}

impl std::fmt::Debug for CompletedJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompletedJob")
            .field("id", &self.id)
            .finish()
    }
}

/// The worker thread pool could not be created.
#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    /// A pool needs at least one worker; rayon would silently use its default
    /// thread count for zero, leaving the pool without worker loops.
    #[error("a job pool needs at least one worker thread")]
    ZeroThreads,
    /// A worker thread could not be spawned.
    #[error("failed to spawn job pool workers: {0}")]
    Spawn(#[from] rayon::ThreadPoolBuildError),
}

type JobBody = Box<dyn FnOnce(&CancelToken) -> Box<dyn Any + Send> + Send>;

struct Job {
    id: JobId,
    token: CancelToken,
    body: JobBody,
}

/// State shared between the pool handle and its workers.
struct Shared {
    /// One queue per priority, indexed by `Priority as usize`.
    queues: [Receiver<Job>; 4],
    submit: [Sender<Job>; 4],
    /// One token per submitted job; wakes a parked worker.
    wake_tx: Sender<()>,
    wake_rx: Receiver<()>,
    completed_tx: Sender<CompletedJob>,
    completed_rx: Receiver<CompletedJob>,
    shutdown: AtomicBool,
}

/// A pool of worker threads executing prioritized, cancellable jobs.
///
/// Submission is lock-free and never blocks. Results are applied by the owner
/// through [`JobPool::drain_completed`], keeping all world mutation on the
/// owning thread. Dropping the pool stops the workers: running jobs finish,
/// queued jobs are dropped without running, and worker threads exit shortly
/// after (rayon does not join them on drop).
///
/// ```
/// use engine_core::jobs::{CancelToken, JobPool, Priority};
///
/// let pool = JobPool::new(2).unwrap();
/// let handle = pool.submit(Priority::Visible, CancelToken::new(), |_token| {
///     // Works on a snapshot; returns a result for the owning thread.
///     21 * 2
/// });
/// # let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
/// loop {
///     let mut done = false;
///     for job in pool.drain_completed() {
///         if job.id() == handle.id() {
///             assert_eq!(job.downcast::<i32>(), Some(42));
///             done = true;
///         }
///     }
///     if done { break; }
///     assert!(std::time::Instant::now() < deadline);
///     std::hint::spin_loop();
/// }
/// ```
pub struct JobPool {
    shared: Arc<Shared>,
    next_id: AtomicU64,
    /// Dropped after `Drop::drop` sets the shutdown flag and wakes workers.
    workers: rayon::ThreadPool,
}

impl JobPool {
    /// Creates a pool with `num_threads` workers (at least one).
    ///
    /// # Errors
    ///
    /// [`BuildError::ZeroThreads`] if `num_threads` is zero,
    /// [`BuildError::Spawn`] if a worker thread cannot be spawned.
    pub fn new(num_threads: usize) -> Result<Self, BuildError> {
        if num_threads == 0 {
            return Err(BuildError::ZeroThreads);
        }
        let (interactive_tx, interactive_rx) = crossbeam_channel::unbounded();
        let (visible_tx, visible_rx) = crossbeam_channel::unbounded();
        let (prefetch_tx, prefetch_rx) = crossbeam_channel::unbounded();
        let (background_tx, background_rx) = crossbeam_channel::unbounded();
        let (wake_tx, wake_rx) = crossbeam_channel::unbounded();
        let (completed_tx, completed_rx) = crossbeam_channel::unbounded();
        let shared = Arc::new(Shared {
            queues: [interactive_rx, visible_rx, prefetch_rx, background_rx],
            submit: [interactive_tx, visible_tx, prefetch_tx, background_tx],
            wake_tx,
            wake_rx,
            completed_tx,
            completed_rx,
            shutdown: AtomicBool::new(false),
        });
        let workers = rayon::ThreadPoolBuilder::new()
            .num_threads(num_threads)
            .thread_name(|i| format!("job-worker-{i}"))
            .build()?;
        for _ in 0..num_threads {
            let shared = Arc::clone(&shared);
            workers.spawn(move || worker_loop(&shared));
        }
        Ok(Self {
            shared,
            next_id: AtomicU64::new(0),
            workers,
        })
    }

    /// Queues a job and returns its handle immediately.
    ///
    /// Lock-free and non-blocking. The body runs on a worker thread and
    /// receives the token for cooperative cancellation. Unless the token is
    /// cancelled, the result is delivered through [`JobPool::drain_completed`].
    /// The body must not panic; a panic in the engine is a bug.
    pub fn submit<R, F>(&self, priority: Priority, token: CancelToken, body: F) -> JobHandle<R>
    where
        R: Send + 'static,
        F: FnOnce(&CancelToken) -> R + Send + 'static,
    {
        let id = JobId(self.next_id.fetch_add(1, Ordering::Relaxed));
        let job = Job {
            id,
            token,
            body: Box::new(move |token| Box::new(body(token)) as Box<dyn Any + Send>),
        };
        // Unbounded crossbeam channels: lock-free enqueue, no Mutex here.
        let _ = self.shared.submit[priority as usize].send(job);
        let _ = self.shared.wake_tx.send(());
        JobHandle {
            id,
            _result: PhantomData,
        }
    }

    /// Takes all results completed since the last drain, oldest first.
    ///
    /// Call from the owning thread (for example in `PostTick`) and apply the
    /// results there; workers never apply anything. Cancelled jobs produce
    /// nothing. The iterator is lazy and allocates nothing itself.
    pub fn drain_completed(&self) -> impl Iterator<Item = CompletedJob> + '_ {
        std::iter::from_fn(move || self.shared.completed_rx.try_recv().ok())
    }
}

impl Drop for JobPool {
    fn drop(&mut self) {
        self.shared.shutdown.store(true, Ordering::Relaxed);
        // Wake every parked worker so it observes the flag and exits.
        for _ in 0..self.workers.current_num_threads() {
            let _ = self.shared.wake_tx.send(());
        }
        // The `workers` field is dropped right after this, signalling rayon
        // termination; workers observe the flag above and exit their loops.
    }
}

/// Worker main loop: always take a job from the highest non-empty queue.
fn worker_loop(shared: &Shared) {
    loop {
        if shared.shutdown.load(Ordering::Relaxed) {
            return;
        }
        let job = shared.queues.iter().find_map(|rx| rx.try_recv().ok());
        match job {
            Some(job) => run_job(shared, job),
            None => {
                // Idle: park until a submit (or shutdown) wakes us. Every
                // submit sends one wake token after its job, so a job cannot
                // be left queued with all workers parked.
                if shared.wake_rx.recv().is_err() {
                    return;
                }
            }
        }
    }
}

fn run_job(shared: &Shared, job: Job) {
    if job.token.is_cancelled() {
        return; // Cancelled before start: the body never runs.
    }
    crate::zone!("jobs::run");
    let result = (job.body)(&job.token);
    if job.token.is_cancelled() {
        return; // Cancelled while running: discard the result.
    }
    let _ = shared
        .completed_tx
        .send(CompletedJob { id: job.id, result });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::{Sender, unbounded};
    use std::collections::{HashMap, HashSet};
    use std::time::{Duration, Instant};

    const TIMEOUT: Duration = Duration::from_secs(30);

    /// Occupies every worker with a gate job until the test releases it, so
    /// queueing order is deterministic. Returns the release sender; send one
    /// token per gated worker. Gate tokens are cancelled after start, so gate
    /// jobs deliver no results.
    fn gate_workers(pool: &JobPool, workers: usize) -> Sender<()> {
        let (release_tx, release_rx) = unbounded::<()>();
        let (started_tx, started_rx) = unbounded::<()>();
        let mut tokens = Vec::new();
        for _ in 0..workers {
            let release_rx = release_rx.clone();
            let started_tx = started_tx.clone();
            let token = CancelToken::new();
            tokens.push(token.clone());
            pool.submit(Priority::Interactive, token, move |_| {
                started_tx.send(()).expect("test receiver alive");
                release_rx.recv().expect("test releases the gate");
            });
        }
        for _ in 0..workers {
            started_rx.recv_timeout(TIMEOUT).expect("gate job starts");
        }
        for token in tokens {
            token.cancel();
        }
        release_tx
    }

    /// Drains until `n` results arrive; fails on timeout (lost results).
    fn drain_n(pool: &JobPool, n: usize) -> Vec<CompletedJob> {
        let deadline = Instant::now() + TIMEOUT;
        let mut out = Vec::new();
        while out.len() < n && Instant::now() < deadline {
            out.extend(pool.drain_completed());
            std::thread::yield_now();
        }
        assert_eq!(out.len(), n, "results lost or timed out");
        out
    }

    #[test]
    fn priority_order_is_highest_first() {
        let mut sorted = Priority::ALL;
        sorted.sort();
        assert_eq!(sorted, Priority::ALL);
        assert!(Priority::Interactive < Priority::Visible);
        assert!(Priority::Visible < Priority::Prefetch);
        assert!(Priority::Prefetch < Priority::Background);
    }

    #[test]
    fn cancel_token_clones_share_state() {
        let a = CancelToken::new();
        let b = a.clone();
        assert!(!a.is_cancelled());
        b.cancel();
        assert!(a.is_cancelled());
        a.cancel(); // idempotent
        assert!(b.is_cancelled());
    }

    #[test]
    fn interactive_starts_before_ten_thousand_background_jobs() {
        let pool = JobPool::new(1).expect("one worker");
        let release = gate_workers(&pool, 1);

        let (order_tx, order_rx) = unbounded::<&'static str>();
        for _ in 0..10_000 {
            let tx = order_tx.clone();
            pool.submit(Priority::Background, CancelToken::new(), move |_| {
                tx.send("background").expect("test receiver alive");
            });
        }
        let tx = order_tx.clone();
        pool.submit(Priority::Interactive, CancelToken::new(), move |_| {
            tx.send("interactive").expect("test receiver alive");
        });
        drop(order_tx);

        release.send(()).expect("release the gate");

        // The first job to start after the gate must be the interactive one.
        assert_eq!(
            order_rx.recv_timeout(TIMEOUT).expect("probe starts"),
            "interactive"
        );
        for _ in 0..10_000 {
            assert_eq!(
                order_rx.recv_timeout(TIMEOUT).expect("background starts"),
                "background"
            );
        }
        assert!(order_rx.try_recv().is_err());
    }

    #[test]
    fn all_priorities_run_highest_first_under_load() {
        let pool = JobPool::new(1).expect("one worker");
        let release = gate_workers(&pool, 1);

        let (order_tx, order_rx) = unbounded::<Priority>();
        // Queue in reverse priority order, several jobs per priority.
        for &priority in Priority::ALL.iter().rev() {
            for _ in 0..50 {
                let tx = order_tx.clone();
                pool.submit(priority, CancelToken::new(), move |_| {
                    tx.send(priority).expect("test receiver alive");
                });
            }
        }
        drop(order_tx);
        release.send(()).expect("release the gate");

        for &priority in &Priority::ALL {
            for _ in 0..50 {
                assert_eq!(
                    order_rx.recv_timeout(TIMEOUT).expect("job starts"),
                    priority
                );
            }
        }
        assert!(order_rx.try_recv().is_err());
    }

    #[test]
    fn cancelled_before_start_never_runs_and_never_applies() {
        let pool = JobPool::new(1).expect("one worker");
        let release = gate_workers(&pool, 1);

        let ran = Arc::new(AtomicBool::new(false));
        let ran_in_body = Arc::clone(&ran);
        let token = CancelToken::new();
        let cancelled = pool.submit(Priority::Background, token.clone(), move |_| {
            ran_in_body.store(true, Ordering::SeqCst);
            7u32
        });
        token.cancel();
        // A sentinel behind the cancelled job proves the worker passed it.
        let sentinel = pool.submit(Priority::Background, CancelToken::new(), |_| 1u32);
        release.send(()).expect("release the gate");

        let mut applied = 0;
        let mut saw_sentinel = false;
        let deadline = Instant::now() + TIMEOUT;
        while !saw_sentinel && Instant::now() < deadline {
            for job in pool.drain_completed() {
                // The apply callback: must never fire for the cancelled job.
                applied += 1;
                assert_eq!(job.id(), sentinel.id(), "cancelled job was applied");
                assert_ne!(job.id(), cancelled.id());
                saw_sentinel = true;
            }
            std::thread::yield_now();
        }
        assert!(saw_sentinel, "sentinel lost");
        assert_eq!(applied, 1);
        assert!(!ran.load(Ordering::SeqCst), "cancelled body ran");
    }

    #[test]
    fn cancelled_after_start_discards_result() {
        let pool = JobPool::new(1).expect("one worker");
        let (started_tx, started_rx) = unbounded::<()>();
        let (finish_tx, finish_rx) = unbounded::<()>();
        let token = CancelToken::new();
        let cancelled = pool.submit(Priority::Interactive, token.clone(), move |token| {
            started_tx.send(()).expect("test receiver alive");
            finish_rx.recv().expect("test lets the body finish");
            assert!(token.is_cancelled(), "cooperative check inside the body");
            42u32
        });
        started_rx.recv_timeout(TIMEOUT).expect("job starts");

        token.cancel(); // cancelled while the body is running
        finish_tx.send(()).expect("let the body finish");
        let sentinel = pool.submit(Priority::Interactive, CancelToken::new(), |_| 0u32);

        let results = drain_n(&pool, 1);
        assert_eq!(results[0].id(), sentinel.id(), "cancelled result delivered");
        assert_ne!(results[0].id(), cancelled.id());
        // Give a stray cancelled result a chance to surface: nothing may come.
        let deadline = Instant::now() + Duration::from_millis(200);
        while Instant::now() < deadline {
            assert!(pool.drain_completed().next().is_none());
            std::thread::yield_now();
        }
    }

    #[test]
    fn no_results_lost_under_load() {
        const N: usize = 2_000;
        const WORKERS: usize = 4;
        let pool = JobPool::new(WORKERS).expect("four workers");
        let release = gate_workers(&pool, WORKERS);

        let mut expected_values = HashMap::new();
        let mut cancelled_ids = HashSet::new();
        for i in 0..N {
            let priority = Priority::ALL[i % 4];
            let token = CancelToken::new();
            let value = i as u64;
            let handle = pool.submit(priority, token.clone(), move |token| {
                assert!(!token.is_cancelled(), "cancelled body ran");
                value * 2
            });
            if i % 7 == 0 {
                token.cancel();
                cancelled_ids.insert(handle.id());
            } else {
                expected_values.insert(handle.id(), value * 2);
            }
        }
        for _ in 0..WORKERS {
            release.send(()).expect("release the gate");
        }

        // Drain until the expected count plus a quiet period, so extra
        // (cancelled) results would also be caught.
        let deadline = Instant::now() + TIMEOUT;
        let mut got = Vec::new();
        let mut last_arrival = Instant::now();
        while Instant::now() < deadline {
            let before = got.len();
            got.extend(pool.drain_completed());
            if got.len() != before {
                last_arrival = Instant::now();
            }
            if got.len() >= expected_values.len()
                && last_arrival.elapsed() > Duration::from_millis(200)
            {
                break;
            }
            std::thread::yield_now();
        }
        assert_eq!(got.len(), expected_values.len(), "lost or extra results");
        for job in got {
            let id = job.id();
            assert!(!cancelled_ids.contains(&id));
            let value = job.downcast::<u64>().expect("u64 result");
            assert_eq!(Some(value), expected_values.remove(&id));
        }
        assert!(expected_values.is_empty(), "some results never arrived");
    }

    #[test]
    fn downcast_returns_none_on_wrong_type() {
        let pool = JobPool::new(1).expect("one worker");
        let number = pool.submit(Priority::Visible, CancelToken::new(), |_| 5u64);
        let text = pool.submit(Priority::Visible, CancelToken::new(), |_| {
            String::from("mesh")
        });
        let results = drain_n(&pool, 2);
        for job in results {
            if job.id() == number.id() {
                assert_eq!(job.downcast::<u64>(), Some(5));
            } else {
                assert_eq!(job.id(), text.id());
                assert!(job.downcast::<u64>().is_none());
            }
        }
    }

    #[test]
    fn zero_threads_is_rejected() {
        assert!(JobPool::new(0).is_err());
    }

    #[test]
    fn drop_with_pending_jobs_terminates() {
        for _ in 0..8 {
            let pool = JobPool::new(2).expect("two workers");
            for _ in 0..100 {
                pool.submit(Priority::Background, CancelToken::new(), |_| 0u8);
            }
            drop(pool);
        }
    }
}
