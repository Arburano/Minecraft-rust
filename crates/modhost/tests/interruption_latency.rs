//! On-demand interruption-latency measurement (T0-13, owner decision No. 11).
//!
//! Not part of default CI: the ≤ 1 ms p99 bound is a measured latency target
//! on a quiet desktop, reported in performance-budgets, not a CI assertion.
//! Run quiet and under load (all logical CPUs busy, e.g. with
//! `cargo run -p modhost --release --example cpu_load` in another terminal):
//!
//! ```text
//! cargo test -p modhost --release --test interruption_latency -- --ignored --nocapture
//! ```
//!
//! Environment: `MODHOST_LATENCY_COUNT` (interruptions, default 1000),
//! `MODHOST_LATENCY_SAMPLES` (optional path; writes one raw sample per line:
//! `overrun_ns wake_ns trap_ns post_return_ns`).

#[path = "support/mod.rs"]
mod support;

use std::time::{Duration, Instant};

use modhost::{BlockPos, Error, ModHost, Region};
use world::BlockStateId;

const BUDGET: Duration = Duration::from_secs(1);
const HOOK_BUDGET: Duration = Duration::from_millis(10);

fn percentile(sorted: &[Duration], fraction: f64) -> Duration {
    let index = ((sorted.len() - 1) as f64 * fraction).round() as usize;
    sorted[index]
}

fn report_line(name: &str, samples: &mut [Duration]) -> String {
    samples.sort_unstable();
    format!(
        "{name:<34} {:>10} {:>10} {:>10} {:>10}",
        fmt(percentile(samples, 0.50)),
        fmt(percentile(samples, 0.90)),
        fmt(percentile(samples, 0.99)),
        fmt(samples[samples.len() - 1]),
    )
}

fn fmt(value: Duration) -> String {
    format!("{:.3} ms", value.as_secs_f64() * 1e3)
}

#[test]
#[ignore = "latency measurement: run on demand, quiet and under load (T0-13)"]
fn interruption_latency() -> anyhow::Result<()> {
    let count: usize = std::env::var("MODHOST_LATENCY_COUNT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1_000);
    assert!(count > 0, "MODHOST_LATENCY_COUNT must be positive");

    let host = ModHost::new()?;
    let prepared = host.load(support::hello_component()?)?;
    let mut overruns = Vec::with_capacity(count);
    let mut wakes = Vec::with_capacity(count);
    let mut traps = Vec::with_capacity(count);
    let mut post_return = Vec::with_capacity(count);
    let mut raw = String::new();
    let save_samples = std::env::var_os("MODHOST_LATENCY_SAMPLES");

    for _ in 0..count {
        let region = Region::new(
            BlockPos { x: 0, y: 0, z: 0 },
            [
                ("hello:example".into(), BlockStateId(1)),
                ("hello:alternate".into(), BlockStateId(2)),
            ],
        );
        let mut hung = prepared.instantiate(region, BUDGET)?;
        hung.init(BUDGET)?;
        let error = hung
            .run_system(2, HOOK_BUDGET)
            .expect_err("infinite loop must be interrupted");
        let observed = Instant::now();
        let Error::Deadline {
            overrun,
            discarded_blocks,
        } = error
        else {
            panic!("expected a deadline, got {error}");
        };
        assert_eq!(discarded_blocks, 4_000, "guest must stage before looping");
        assert!(overrun < Duration::from_secs(1), "hang, not an overrun");
        let timing = hung.last_interruption().expect("interruption timing");
        let wake = timing.interrupt.duration_since(timing.deadline);
        let trap = timing.returned.duration_since(timing.interrupt);
        let post = observed.duration_since(timing.returned);
        overruns.push(overrun);
        wakes.push(wake);
        traps.push(trap);
        post_return.push(post);
        if save_samples.is_some() {
            raw.push_str(&format!(
                "{} {} {} {}\n",
                overrun.as_nanos(),
                wake.as_nanos(),
                trap.as_nanos(),
                post.as_nanos()
            ));
        }
    }

    if let Some(path) = save_samples {
        std::fs::write(path, raw)?;
    }

    let cpus = std::thread::available_parallelism().map_or(0, |n| n.get());
    println!("interruption-latency measurement (T0-13, owner decision No. 11)");
    println!(
        "machine: {} {}, {cpus} logical CPUs; build profile: {}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        option_env!("PROFILE").unwrap_or("unknown"),
    );
    println!(
        "interruptions: {count}, hook budget: {HOOK_BUDGET:?}, \
         load: external (none spawned by this test)"
    );
    println!(
        "{:<34} {:>10} {:>10} {:>10} {:>10}",
        "metric", "p50", "p90", "p99", "max"
    );
    println!(
        "{}",
        report_line("deadline-to-return overrun", &mut overruns)
    );
    println!(
        "{}",
        report_line("watchdog wake (deadline->interrupt)", &mut wakes)
    );
    println!(
        "{}",
        report_line("trap/unwind (interrupt->return)", &mut traps)
    );
    println!(
        "{}",
        report_line("post-return (return->observer)", &mut post_return)
    );
    overruns.sort_unstable();
    let p99 = percentile(&overruns, 0.99);
    println!(
        "quiet-desktop target p99 <= 1 ms: {} (informational; under load this is expected to fail)",
        if p99 <= Duration::from_millis(1) {
            "MET"
        } else {
            "NOT MET"
        }
    );
    Ok(())
}
