//! Epoch-budgeted component calls with region-local transactional changes.

mod state;

use std::path::Path;
use std::sync::Arc;
#[cfg(test)]
use std::sync::atomic::AtomicU64;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use wasmtime::component::{Component, InstancePre, Linker};
use wasmtime::{
    Config, Engine, InstanceAllocationStrategy, PoolingAllocationConfig, Store, UpdateDeadline,
};

/// Host-side bindings to engine:api 0.1.
#[allow(missing_docs)]
pub mod bindings {
    wasmtime::component::bindgen!({
        path: "../../wit",
        world: "mod-world",
        imports: { default: trappable },
    });
}

pub use bindings::engine::api::types::{ApiError, BlockChange, BlockPos, Event};
pub use bindings::exports::engine::api::guest::ModDecl;
use state::HostState;
pub use state::Region;

/// Failures are returned to the tick executor, never converted to panics.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A component could not be compiled, linked, instantiated or called.
    #[error("component failure: {0}")]
    Runtime(#[from] wasmtime::Error),
    /// The watchdog could not be started.
    #[error("epoch watchdog could not start: {0}")]
    Watchdog(#[from] std::io::Error),
    /// The entire write batch was discarded after a deadline overrun.
    #[error("mod exceeded its deadline by {overrun:?}")]
    Deadline {
        /// Wall-clock time between the deadline and the returned interruption.
        overrun: Duration,
        /// Number of validated block writes discarded from the interrupted hook.
        discarded_blocks: usize,
    },
    /// A rejected host write invalidated the transaction.
    #[error("transaction rejected: {0:?}")]
    Transaction(ApiError),
    /// A trapping instance must be discarded rather than called again.
    #[error("component instance was previously trapped; create another worker")]
    Trapped,
    /// A caller supplied a deadline outside the platform clock range.
    #[error("budget is outside the platform clock range")]
    InvalidBudget,
}

struct EpochWatchdog {
    stop: Arc<AtomicBool>,
    in_flight: Arc<AtomicUsize>,
    unparker: thread::Thread,
    thread: Option<JoinHandle<()>>,
    #[cfg(test)]
    ticks: Arc<AtomicU64>,
}

/// Raises the calling thread's priority for the duration of a guest call so
/// CPU-bound background load cannot preempt the worker past its deadline
/// (T0-13: the epoch check runs on the worker thread, so an off-CPU worker
/// cannot observe the watchdog's increment). Best-effort and restored on
/// every exit path; a failure keeps the caller's current priority.
struct CallPriority {
    previous: Option<thread_priority::ThreadPriority>,
}

impl CallPriority {
    fn raise() -> Self {
        let previous = thread_priority::get_current_thread_priority().ok();
        if thread_priority::set_current_thread_priority(thread_priority::ThreadPriority::Max)
            .is_err()
        {
            return Self { previous: None };
        }
        Self { previous }
    }
}

impl Drop for CallPriority {
    fn drop(&mut self) {
        if let Some(priority) = self.previous {
            let _ = priority.set_for_current();
        }
    }
}

impl EpochWatchdog {
    fn start(engine: Engine) -> Result<Self, Error> {
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let in_flight = Arc::new(AtomicUsize::new(0));
        let idle = Arc::clone(&in_flight);
        #[cfg(test)]
        let ticks = Arc::new(AtomicU64::new(0));
        #[cfg(test)]
        let tick_counter = Arc::clone(&ticks);
        let thread = thread::Builder::new()
            .name("mod-epoch".into())
            .spawn(move || {
                // Best-effort: a raised priority shortens wake latency when all
                // cores are busy; on failure (e.g. unprivileged Linux) the
                // watchdog keeps its normal priority.
                let _ = thread_priority::ThreadPriority::Max.set_for_current();
                while !stopped.load(Ordering::Acquire) {
                    // Tick only while a guest call is in flight; an idle server
                    // parks instead of waking 5,000 times per second. unpark
                    // tokens are not lost: a call starting between the load and
                    // park makes park return immediately.
                    if idle.load(Ordering::Acquire) == 0 {
                        thread::park();
                        continue;
                    }
                    // Windows sleep uses a high-resolution timer; park_timeout rounds to milliseconds.
                    thread::sleep(Duration::from_micros(200));
                    engine.increment_epoch();
                    #[cfg(test)]
                    tick_counter.fetch_add(1, Ordering::Relaxed);
                }
            })?;
        let unparker = thread.thread().clone();
        Ok(Self {
            stop,
            in_flight,
            unparker,
            thread: Some(thread),
            #[cfg(test)]
            ticks,
        })
    }

    fn enter(&self) -> InFlight {
        self.in_flight.fetch_add(1, Ordering::Release);
        self.unparker.unpark();
        InFlight(Arc::clone(&self.in_flight))
    }

    #[cfg(test)]
    fn tick_count(&self) -> u64 {
        self.ticks.load(Ordering::Relaxed)
    }
}

/// Decrements the watchdog's in-flight counter on every exit path.
struct InFlight(Arc<AtomicUsize>);

impl Drop for InFlight {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Release);
    }
}

impl Drop for EpochWatchdog {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.unparker.unpark();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// One engine and epoch watchdog shared by all worker instances.
#[derive(Clone)]
pub struct ModHost {
    engine: Engine,
    watchdog: Arc<EpochWatchdog>,
}

impl ModHost {
    /// Creates a pooled engine with epoch interruption and 32 MiB guest memories.
    pub fn new() -> Result<Self, Error> {
        let mut pool = PoolingAllocationConfig::default();
        pool.total_component_instances(16)
            .total_core_instances(64)
            .total_memories(16)
            .max_memories_per_component(1)
            .max_memory_size(32 * 1024 * 1024);
        let mut config = Config::new();
        config
            .wasm_component_model(true)
            .epoch_interruption(true)
            .wasm_backtrace_max_frames(None)
            .allocation_strategy(InstanceAllocationStrategy::Pooling(pool));
        let engine = Engine::new(&config)?;
        let watchdog = Arc::new(EpochWatchdog::start(engine.clone())?);
        Ok(Self { engine, watchdog })
    }

    /// Compiles a component once and resolves only the engine contract imports.
    pub fn load(&self, path: impl AsRef<Path>) -> Result<PreparedMod, Error> {
        let component = Component::from_file(&self.engine, path)?;
        let mut linker = Linker::new(&self.engine);
        bindings::ModWorld::add_to_linker::<_, wasmtime::component::HasSelf<_>>(
            &mut linker,
            |state| state,
        )?;
        let pre = linker.instantiate_pre(&component)?;
        Ok(PreparedMod {
            host: self.clone(),
            pre,
        })
    }
}

/// A compiled and pre-linked component, reusable across worker threads.
#[derive(Clone)]
pub struct PreparedMod {
    host: ModHost,
    pre: InstancePre<HostState>,
}

impl PreparedMod {
    /// Instantiates a component on a worker and bounds its start function as well.
    pub fn instantiate(&self, region: Region, budget: Duration) -> Result<ModWorker, Error> {
        let _priority = CallPriority::raise();
        let _in_flight = self.host.watchdog.enter();
        let mut store = Store::new(&self.host.engine, HostState::new(region));
        store.epoch_deadline_callback(|mut state| {
            let now = Instant::now();
            Ok(if now >= state.data().deadline {
                state.data_mut().interrupt_at = Some(now);
                UpdateDeadline::Interrupt
            } else {
                UpdateDeadline::Continue(1)
            })
        });
        store.data_mut().deadline = Instant::now()
            .checked_add(budget)
            .ok_or(Error::InvalidBudget)?;
        store.set_epoch_deadline(1);
        let instance = self.pre.instantiate(&mut store)?;
        let guest = bindings::ModWorld::new(&mut store, &instance)?;
        if Instant::now() >= store.data().deadline {
            return Err(Error::Deadline {
                overrun: Instant::now().duration_since(store.data().deadline),
                discarded_blocks: store.data().pending_blocks(),
            });
        }
        Ok(ModWorker {
            store,
            guest,
            trapped: false,
            _watchdog: Arc::clone(&self.host.watchdog),
        })
    }
}

/// A worker-owned Store; failed hooks never publish their staged writes.
pub struct ModWorker {
    store: Store<HostState>,
    guest: bindings::ModWorld,
    trapped: bool,
    _watchdog: Arc<EpochWatchdog>,
}

/// Timing breakdown of a call that ended in a deadline overrun (T0-13
/// diagnostics). All instants use the same monotonic clock as the deadline.
#[derive(Clone, Copy, Debug)]
pub struct InterruptionTiming {
    /// The wall-clock deadline the call exceeded.
    pub deadline: Instant,
    /// When the epoch callback returned `UpdateDeadline::Interrupt`:
    /// deadline → interrupt is the watchdog wake latency (tick granularity
    /// plus OS scheduling).
    pub interrupt: Instant,
    /// When the guest call returned to the host: interrupt → returned is
    /// the trap/unwind cost.
    pub returned: Instant,
}

impl ModWorker {
    /// Declares the mod's systems and subscriptions.
    pub fn init(&mut self, budget: Duration) -> Result<ModDecl, Error> {
        self.call(budget, |guest, store| {
            guest.engine_api_guest().call_init(store)
        })
    }

    /// Delivers a single batch of typed events, including string payloads.
    pub fn on_events(&mut self, events: &[Event], budget: Duration) -> Result<(), Error> {
        self.call(budget, |guest, store| {
            guest.engine_api_guest().call_on_events(store, events)
        })
    }

    /// Runs one declared system under a wall-clock hard budget.
    pub fn run_system(&mut self, index: u32, budget: Duration) -> Result<(), Error> {
        self.call(budget, |guest, store| {
            guest.engine_api_guest().call_run_system(store, index)
        })
    }

    /// Returns committed region state; pending writes are never visible here.
    pub fn region(&self) -> &Region {
        &self.store.data().region
    }

    /// Timing breakdown of the most recent call, if it ended in a deadline
    /// overrun. `None` after a successful call, a non-deadline trap, or before
    /// the first call.
    pub fn last_interruption(&self) -> Option<InterruptionTiming> {
        let state = self.store.data();
        Some(InterruptionTiming {
            deadline: state.deadline,
            interrupt: state.interrupt_at?,
            returned: state.returned_at?,
        })
    }

    /// Releases the instance and returns committed state, including after a trap.
    pub fn into_region(self) -> Region {
        self.store.into_data().region
    }

    fn call<T>(
        &mut self,
        budget: Duration,
        call: impl FnOnce(&bindings::ModWorld, &mut Store<HostState>) -> wasmtime::Result<T>,
    ) -> Result<T, Error> {
        engine_core::zone!("modhost::call");
        if self.trapped {
            return Err(Error::Trapped);
        }
        let deadline = Instant::now()
            .checked_add(budget)
            .ok_or(Error::InvalidBudget)?;
        let _priority = CallPriority::raise();
        let _in_flight = self._watchdog.enter();
        self.store.data_mut().begin(deadline);
        self.store.set_epoch_deadline(1);
        let result = call(&self.guest, &mut self.store);
        let finished = Instant::now();
        if result.is_err() {
            self.trapped = true;
        }
        if finished >= deadline {
            self.store.data_mut().returned_at = Some(finished);
            let discarded_blocks = self.store.data().pending_blocks();
            self.store.data_mut().discard();
            return Err(Error::Deadline {
                overrun: finished.duration_since(deadline),
                discarded_blocks,
            });
        }
        match result {
            Ok(value) => {
                self.store.data_mut().commit()?;
                Ok(value)
            }
            Err(error) => {
                self.store.data_mut().discard();
                Err(Error::Runtime(error))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watchdog_parks_while_idle_and_ticks_while_in_flight() {
        let host = ModHost::new().expect("host");
        let idle_ticks = host.watchdog.tick_count();
        thread::sleep(Duration::from_millis(50));
        assert_eq!(
            host.watchdog.tick_count(),
            idle_ticks,
            "an idle watchdog must park, not tick"
        );
        let guard = host.watchdog.enter();
        thread::sleep(Duration::from_millis(50));
        assert!(
            host.watchdog.tick_count() > idle_ticks,
            "an in-flight watchdog must tick"
        );
        drop(guard);
        let settled = host.watchdog.tick_count();
        thread::sleep(Duration::from_millis(50));
        // A tick already sleeping when the counter dropped may still land.
        assert!(host.watchdog.tick_count() <= settled + 1);
    }
}
