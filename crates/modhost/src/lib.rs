//! Epoch-budgeted component calls with region-local transactional changes.

mod state;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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
    thread: Option<JoinHandle<()>>,
}

impl EpochWatchdog {
    fn start(engine: Engine) -> Result<Self, Error> {
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let thread = thread::Builder::new()
            .name("mod-epoch".into())
            .spawn(move || {
                while !stopped.load(Ordering::Acquire) {
                    // Windows sleep uses a high-resolution timer; park_timeout rounds to milliseconds.
                    thread::sleep(Duration::from_micros(200));
                    engine.increment_epoch();
                }
            })?;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }
}

impl Drop for EpochWatchdog {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
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
        let mut store = Store::new(&self.host.engine, HostState::new(region));
        store.epoch_deadline_callback(|state| {
            Ok(if Instant::now() >= state.data().deadline {
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

    /// Releases the instance and returns committed state, including after a trap.
    pub fn into_region(self) -> Region {
        self.store.into_data().region
    }

    fn call<T>(
        &mut self,
        budget: Duration,
        call: impl FnOnce(&bindings::ModWorld, &mut Store<HostState>) -> wasmtime::Result<T>,
    ) -> Result<T, Error> {
        if self.trapped {
            return Err(Error::Trapped);
        }
        let deadline = Instant::now()
            .checked_add(budget)
            .ok_or(Error::InvalidBudget)?;
        self.store.data_mut().begin(deadline);
        self.store.set_epoch_deadline(1);
        let result = call(&self.guest, &mut self.store);
        let finished = Instant::now();
        if result.is_err() {
            self.trapped = true;
        }
        if finished >= deadline {
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
