//! Property test (T0-09 acceptance): random bot movement plus merge/split
//! attempts — no entity is lost or duplicated, every entity stays inside its
//! region's territory, and the partitioning keeps tiling the arena.

mod common;

use common::{arena, build, check_conservation, check_partitioning};
use proptest::prelude::*;
use sim::fixture::bot_intents;
use sim::{Axis, SimError, SplitOutcome, Tick};
use world::SECTION_SIZE;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    #[test]
    fn movement_and_repartitioning_conserve_entities(
        bots in 1u64..=100,
        ticks in 1u64..=80,
        ops in proptest::collection::vec((0u8..4, 0usize..8, 0u8..2, 0i32..8), 0..40),
    ) {
        let (mut node, ids) = build(arena(), 2, bots, 2);
        let mut intents = Vec::new();
        let mut ops = ops.iter();
        check_conservation(&node, &ids);
        check_partitioning(&node);

        for t in 0..ticks {
            bot_intents(&ids, Tick(t), &mut intents);
            node.tick(&intents);
            check_conservation(&node, &ids);
            check_partitioning(&node);

            let Some(&(op, idx, axis_byte, off)) = ops.next() else {
                continue;
            };
            let len = node.regions().len();
            match op {
                0 if len >= 2 => {
                    let a = node.regions()[idx % len].id();
                    let b = node.regions()[(idx + 1) % len].id();
                    // NotAdjacent is a legitimate no-op for random pairs.
                    let _ = node.try_merge(a, b);
                }
                1 | 2 => {
                    let (rid, territory) = {
                        let r = &node.regions()[idx % len];
                        (r.id(), r.territory())
                    };
                    let axis = if axis_byte == 0 { Axis::X } else { Axis::Z };
                    let (lo, hi) = match axis {
                        Axis::X => (territory.min_x, territory.max_x),
                        Axis::Z => (territory.min_z, territory.max_z),
                    };
                    let span = hi - lo + 1;
                    let line = lo + 3 + off % (span - 5).max(1);
                    match node.try_split(rid, axis, line) {
                        Ok(SplitOutcome::Split(new_id)) => {
                            // The buffer invariant at the new border: no
                            // entity in the two columns next to the line.
                            for id in [rid, new_id] {
                                let r = node.region(id).expect("half exists");
                                for net_id in r.net_ids() {
                                    let b = r.body(net_id).expect("resident");
                                    let c = match axis {
                                        Axis::X => b.position.x as i32 / SECTION_SIZE,
                                        Axis::Z => b.position.z as i32 / SECTION_SIZE,
                                    };
                                    prop_assert!(c != line - 1 && c != line);
                                }
                            }
                        }
                        Ok(SplitOutcome::Deferred) => {}
                        Err(SimError::TooNarrow) => {}
                        Err(e) => prop_assert!(false, "unexpected split error: {e}"),
                    }
                }
                _ => {}
            }
            check_conservation(&node, &ids);
            check_partitioning(&node);
        }
    }
}
