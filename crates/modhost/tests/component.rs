//! Real Rust component integration and transactional interruption tests.

mod support;

use std::sync::OnceLock;
use std::time::Duration;

use engine_core::Tick;
use modhost::{BlockPos, Error, Event, ModHost, ModWorker, PreparedMod, Region};
use world::BlockStateId;

const BUDGET: Duration = Duration::from_secs(1);

// Decision: open question 11 (owner, 2026-11-19). CI asserts this functional
// bound with margin; the ≤ 1 ms p99 latency target on a quiet desktop is
// measured separately (tests/interruption_latency.rs) and reported, not
// asserted here. An OS-thread watchdog cannot guarantee 1 ms under load.
const CI_OVERRUN_BOUND: Duration = Duration::from_millis(10);

fn prepared() -> anyhow::Result<&'static PreparedMod> {
    static PREPARED: OnceLock<Result<PreparedMod, String>> = OnceLock::new();
    PREPARED
        .get_or_init(|| {
            let host = ModHost::new().map_err(|error| error.to_string())?;
            host.load(support::hello_component().map_err(|error| error.to_string())?)
                .map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(|error| anyhow::anyhow!("{error}"))
}

fn worker() -> anyhow::Result<ModWorker> {
    let region = Region::new(
        BlockPos { x: 0, y: 0, z: 0 },
        [
            ("hello:example".into(), BlockStateId(1)),
            ("hello:alternate".into(), BlockStateId(2)),
        ],
    );
    let mut worker = prepared()?.instantiate(region, BUDGET)?;
    assert_eq!(worker.init(BUDGET)?.systems.len(), 6);
    Ok(worker)
}

#[test]
fn successful_batch_matches_reference_section() -> anyhow::Result<()> {
    let mut worker = worker()?;
    worker.run_system(0, BUDGET)?;
    assert_eq!(worker.region().section().non_air(), 0);
    worker.run_system(1, BUDGET)?;
    assert_eq!(worker.region().section().non_air(), 4_000);
    for index in 0..world::SECTION_VOLUME {
        assert_eq!(
            worker.region().section().get(index),
            BlockStateId(u16::from(index < 4_000))
        );
    }
    Ok(())
}

#[test]
fn strings_in_event_batch_reach_guest_and_host_state() -> anyhow::Result<()> {
    let mut worker = worker()?;
    let events = vec![Event::Message(("hello:message".into(), vec![1, 2, 3, 4])); 1_000];
    worker.on_events(&events, BUDGET)?;
    assert_eq!(
        worker.region().component(0, 0),
        Some((17_000u64).to_le_bytes().as_slice())
    );
    Ok(())
}

#[test]
fn event_allocations_handle_empty_unicode_and_repeated_batches() -> anyhow::Result<()> {
    let mut worker = worker()?;
    for count in [0, 1, 31, 1_000, 2, 0] {
        let channel = "hello:消息";
        let payload = vec![7; 257];
        let expected = (channel.len() + payload.len()) as u64 * count as u64;
        let events = vec![Event::Message((channel.into(), payload)); count];
        worker.on_events(&events, BUDGET)?;
        assert_eq!(
            worker.region().component(0, 0),
            Some(expected.to_le_bytes().as_slice())
        );
    }
    Ok(())
}

#[test]
fn expired_call_cannot_apply_changes() -> anyhow::Result<()> {
    let mut worker = worker()?;
    assert!(matches!(
        worker.run_system(1, Duration::ZERO),
        Err(Error::Deadline { .. })
    ));
    assert_eq!(worker.region().section().non_air(), 0);
    Ok(())
}

#[test]
fn infinite_loop_is_interrupted_and_tick_continues() -> anyhow::Result<()> {
    let mut healthy = worker()?;
    let mut tick = Tick(41);
    let mut maximum_overrun = Duration::ZERO;
    for _ in 0..10 {
        let mut hung = worker()?;
        let error = hung
            .run_system(2, Duration::from_millis(10))
            .expect_err("infinite loop must be interrupted");
        let Error::Deadline {
            overrun,
            discarded_blocks,
        } = error
        else {
            panic!("expected a deadline, got {error}");
        };
        assert_eq!(
            discarded_blocks, 4_000,
            "the guest must stage the batch before looping"
        );
        maximum_overrun = maximum_overrun.max(overrun);
        assert!(
            overrun <= CI_OVERRUN_BOUND,
            "interruption returned {overrun:?} after the deadline"
        );
        assert_eq!(hung.region().section().non_air(), 0);
        assert!(matches!(hung.run_system(0, BUDGET), Err(Error::Trapped)));
        healthy.run_system(0, BUDGET)?;
        tick = tick.next();
    }
    eprintln!("10 interruption runs: maximum deadline overrun {maximum_overrun:?}");
    assert_eq!(tick, Tick(51));
    healthy.run_system(1, BUDGET)?;
    assert_eq!(healthy.region().section().non_air(), 4_000);
    Ok(())
}

#[test]
fn guest_trap_discards_block_batch() -> anyhow::Result<()> {
    let mut worker = worker()?;
    worker.run_system(5, BUDGET)?;
    assert!(matches!(
        worker.run_system(3, BUDGET),
        Err(Error::Runtime(_))
    ));
    assert_eq!(worker.region().section().get(0), BlockStateId(2));
    let mut replacement = prepared()?.instantiate(worker.into_region(), BUDGET)?;
    replacement.run_system(1, BUDGET)?;
    assert_eq!(replacement.region().section().get(0), BlockStateId(1));
    Ok(())
}

#[test]
fn rejected_write_discards_previous_batches_and_worker_remains_usable() -> anyhow::Result<()> {
    let mut worker = worker()?;
    assert!(matches!(
        worker.run_system(4, BUDGET),
        Err(Error::Transaction(_))
    ));
    assert_eq!(worker.region().section().non_air(), 0);
    worker.run_system(1, BUDGET)?;
    assert_eq!(worker.region().section().non_air(), 4_000);
    Ok(())
}
