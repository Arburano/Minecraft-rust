//! Tick loop, tick phases, job scheduler, time budgets, common IDs.
//!
//! Spec:
//! - `docs/architecture/engine-core.md`
//!
//! Status: skeleton. Only the shared vocabulary (phases, tick numbers, budgets)
//! exists. The scheduler and region executor are tasks T0/T1 in `tasks/`.

use std::time::Duration;

/// Simulation rate in ticks per second. Configurable per world, 20 by default.
pub const DEFAULT_TPS: u32 = 20;

/// Monotonic simulation tick number. Wraps never in practice (u64).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tick(pub u64);

impl Tick {
    /// The tick after this one.
    #[must_use]
    pub fn next(self) -> Tick {
        Tick(self.0 + 1)
    }
}

/// Phases of one simulation tick, executed strictly in this order.
///
/// A message sent during `Simulate` of tick N is handled in `Simulate` of tick N+1,
/// so a reply arrives no earlier than tick N+2. See `docs/architecture/engine-core.md`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TickPhase {
    /// Network input, player intents, scheduled commands.
    PreTick,
    /// Per-region simulation, regions run in parallel.
    Simulate,
    /// Cross-region and global messages are sorted and delivered.
    Exchange,
    /// Persistence, outgoing deltas, region merge/split.
    PostTick,
}

impl TickPhase {
    /// All phases in execution order.
    pub const ORDER: [TickPhase; 4] = [
        TickPhase::PreTick,
        TickPhase::Simulate,
        TickPhase::Exchange,
        TickPhase::PostTick,
    ];
}

/// A time budget for a system, a mod or a whole tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    /// Target time. Exceeding it is logged by the profiler.
    pub soft: Duration,
    /// Hard limit. Exceeding it interrupts the work (epoch for mods).
    pub hard: Duration,
}

impl Budget {
    /// Server tick budget: p99 must stay under 25 ms at 20 TPS (50 ms hard limit).
    pub const SERVER_TICK: Budget = Budget {
        soft: Duration::from_millis(25),
        hard: Duration::from_millis(50),
    };

    /// Length of one tick at the given rate.
    #[must_use]
    pub fn tick_length(tps: u32) -> Duration {
        Duration::from_secs(1) / tps.max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phases_are_ordered() {
        let mut sorted = TickPhase::ORDER;
        sorted.sort();
        assert_eq!(sorted, TickPhase::ORDER);
    }

    #[test]
    fn tick_length_at_20_tps_is_50_ms() {
        assert_eq!(Budget::tick_length(DEFAULT_TPS), Duration::from_millis(50));
    }

    #[test]
    fn tick_next_increments() {
        assert_eq!(Tick(41).next(), Tick(42));
    }
}
