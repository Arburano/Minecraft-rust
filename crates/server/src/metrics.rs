//! Server metrics for load runs (T2-05, T2-09): tick times, players,
//! regions, process memory and CPU time.
//!
//! The world thread appends one record per tick to a lock-free ring; a
//! reader (the harness, the periodic log line) copies what it needs.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

/// Ticks kept (20 minutes at 20 TPS).
const CAPACITY: usize = 24_000;

/// The ring of tick records.
pub struct TickLog {
    micros: Box<[AtomicU32]>,
    players: Box<[AtomicU32]>,
    next: AtomicU64,
    regions: AtomicU32,
}

impl Default for TickLog {
    fn default() -> Self {
        Self {
            micros: (0..CAPACITY).map(|_| AtomicU32::new(0)).collect(),
            players: (0..CAPACITY).map(|_| AtomicU32::new(0)).collect(),
            next: AtomicU64::new(0),
            regions: AtomicU32::new(0),
        }
    }
}

impl TickLog {
    /// Records one tick.
    pub fn push(&self, tick: Duration, players: usize, regions: usize) {
        let i = self.next.load(Ordering::Relaxed);
        let slot = (i % CAPACITY as u64) as usize;
        self.micros[slot].store(
            u32::try_from(tick.as_micros()).unwrap_or(u32::MAX),
            Ordering::Relaxed,
        );
        self.players[slot].store(players as u32, Ordering::Relaxed);
        self.regions.store(regions as u32, Ordering::Relaxed);
        self.next.store(i + 1, Ordering::Release);
    }

    /// Ticks recorded so far.
    #[must_use]
    pub fn count(&self) -> u64 {
        self.next.load(Ordering::Acquire)
    }

    /// Tick times and player counts recorded since tick `from` (as far as
    /// the ring still holds them).
    #[must_use]
    pub fn since(&self, from: u64) -> Vec<(Duration, u32)> {
        let end = self.count();
        let start = from.max(end.saturating_sub(CAPACITY as u64));
        (start..end)
            .map(|i| {
                let slot = (i % CAPACITY as u64) as usize;
                (
                    Duration::from_micros(u64::from(self.micros[slot].load(Ordering::Relaxed))),
                    self.players[slot].load(Ordering::Relaxed),
                )
            })
            .collect()
    }

    /// Regions at the last tick.
    #[must_use]
    pub fn regions(&self) -> u32 {
        self.regions.load(Ordering::Relaxed)
    }
}

/// Percentiles of tick times.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Percentiles {
    /// Samples.
    pub count: usize,
    /// Median.
    pub p50: Duration,
    /// 95th percentile.
    pub p95: Duration,
    /// 99th percentile.
    pub p99: Duration,
    /// Maximum.
    pub max: Duration,
}

impl Percentiles {
    /// Of a set of samples (nearest rank).
    #[must_use]
    pub fn of(samples: &[Duration]) -> Self {
        if samples.is_empty() {
            return Self::default();
        }
        let mut v = samples.to_vec();
        v.sort_unstable();
        let at = |p: f64| v[((v.len() - 1) as f64 * p).round() as usize];
        Self {
            count: v.len(),
            p50: at(0.5),
            p95: at(0.95),
            p99: at(0.99),
            max: at(1.0),
        }
    }

    /// `p50 / p95 / p99 / max` in milliseconds.
    #[must_use]
    pub fn line(&self) -> String {
        let ms = |d: Duration| d.as_secs_f64() * 1e3;
        format!(
            "p50 {:.2} ms, p95 {:.2} ms, p99 {:.2} ms, max {:.2} ms ({} ticks)",
            ms(self.p50),
            ms(self.p95),
            ms(self.p99),
            ms(self.max),
            self.count
        )
    }
}

/// Process memory and CPU time now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Process {
    /// Physical memory in use, bytes (if known).
    pub memory: Option<u64>,
    /// User + system CPU time since start (Linux `/proc` only).
    pub cpu: Option<Duration>,
}

impl Process {
    /// Reads the current values.
    #[must_use]
    pub fn now() -> Self {
        let memory = memory_stats::memory_stats().map(|m| m.physical_mem as u64);
        // Fields 14 and 15 of /proc/self/stat (utime, stime) in clock ticks;
        // CLK_TCK is 100 on Linux in practice.
        let cpu = std::fs::read_to_string("/proc/self/stat")
            .ok()
            .and_then(|s| {
                let rest = &s[s.rfind(')')? + 2..];
                let f: Vec<&str> = rest.split_whitespace().collect();
                let ticks = f.get(11)?.parse::<u64>().ok()? + f.get(12)?.parse::<u64>().ok()?;
                Some(Duration::from_millis(ticks * 10))
            });
        Self { memory, cpu }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_and_percentiles() {
        let log = TickLog::default();
        for i in 1..=100u64 {
            log.push(Duration::from_millis(i), 3, 1);
        }
        let all: Vec<Duration> = log.since(0).into_iter().map(|(d, _)| d).collect();
        assert_eq!(all.len(), 100);
        let p = Percentiles::of(&all);
        assert_eq!(p.p50, Duration::from_millis(51));
        assert_eq!(p.p99, Duration::from_millis(99));
        assert_eq!(p.max, Duration::from_millis(100));
        assert_eq!(log.since(90).len(), 10);
        assert_eq!(log.regions(), 1);
        let now = Process::now();
        if cfg!(target_os = "linux") {
            assert!(now.cpu.is_some() && now.memory.is_some());
        }
    }
}
