//! Deterministic benchmark fixture: scripted wandering bodies on the flat
//! prototype world. Not gameplay — the engine does not know mobs; this is
//! the load generator for the T0-09 tick benchmark and the property tests.
//!
//! Every [`TURN_PERIOD`] ticks each body picks a new direction from a
//! process-independent hash of `(NetId, epoch)`, so the same input gives the
//! same trajectory in every run.

use engine_core::Tick;
use glam::Vec3;
use world::SECTION_SIZE;

use crate::hash::fnv_mix;
use crate::ids::NetId;
use crate::node::{RegionNode, SimError};
use crate::region::{Body, GROUND_Y};

/// Walking speed of a fixture body, blocks per second.
pub const BOT_SPEED: f32 = 4.0;

/// Ticks between direction changes.
pub const TURN_PERIOD: u64 = 32;

/// Spawns `count` bodies scattered over the arena's inner area, each in the
/// region owning its position. The partitioning must already tile the arena.
/// Returns the assigned `NetId`s (1..=count) in order.
///
/// # Errors
/// [`SimError`] if a position has no owning region or a spawn is rejected.
pub fn spawn_bots(node: &mut RegionNode, count: u64) -> Result<Vec<NetId>, SimError> {
    let arena = node.arena();
    let size = SECTION_SIZE as f32;
    let lo_x = (arena.min_x + 1) as f32 * size;
    let span_x = ((arena.max_x - arena.min_x - 1) * SECTION_SIZE) as u64;
    let lo_z = (arena.min_z + 1) as f32 * size;
    let span_z = ((arena.max_z - arena.min_z - 1) * SECTION_SIZE) as u64;
    let mut ids = Vec::with_capacity(count as usize);
    for i in 1..=count {
        let net_id = NetId(i);
        let x = lo_x + (fnv_mix(i, 1) % span_x.max(1)) as f32 + 0.5;
        let z = lo_z + (fnv_mix(i, 2) % span_z.max(1)) as f32 + 0.5;
        let velocity = direction(net_id, 0);
        let region = node
            .region_at(x as i32 / SECTION_SIZE, z as i32 / SECTION_SIZE)
            .ok_or(SimError::BadTerritory)?;
        node.spawn(
            region,
            net_id,
            Body {
                position: Vec3::new(x, GROUND_Y, z),
                velocity,
            },
        )?;
        ids.push(net_id);
    }
    Ok(ids)
}

/// Fills `out` with the direction changes of tick `tick` (empty on other
/// ticks). Reuses the caller's buffer: no per-tick allocation.
pub fn bot_intents(ids: &[NetId], tick: Tick, out: &mut Vec<(NetId, Vec3)>) {
    out.clear();
    if tick.0 % TURN_PERIOD != 0 {
        return;
    }
    let epoch = tick.0 / TURN_PERIOD;
    for &net_id in ids {
        out.push((net_id, direction(net_id, epoch)));
    }
}

fn direction(net_id: NetId, epoch: u64) -> Vec3 {
    let angle = (fnv_mix(net_id.0, epoch) % 10_000) as f32 / 10_000.0 * std::f32::consts::TAU;
    Vec3::new(angle.cos() * BOT_SPEED, 0.0, angle.sin() * BOT_SPEED)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::territory::SectionRect;

    #[test]
    fn intents_are_periodic_and_deterministic() {
        let ids = [NetId(1), NetId(2)];
        let mut a = Vec::new();
        let mut b = Vec::new();
        bot_intents(&ids, Tick(32), &mut a);
        bot_intents(&ids, Tick(32), &mut b);
        assert_eq!(a, b);
        assert_eq!(a.len(), 2);
        bot_intents(&ids, Tick(33), &mut a);
        assert!(a.is_empty());
        bot_intents(&ids, Tick(64), &mut a);
        assert_eq!(a.len(), 2);
    }

    #[test]
    fn spawn_routes_by_position() {
        let mut node =
            RegionNode::new(SectionRect::new(0, 0, 15, 7).expect("valid"), 1).expect("pool");
        let a = node
            .add_region(SectionRect::new(0, 0, 7, 7).expect("valid"))
            .expect("fits");
        let b = node
            .add_region(SectionRect::new(8, 0, 15, 7).expect("valid"))
            .expect("fits");
        let ids = spawn_bots(&mut node, 50).expect("spawn");
        assert_eq!(ids.len(), 50);
        assert_eq!(node.entity_count(), 50);
        for id in ids {
            let owner = node.owner_of(id).expect("owned");
            assert!(owner == a || owner == b);
        }
    }
}
