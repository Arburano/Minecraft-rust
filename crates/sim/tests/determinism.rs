//! Repeatability (T0-09 acceptance): the same input and the same region
//! partitioning give the same state hash — across runs and across region
//! pool sizes (parallel `Simulate` must not change the outcome).

mod common;

use common::build;
use sim::fixture::bot_intents;
use sim::{Axis, RegionId, SectionRect, Tick};

/// A fixed 200-tick scenario with scripted merge/split in PostTick.
/// Returns state hashes at tick 0 and every 25 ticks.
fn scenario(threads: usize) -> Vec<u64> {
    let rect = SectionRect::new(0, 0, 31, 7).expect("valid");
    let (mut node, ids) = build(rect, 4, 200, threads);
    let initial: Vec<RegionId> = node.regions().iter().map(|r| r.id()).collect();
    let mut hashes = vec![node.state_hash()];
    let mut intents = Vec::new();
    for t in 0..200u64 {
        bot_intents(&ids, Tick(t), &mut intents);
        node.tick(&intents);
        match t {
            49 => {
                let _ = node.try_merge(initial[0], initial[1]);
            }
            99 => {
                let _ = node.try_split(initial[0], Axis::X, 8);
            }
            149 => {
                let regions = node.regions();
                if regions.len() >= 2 {
                    let _ = node.try_merge(regions[0].id(), regions[1].id());
                }
            }
            _ => {}
        }
        if t % 25 == 24 {
            hashes.push(node.state_hash());
        }
    }
    hashes
}

#[test]
fn same_input_and_partitioning_give_the_same_hash() {
    let one = scenario(1);
    let two = scenario(2);
    let four = scenario(4);
    let again = scenario(1);
    assert_eq!(one, two, "pool size must not change the state");
    assert_eq!(one, four, "pool size must not change the state");
    assert_eq!(one, again, "runs must repeat");
    assert!(one.windows(2).any(|w| w[0] != w[1]), "state must evolve");
}
