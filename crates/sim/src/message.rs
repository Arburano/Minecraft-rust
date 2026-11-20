//! Inter-region messages (ADR-0008): sent in `Simulate` of tick N, delivered
//! in `Exchange`, processed in `Simulate` of tick N+1.
//!
//! `Exchange` sorts by `(tick, sender_region_stable_id, sender_pos, counter)`,
//! never by queue arrival order (engine-core invariant 2). Routing is by
//! target position, so a merge or split between send and delivery does not
//! lose a message.

use engine_core::Tick;
use world::coords::SectionPos;

use crate::ids::{NetId, RegionId};
use crate::region::Body;

/// Message payload. The prototype carries entity transfers; the enum is the
/// extension point for later distant actions (teleport, commands).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Payload {
    /// Move an entity to the region owning `target_pos`; the body is copied
    /// by stable `NetId` and the source despawns it at send time.
    Transfer {
        /// Stable identity, unchanged by the transfer.
        net_id: NetId,
        /// Component state at send time.
        body: Body,
    },
}

/// One inter-region message with its deterministic sort key.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Message {
    /// Tick the message was sent in.
    pub sent_at: Tick,
    /// Stable id of the sending region.
    pub sender: RegionId,
    /// Sender section (x, z) at send time; part of the sort key.
    pub sender_pos: (i32, i32),
    /// Per-region send counter; breaks ties deterministically.
    pub counter: u64,
    /// Section the message is routed to.
    pub target_pos: SectionPos,
    /// The payload.
    pub payload: Payload,
}

impl Message {
    /// The `Exchange` sort key: `(tick, sender, sender_pos, counter)`.
    #[must_use]
    pub fn sort_key(&self) -> (Tick, RegionId, (i32, i32), u64) {
        (self.sent_at, self.sender, self.sender_pos, self.counter)
    }

    /// The transferred `NetId`, if this is a transfer.
    #[must_use]
    pub fn transferred(&self) -> Option<NetId> {
        match self.payload {
            Payload::Transfer { net_id, .. } => Some(net_id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    fn msg(tick: u64, sender: u64, x: i32, counter: u64) -> Message {
        Message {
            sent_at: Tick(tick),
            sender: RegionId(sender),
            sender_pos: (x, 0),
            counter,
            target_pos: SectionPos { x, y: 0, z: 0 },
            payload: Payload::Transfer {
                net_id: NetId(1),
                body: Body {
                    position: Vec3::ZERO,
                    velocity: Vec3::ZERO,
                },
            },
        }
    }

    #[test]
    fn sort_key_orders_by_tick_sender_pos_counter() {
        let mut v = [
            msg(2, 0, 0, 0),
            msg(1, 9, 5, 3),
            msg(1, 2, 7, 0),
            msg(1, 2, 3, 9),
            msg(1, 2, 3, 1),
        ];
        v.sort_by_key(Message::sort_key);
        let keys: Vec<_> = v.iter().map(Message::sort_key).collect();
        assert_eq!(
            keys,
            vec![
                (Tick(1), RegionId(2), (3, 0), 1),
                (Tick(1), RegionId(2), (3, 0), 9),
                (Tick(1), RegionId(2), (7, 0), 0),
                (Tick(1), RegionId(9), (5, 0), 3),
                (Tick(2), RegionId(0), (0, 0), 0),
            ]
        );
    }
}
