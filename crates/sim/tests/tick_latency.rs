//! On-demand tick latency measurement (T0-09), not a CI assertion:
//!
//! ```bash
//! cargo test -p sim --release --test tick_latency -- --ignored --nocapture
//! ```
//!
//! Reports p50/p90/p99/max of the full tick (PreTick + Simulate + Exchange)
//! for 100/500/2 000 bots on 1 and 8 regions. `SIM_LATENCY_TICKS` overrides
//! the measured tick count (default 1 000, plus 100 warmup ticks).

mod common;

use std::time::Instant;

use common::build;
use sim::fixture::bot_intents;
use sim::{SectionRect, Tick};

fn percentile(sorted_ms: &[f64], p: f64) -> f64 {
    let idx = ((sorted_ms.len() as f64 - 1.0) * p).round() as usize;
    sorted_ms[idx]
}

#[test]
#[ignore = "tick latency measurement: run on demand, release profile (T0-09)"]
fn tick_latency_matrix() {
    let ticks: u64 = std::env::var("SIM_LATENCY_TICKS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1_000);
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let arena = SectionRect::new(0, 0, 63, 7).expect("valid");
    println!("ticks per config: {ticks} (+100 warmup), logical CPUs: {cores}");
    println!("arena: 64 x 8 sections (1024 x 128 blocks), flat ground");
    for regions in [1usize, 8] {
        for bots in [100u64, 500, 2_000] {
            let threads = regions.min(cores);
            let (mut node, ids) = build(arena, regions, bots, threads);
            let mut intents = Vec::new();
            for t in 0..100 {
                bot_intents(&ids, Tick(t), &mut intents);
                node.tick(&intents);
            }
            let mut samples = Vec::with_capacity(ticks as usize);
            for t in 100..100 + ticks {
                bot_intents(&ids, Tick(t), &mut intents);
                let start = Instant::now();
                node.tick(&intents);
                samples.push(start.elapsed().as_secs_f64() * 1_000.0);
            }
            samples.sort_by(|a, b| a.partial_cmp(b).expect("finite samples"));
            println!(
                "{bots:>5} bots / {regions} region(s), {threads} thread(s): \
                 p50 {:.3} / p90 {:.3} / p99 {:.3} / max {:.3} ms",
                percentile(&samples, 0.50),
                percentile(&samples, 0.90),
                percentile(&samples, 0.99),
                samples[samples.len() - 1],
            );
            assert_eq!(
                node.entity_count() + node.in_flight(),
                bots as usize,
                "conserved total"
            );
        }
    }
}
