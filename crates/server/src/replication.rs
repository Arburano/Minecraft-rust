//! Entity deltas per client (T2-03, `networking-protocol.md`, "Entity deltas").
//!
//! For each client the server keeps the snapshots it sent: what the client
//! knows after applying each one. A new datagram is a delta against the
//! newest snapshot the client acknowledged. Entities that do not fit the
//! datagram, or are far away and not due this tick, keep their baseline
//! state, so the snapshot record always equals the client's view.
//!
//! Interest management: entities within the view radius only; within
//! [`NEAR_BLOCKS`] every tick (20 Hz), farther every [`FAR_EVERY`] ticks.

use std::collections::{HashMap, VecDeque};

use protocol::{
    BlockXyz, Correction, EntityDatagram, EntityUpdate, Look, MAX_DATAGRAM, MAX_ENTITY_UPDATES,
    Message, NetId, RelPos, Step8,
};

/// Entities nearer than this (blocks) update every tick.
pub const NEAR_BLOCKS: f64 = 48.0;
/// Farther entities update every this many ticks (5 Hz at 20 TPS).
pub const FAR_EVERY: u32 = 4;
/// Snapshots kept per client (3.2 s): an acknowledgement older than this
/// means a full update.
const HISTORY: usize = 64;

/// An entity as the client sees it: quantized exactly as sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntityState {
    /// Entity type (0 = player).
    pub kind: u16,
    /// Position in 1/32 block, absolute.
    pub pos: [i32; 3],
    /// Orientation.
    pub look: Look,
    /// [`EntityUpdate`] flag bits.
    pub flags: u8,
}

/// A world position in absolute 1/32-block units.
#[must_use]
pub fn quantize(pos: [f64; 3]) -> [i32; 3] {
    pos.map(|v| {
        (v * RelPos::SCALE)
            .round()
            .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
    })
}

/// The position an absolute 1/32-block value stands for.
#[must_use]
pub fn dequantize(q: [i32; 3]) -> [f64; 3] {
    q.map(|v| f64::from(v) / RelPos::SCALE)
}

fn rel(q: [i32; 3], origin: BlockXyz) -> Option<RelPos> {
    let o = [origin.x, origin.y, origin.z].map(|v| i64::from(v) * 32);
    let d = |i: usize| i16::try_from(i64::from(q[i]) - o[i]).ok();
    Some(RelPos {
        x: d(0)?,
        y: d(1)?,
        z: d(2)?,
    })
}

/// The absolute 1/32-block position of a [`RelPos`].
#[must_use]
pub fn absolute(r: RelPos, origin: BlockXyz) -> [i32; 3] {
    [
        origin.x.saturating_mul(32).saturating_add(i32::from(r.x)),
        origin.y.saturating_mul(32).saturating_add(i32::from(r.y)),
        origin.z.saturating_mul(32).saturating_add(i32::from(r.z)),
    ]
}

/// An entity this tick, as a candidate for one client.
#[derive(Clone, Copy, Debug)]
pub struct Candidate {
    /// Which entity.
    pub id: NetId,
    /// Its state now.
    pub state: EntityState,
    /// Distance to the receiving player, blocks.
    pub distance: f64,
}

/// One client's replication state.
#[derive(Default)]
pub struct Replicator {
    history: VecDeque<(u32, HashMap<NetId, EntityState>)>,
    acked: u32,
}

impl Replicator {
    /// The client reports the newest snapshot it reconstructed.
    pub fn ack(&mut self, snapshot: u32) {
        if snapshot > self.acked && self.history.iter().any(|(s, _)| *s == snapshot) {
            self.acked = snapshot;
            while self.history.front().is_some_and(|(s, _)| *s < snapshot) {
                self.history.pop_front();
            }
        }
    }

    /// Builds this tick's datagram. `candidates` are the entities of interest
    /// (within the view radius), in any order.
    pub fn build(
        &mut self,
        snapshot: u32,
        origin: BlockXyz,
        ack_tick: Option<u32>,
        correction: Option<Correction>,
        mut candidates: Vec<Candidate>,
    ) -> EntityDatagram {
        let (baseline, base) = match self.history.iter().find(|(s, _)| *s == self.acked) {
            Some((s, m)) => (*s, m.clone()),
            None => (0, HashMap::new()),
        };
        candidates.sort_by(|a, b| a.distance.total_cmp(&b.distance));
        let present: std::collections::HashSet<NetId> = candidates.iter().map(|c| c.id).collect();

        let mut updates = Vec::new();
        for c in &candidates {
            let due = c.distance <= NEAR_BLOCKS
                || (snapshot.wrapping_add(c.id.0 as u32)) % FAR_EVERY == 0;
            let u = match base.get(&c.id) {
                None => rel(c.state.pos, origin).map(|pos| EntityUpdate {
                    id: c.id,
                    kind: Some(c.state.kind),
                    pos: Some(pos),
                    moved: None,
                    look: Some(c.state.look),
                    flags: Some(c.state.flags),
                }),
                Some(_) if !due => None,
                Some(b) => {
                    let mut u = EntityUpdate {
                        id: c.id,
                        ..EntityUpdate::default()
                    };
                    if b.pos != c.state.pos {
                        match Step8::between(b.pos, c.state.pos) {
                            Some(s) => u.moved = Some(s),
                            None => u.pos = rel(c.state.pos, origin),
                        }
                    }
                    if b.look != c.state.look {
                        u.look = Some(c.state.look);
                    }
                    if b.flags != c.state.flags {
                        u.flags = Some(c.state.flags);
                    }
                    (u.moved.is_some() || u.pos.is_some() || u.look.is_some() || u.flags.is_some())
                        .then_some(u)
                }
            };
            if let Some(u) = u {
                updates.push(u);
            }
        }
        let mut removed: Vec<NetId> = base
            .keys()
            .filter(|id| !present.contains(id))
            .copied()
            .collect();
        removed.sort();
        updates.truncate(MAX_ENTITY_UPDATES);
        removed.truncate(MAX_ENTITY_UPDATES - updates.len());

        let mut d = EntityDatagram {
            snapshot,
            baseline,
            ack_tick,
            correction,
            origin,
            updates,
            removed,
        };
        // Keep the nearest updates that fit one datagram; the rest follow.
        // Binary search on the number of updates (each try is an encode),
        // then trim removals if even no update fits.
        if d.encode().is_err() {
            let all = std::mem::take(&mut d.updates);
            let (mut lo, mut hi) = (0, all.len());
            while lo < hi {
                let mid = (lo + hi).div_ceil(2);
                d.updates.clear();
                d.updates.extend_from_slice(&all[..mid]);
                if d.encode().is_ok() {
                    lo = mid;
                } else {
                    hi = mid - 1;
                }
            }
            d.updates.clear();
            d.updates.extend_from_slice(&all[..lo]);
            while d.encode().is_err() && d.removed.pop().is_some() {}
        }
        debug_assert!(d.encode().map_or(true, |b| b.len() <= MAX_DATAGRAM));

        // The record of what the client will know after this datagram.
        let mut record = base;
        let states: HashMap<NetId, EntityState> =
            candidates.iter().map(|c| (c.id, c.state)).collect();
        for u in &d.updates {
            if let Some(s) = states.get(&u.id) {
                record.insert(u.id, *s);
            }
        }
        for id in &d.removed {
            record.remove(id);
        }
        self.history.push_back((snapshot, record));
        while self.history.len() > HISTORY {
            self.history.pop_front();
        }
        d
    }
}

/// The client side: rebuilds full snapshots from deltas.
#[derive(Default)]
pub struct Reconstructor {
    history: VecDeque<(u32, HashMap<NetId, EntityState>)>,
}

impl Reconstructor {
    /// Applies a datagram. Returns the new full state and the IDs that
    /// changed, or `None` if it is older than the newest snapshot or its
    /// baseline is unknown (the next datagram fixes that).
    pub fn apply(
        &mut self,
        d: &EntityDatagram,
    ) -> Option<(&HashMap<NetId, EntityState>, Vec<NetId>)> {
        if self.history.back().is_some_and(|(s, _)| *s >= d.snapshot) {
            return None;
        }
        let mut state = if d.baseline == 0 {
            HashMap::new()
        } else {
            self.history
                .iter()
                .find(|(s, _)| *s == d.baseline)?
                .1
                .clone()
        };
        let mut changed = Vec::with_capacity(d.updates.len() + d.removed.len());
        for u in &d.updates {
            let prev = state.get(&u.id).copied();
            let Some(mut s) = prev.or_else(|| {
                u.kind.map(|kind| EntityState {
                    kind,
                    pos: [0; 3],
                    look: Look::default(),
                    flags: 0,
                })
            }) else {
                continue; // an update for an entity we never saw: ignore
            };
            if let Some(k) = u.kind {
                s.kind = k;
            }
            if let Some(p) = u.pos {
                s.pos = absolute(p, d.origin);
            } else if let Some(m) = u.moved {
                s.pos = m.apply(s.pos);
            }
            if let Some(l) = u.look {
                s.look = l;
            }
            if let Some(f) = u.flags {
                s.flags = f;
            }
            state.insert(u.id, s);
            changed.push(u.id);
        }
        for id in &d.removed {
            if state.remove(id).is_some() {
                changed.push(*id);
            }
        }
        self.history.push_back((d.snapshot, state));
        while self.history.len() > HISTORY {
            self.history.pop_front();
        }
        self.history.back().map(|(_, m)| (m, changed))
    }

    /// The newest reconstructed snapshot (to acknowledge).
    #[must_use]
    pub fn latest(&self) -> u32 {
        self.history.back().map_or(0, |(s, _)| *s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::Angle8;

    fn state(x: f64, yaw: u8) -> EntityState {
        EntityState {
            kind: 0,
            pos: quantize([x, 64.0, 5.0]),
            look: Look {
                yaw: Angle8(yaw),
                ..Look::default()
            },
            flags: 1,
        }
    }

    fn cand(id: u64, x: f64, yaw: u8) -> Candidate {
        Candidate {
            id: NetId(id),
            state: state(x, yaw),
            distance: x.abs(),
        }
    }

    /// Server and client stay in sync over acks, losses and removals.
    #[test]
    fn deltas_rebuild_the_server_view() {
        let origin = BlockXyz { x: 0, y: 64, z: 0 };
        let mut server = Replicator::default();
        let mut client = Reconstructor::default();
        let mut lost = 0;
        for t in 1..200u32 {
            // Entity 1 walks near, 2 walks far (5 Hz), 3 appears at 50 and leaves at 120.
            let mut c = vec![
                cand(1, 10.0 + f64::from(t % 100) * 0.2, (t % 256) as u8),
                cand(2, 100.0 + f64::from(t) * 0.25, 3),
            ];
            if (50..120).contains(&t) {
                c.push(cand(3, -20.0, 0));
            }
            let d = server.build(t, origin, Some(t), None, c.clone());
            assert!(d.encode().expect("fits").len() <= MAX_DATAGRAM);
            if t % 7 == 3 {
                lost += 1;
                continue; // this datagram is lost
            }
            if let Some((view, _)) = client.apply(&d) {
                // Near entities are exact every tick; far ones within 4 ticks.
                assert_eq!(view.get(&NetId(1)), Some(&c[0].state), "tick {t}");
                let far = view.get(&NetId(2)).expect("far entity known");
                assert!(
                    (far.pos[0] - c[1].state.pos[0]).abs() <= 3 * 32,
                    "far lag at {t}"
                );
                assert_eq!(view.contains_key(&NetId(3)), (50..120).contains(&t));
            }
            server.ack(client.latest());
        }
        assert!(lost > 20);
    }

    #[test]
    fn steady_state_is_small_and_stops_when_nothing_moves() {
        let origin = BlockXyz { x: 0, y: 64, z: 0 };
        let mut server = Replicator::default();
        let mut client = Reconstructor::default();
        let mut sizes = Vec::new();
        for t in 1..40u32 {
            let c: Vec<_> = (0..20)
                .map(|i| {
                    cand(
                        i,
                        5.0 + i as f64 + f64::from(t) * 0.25,
                        (t + i as u32) as u8,
                    )
                })
                .collect();
            let d = server.build(t, origin, Some(t), None, c);
            sizes.push(d.encode().expect("fits").len());
            client.apply(&d);
            server.ack(client.latest());
        }
        let steady = sizes[sizes.len() - 1];
        assert!(steady < sizes[0], "deltas {steady} B < full {} B", sizes[0]);
        // Nothing moves: no updates at all.
        let still: Vec<_> = (0..20)
            .map(|i| cand(i, 5.0 + i as f64 + 39.0 * 0.25, (39 + i as u32) as u8))
            .collect();
        let d = server.build(40, origin, Some(40), None, still.clone());
        client.apply(&d);
        server.ack(client.latest());
        let d = server.build(41, origin, Some(41), None, still);
        assert!(d.updates.is_empty() && d.removed.is_empty());
    }

    #[test]
    fn too_many_entities_send_the_nearest_first() {
        let origin = BlockXyz { x: 0, y: 64, z: 0 };
        let mut server = Replicator::default();
        let mut client = Reconstructor::default();
        let c: Vec<_> = (0..300)
            .map(|i| cand(i, f64::from(i as u32) * 0.5, 0))
            .collect();
        let mut known = 0;
        for t in 1..10 {
            let d = server.build(t, origin, None, None, c.clone());
            assert!(d.encode().expect("fits").len() <= MAX_DATAGRAM);
            if let Some((v, _)) = client.apply(&d) {
                known = v.len();
                assert!(v.contains_key(&NetId(0)), "the nearest comes first");
            }
            server.ack(client.latest());
        }
        assert_eq!(known, 300, "the rest followed in later snapshots");
    }
}
