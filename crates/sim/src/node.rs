//! The region node: tick phases, parallel `Simulate`, sorted `Exchange`,
//! and structural merge/split in `PostTick` (ADR-0008, engine-core spec).
//!
//! Phase order per tick: `PreTick` (intents, sequential) → `Simulate`
//! (regions in parallel on a dedicated pool) → `Exchange` (messages sorted
//! by `(tick, sender, sender_pos, counter)` and routed by target position)
//! → `PostTick` (structural operations, called between ticks). A message
//! sent in `Simulate` of tick N is processed in `Simulate` of tick N+1.
//!
//! Merge and split are only exposed between ticks, which is the `PostTick`
//! window: a region that is currently ticking is never restructured, so the
//! "defer until the end of its tick" rule holds by construction.

use std::collections::HashMap;

use engine_core::{DEFAULT_TPS, Tick};
use glam::Vec3;
use thiserror::Error;
use world::coords::BlockPos;

use crate::hash::StateHasher;
use crate::ids::{NetId, RegionId};
use crate::message::Message;
use crate::region::{Body, Region};
use crate::territory::{Axis, SectionRect};

/// Seconds per tick at the default rate.
const DT: f32 = 1.0 / DEFAULT_TPS as f32;

/// Errors of node structure operations.
#[derive(Debug, Error)]
pub enum SimError {
    /// The region id is not present in the node.
    #[error("unknown region {0:?}")]
    UnknownRegion(RegionId),
    /// The `NetId` is already tracked by the node.
    #[error("duplicate net id {0:?}")]
    DuplicateNetId(NetId),
    /// The spawn position is outside the region's territory.
    #[error("position is outside the territory of region {0:?}")]
    OutOfRegion(RegionId),
    /// The territory is outside the arena or overlaps another region.
    #[error("territory does not fit the arena partitioning")]
    BadTerritory,
    /// The two regions do not share a full edge.
    #[error("regions are not adjacent")]
    NotAdjacent,
    /// A split half would be narrower than the buffer allows.
    #[error("split would leave a region narrower than the buffer allows")]
    TooNarrow,
    /// The region thread pool could not be built.
    #[error("region pool: {0}")]
    Pool(String),
}

/// Result of a split attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitOutcome {
    /// Split done; the id of the new (second) region. The first keeps the
    /// original id.
    Split(RegionId),
    /// An entity sits in the two section columns next to the split line;
    /// the split is deferred to a later `PostTick` (buffer invariant).
    Deferred,
}

/// A node owning the regions of one flat prototype arena.
pub struct RegionNode {
    arena: SectionRect,
    regions: Vec<Region>,
    owner: HashMap<NetId, RegionId>,
    tick: Tick,
    pool: rayon::ThreadPool,
    exchange: Vec<Message>,
    next_region_id: u64,
}

impl RegionNode {
    /// An empty node over `arena` with a region pool of `threads` threads.
    ///
    /// # Errors
    /// [`SimError::Pool`] if the thread pool cannot be built.
    pub fn new(arena: SectionRect, threads: usize) -> Result<Self, SimError> {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads.max(1))
            .build()
            .map_err(|e| SimError::Pool(e.to_string()))?;
        Ok(Self {
            arena,
            regions: Vec::new(),
            owner: HashMap::new(),
            tick: Tick(0),
            pool,
            exchange: Vec::new(),
            next_region_id: 1,
        })
    }

    /// Adds a region. The territory must lie inside the arena and must not
    /// overlap an existing region (the partitioning stays a tiling).
    ///
    /// # Errors
    /// [`SimError::BadTerritory`] on overlap or escape from the arena.
    pub fn add_region(&mut self, territory: SectionRect) -> Result<RegionId, SimError> {
        let fits = self.arena.contains(territory.min_x, territory.min_z)
            && self.arena.contains(territory.max_x, territory.max_z);
        let overlaps = self.regions.iter().any(|r| {
            let t = r.territory;
            territory.min_x <= t.max_x
                && t.min_x <= territory.max_x
                && territory.min_z <= t.max_z
                && t.min_z <= territory.max_z
        });
        if !fits || overlaps {
            return Err(SimError::BadTerritory);
        }
        let id = RegionId(self.next_region_id);
        self.next_region_id += 1;
        self.regions.push(Region::new(id, territory));
        Ok(id)
    }

    /// Spawns an entity in a region; the position must be inside its
    /// territory.
    ///
    /// # Errors
    /// [`SimError::UnknownRegion`], [`SimError::DuplicateNetId`],
    /// [`SimError::OutOfRegion`].
    pub fn spawn(&mut self, region: RegionId, net_id: NetId, body: Body) -> Result<(), SimError> {
        if self.owner.contains_key(&net_id) {
            return Err(SimError::DuplicateNetId(net_id));
        }
        let r = self
            .regions
            .iter_mut()
            .find(|r| r.id == region)
            .ok_or(SimError::UnknownRegion(region))?;
        let section = BlockPos::new(body.position.x as i32, 0, body.position.z as i32).section();
        if !r.territory.contains_pos(section) {
            return Err(SimError::OutOfRegion(region));
        }
        r.spawn(net_id, body);
        self.owner.insert(net_id, region);
        Ok(())
    }

    /// The region whose territory contains the section, if any.
    #[must_use]
    pub fn region_at(&self, x: i32, z: i32) -> Option<RegionId> {
        self.regions
            .iter()
            .find(|r| r.territory.contains(x, z))
            .map(|r| r.id)
    }

    /// One full tick: `PreTick` → `Simulate` → `Exchange` → `PostTick`.
    /// Intents for entities that are in flight or unknown are dropped (the
    /// entity is between regions; the drop is deterministic).
    pub fn tick(&mut self, intents: &[(NetId, Vec3)]) {
        self.pre_tick(intents);
        self.simulate();
        self.exchange();
        // PostTick: structural operations are the public try_merge/try_split
        // calls made between ticks; nothing automatic in the prototype.
        self.tick = self.tick.next();
    }

    fn pre_tick(&mut self, intents: &[(NetId, Vec3)]) {
        for &(net_id, velocity) in intents {
            let Some(&region) = self.owner.get(&net_id) else {
                continue;
            };
            if let Some(r) = self.regions.iter_mut().find(|r| r.id == region) {
                r.push_intent(net_id, velocity);
            }
        }
    }

    fn simulate(&mut self) {
        let arena = &self.arena;
        let tick = self.tick;
        let regions = &mut self.regions;
        self.pool.scope(|s| {
            for r in regions.iter_mut() {
                s.spawn(move |_| r.simulate(tick, arena, DT));
            }
        });
    }

    fn exchange(&mut self) {
        self.exchange.clear();
        for r in &mut self.regions {
            self.exchange.append(&mut r.outbox);
        }
        self.exchange.sort_unstable_by_key(Message::sort_key);
        for i in 0..self.exchange.len() {
            let message = self.exchange[i];
            let target = self
                .regions
                .iter_mut()
                .find(|r| r.territory.contains_pos(message.target_pos))
                .expect("the partitioning tiles the arena");
            if let Some(net_id) = message.transferred() {
                self.owner.insert(net_id, target.id);
            }
            target.deliver(message);
        }
        self.exchange.clear();
    }

    /// Merges two adjacent regions into the first (its id survives; the
    /// second id is retired). Entities move by stable `NetId`, sections and
    /// pending messages move with them. Call in `PostTick` (between ticks).
    ///
    /// # Errors
    /// [`SimError::UnknownRegion`], [`SimError::NotAdjacent`].
    pub fn try_merge(&mut self, a: RegionId, b: RegionId) -> Result<(), SimError> {
        let (ia, ib) = self.region_pair(a, b)?;
        let (ta, tb) = (self.regions[ia].territory, self.regions[ib].territory);
        if ta.adjacent_axis(&tb).is_none() {
            return Err(SimError::NotAdjacent);
        }
        let union = ta.union(&tb);
        // Move everything out of b first; indices stay valid (no removal yet).
        let ids = self.regions[ib].net_ids();
        for net_id in ids {
            if let Some(body) = self.regions[ib].take(net_id) {
                self.regions[ia].spawn(net_id, body);
                self.owner.insert(net_id, a);
            }
        }
        let (ra, rb) = if ia < ib {
            let (lo, hi) = self.regions.split_at_mut(ib);
            (&mut lo[ia], &mut hi[0])
        } else {
            let (lo, hi) = self.regions.split_at_mut(ia);
            (&mut hi[0], &mut lo[ib])
        };
        ra.territory = union;
        ra.sections.extend(rb.sections.drain());
        ra.inbox.append(&mut rb.inbox);
        ra.outbox.append(&mut rb.outbox);
        self.regions.swap_remove(ib);
        Ok(())
    }

    /// Splits a region at `line` along `axis` (coordinate < line stays in
    /// the first half, which keeps the id). Deferred while an entity sits
    /// in the two section columns next to the line, so no entity ends up in
    /// the new buffer ring. Call in `PostTick` (between ticks).
    ///
    /// # Errors
    /// [`SimError::UnknownRegion`], [`SimError::TooNarrow`].
    pub fn try_split(
        &mut self,
        region: RegionId,
        axis: Axis,
        line: i32,
    ) -> Result<SplitOutcome, SimError> {
        let index = self
            .regions
            .iter()
            .position(|r| r.id == region)
            .ok_or(SimError::UnknownRegion(region))?;
        let territory = self.regions[index].territory;
        let Some((first, second)) = territory.split(axis, line) else {
            return Err(SimError::TooNarrow);
        };
        let coord = |p: Vec3| -> i32 {
            let section = BlockPos::new(p.x as i32, 0, p.z as i32).section();
            match axis {
                Axis::X => section.x,
                Axis::Z => section.z,
            }
        };
        for net_id in self.regions[index].net_ids() {
            if let Some(body) = self.regions[index].body(net_id) {
                let c = coord(body.position);
                if c == line - 1 || c == line {
                    return Ok(SplitOutcome::Deferred);
                }
            }
        }
        let new_id = RegionId(self.next_region_id);
        self.next_region_id += 1;
        let mut new_region = Region::new(new_id, second);
        new_region.sections.clear();
        for pos in second.sections() {
            if let Some(section) = self.regions[index].sections.remove(&pos) {
                new_region.sections.insert(pos, section);
            }
        }
        for net_id in self.regions[index].net_ids() {
            let Some(body) = self.regions[index].body(net_id) else {
                continue;
            };
            let section =
                BlockPos::new(body.position.x as i32, 0, body.position.z as i32).section();
            if second.contains_pos(section) {
                if let Some(body) = self.regions[index].take(net_id) {
                    new_region.spawn(net_id, body);
                    self.owner.insert(net_id, new_id);
                }
            }
        }
        let pending: Vec<Message> = self.regions[index].inbox.drain(..).collect();
        for message in pending {
            if second.contains_pos(message.target_pos) {
                new_region.deliver(message);
            } else {
                self.regions[index].deliver(message);
            }
        }
        self.regions[index].territory = first;
        self.regions.push(new_region);
        Ok(SplitOutcome::Split(new_id))
    }

    fn region_pair(&self, a: RegionId, b: RegionId) -> Result<(usize, usize), SimError> {
        let ia = self
            .regions
            .iter()
            .position(|r| r.id == a)
            .ok_or(SimError::UnknownRegion(a))?;
        let ib = self
            .regions
            .iter()
            .position(|r| r.id == b)
            .ok_or(SimError::UnknownRegion(b))?;
        Ok((ia, ib))
    }

    /// Current tick number (the next tick to be simulated).
    #[must_use]
    pub fn current_tick(&self) -> Tick {
        self.tick
    }

    /// The arena rectangle.
    #[must_use]
    pub fn arena(&self) -> SectionRect {
        self.arena
    }

    /// The regions, in internal order (not significant for state).
    #[must_use]
    pub fn regions(&self) -> &[Region] {
        &self.regions
    }

    /// A region by id.
    #[must_use]
    pub fn region(&self, id: RegionId) -> Option<&Region> {
        self.regions.iter().find(|r| r.id == id)
    }

    /// The region responsible for the entity: the sender until `Exchange`
    /// delivers a pending transfer, then the target.
    #[must_use]
    pub fn owner_of(&self, net_id: NetId) -> Option<RegionId> {
        self.owner.get(&net_id).copied()
    }

    /// Resident entities across all regions.
    #[must_use]
    pub fn entity_count(&self) -> usize {
        self.regions.iter().map(Region::entity_count).sum()
    }

    /// Transfers sent but not yet processed (in inboxes and outboxes).
    /// `entity_count() + in_flight()` is the conserved total.
    #[must_use]
    pub fn in_flight(&self) -> usize {
        self.regions
            .iter()
            .flat_map(|r| r.inbox().iter().chain(r.outbox()))
            .filter(|m| m.transferred().is_some())
            .count()
    }

    /// The `NetId`s currently in flight (test helper; allocates).
    #[must_use]
    pub fn in_flight_ids(&self) -> Vec<NetId> {
        self.regions
            .iter()
            .flat_map(|r| r.inbox().iter().chain(r.outbox()))
            .filter_map(Message::transferred)
            .collect()
    }

    /// Deterministic fingerprint of the whole node state: tick, partitioning,
    /// sections, entities in `NetId` order and in-flight messages in sort
    /// order. Same input and partitioning → same hash (test helper;
    /// allocates, not for the hot loop).
    #[must_use]
    pub fn state_hash(&self) -> u64 {
        let mut h = StateHasher::new();
        h.u64(self.tick.0);
        h.i32(self.arena.min_x);
        h.i32(self.arena.min_z);
        h.i32(self.arena.max_x);
        h.i32(self.arena.max_z);
        let mut order: Vec<usize> = (0..self.regions.len()).collect();
        order.sort_unstable_by_key(|&i| self.regions[i].id);
        for i in order {
            let r = &self.regions[i];
            h.u64(r.id.0);
            let t = r.territory;
            h.i32(t.min_x);
            h.i32(t.min_z);
            h.i32(t.max_x);
            h.i32(t.max_z);
            let mut sections: Vec<_> = r.sections.iter().collect();
            sections.sort_unstable_by_key(|(p, _)| *p);
            for (p, s) in sections {
                h.i32(p.x);
                h.i32(p.z);
                h.u64(u64::from(s.non_air()));
            }
            for net_id in r.net_ids() {
                let body = r.body(net_id).expect("indexed entity is resident");
                h.u64(net_id.0);
                h.f32(body.position.x);
                h.f32(body.position.y);
                h.f32(body.position.z);
                h.f32(body.velocity.x);
                h.f32(body.velocity.y);
                h.f32(body.velocity.z);
            }
        }
        let mut messages: Vec<Message> = self
            .regions
            .iter()
            .flat_map(|r| r.inbox().iter().chain(r.outbox()))
            .copied()
            .collect();
        messages.sort_unstable_by_key(Message::sort_key);
        for m in messages {
            h.u64(m.sent_at.0);
            h.u64(m.sender.0);
            h.u64(m.counter);
            h.i32(m.target_pos.x);
            h.i32(m.target_pos.z);
            if let Some(net_id) = m.transferred() {
                h.u64(net_id.0);
            }
        }
        h.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::Payload;
    use crate::region::GROUND_Y;
    use world::coords::SectionPos;

    fn arena() -> SectionRect {
        SectionRect::new(0, 0, 15, 7).expect("valid")
    }

    fn two_regions() -> (RegionNode, RegionId, RegionId) {
        let mut node = RegionNode::new(arena(), 2).expect("pool");
        let a = node
            .add_region(SectionRect::new(0, 0, 7, 7).expect("valid"))
            .expect("fits");
        let b = node
            .add_region(SectionRect::new(8, 0, 15, 7).expect("valid"))
            .expect("fits");
        (node, a, b)
    }

    fn walker(x: f32, vx: f32) -> Body {
        Body {
            position: Vec3::new(x, GROUND_Y, 40.0),
            velocity: Vec3::new(vx, 0.0, 0.0),
        }
    }

    #[test]
    fn transfer_is_processed_in_the_next_tick() {
        let (mut node, a, b) = two_regions();
        node.spawn(a, NetId(1), walker(127.9, 4.0)).expect("spawn");
        node.tick(&[]);
        // Sent in Simulate of tick 0: in flight, resident nowhere.
        assert_eq!(node.current_tick(), Tick(1));
        assert_eq!(node.entity_count(), 0);
        assert_eq!(node.in_flight(), 1);
        // Exchange already delivered to b's inbox: b owns the pending entity.
        assert_eq!(node.owner_of(NetId(1)), Some(b));
        node.tick(&[]);
        // Processed in Simulate of tick 1.
        assert_eq!(node.entity_count(), 1);
        assert_eq!(node.in_flight(), 0);
        assert_eq!(node.owner_of(NetId(1)), Some(b));
        assert!(node.region(b).expect("region").contains(NetId(1)));
    }

    #[test]
    fn reply_comes_no_earlier_than_n_plus_2() {
        let (mut node, a, b) = two_regions();
        node.spawn(a, NetId(1), walker(127.9, 4.0)).expect("spawn");
        node.tick(&[]); // tick 0: sent a -> b, in flight
        node.tick(&[]); // tick 1: arrives in b (x = 128.3)
        assert!(node.region(b).expect("region").contains(NetId(1)));
        // An intent for an in-flight entity would be dropped; turn it once
        // it is resident. Reply sent in tick 3, delivered in tick 4 ≥ 0 + 2.
        node.tick(&[(NetId(1), Vec3::new(-4.0, 0.0, 0.0))]); // tick 2: x = 128.1
        assert!(node.region(b).expect("region").contains(NetId(1)));
        node.tick(&[]); // tick 3: x = 127.9, sent b -> a
        assert_eq!(node.entity_count(), 0);
        assert_eq!(node.in_flight(), 1);
        assert!(!node.region(a).expect("region").contains(NetId(1)));
        node.tick(&[]); // tick 4: reply delivered
        assert!(node.region(a).expect("region").contains(NetId(1)));
        assert_eq!(node.owner_of(NetId(1)), Some(a));
    }

    #[test]
    fn exchange_sorts_by_key_not_arrival() {
        let (mut node, a, b) = two_regions();
        let target = SectionPos { x: 9, y: 0, z: 2 };
        let mut keys = [(2u64, 0u64), (1, 9), (1, 2), (0, 5)];
        // Push out of order into a's outbox; all route to b.
        for (tick, counter) in keys {
            node.regions[0].outbox.push(Message {
                sent_at: Tick(tick),
                sender: a,
                sender_pos: (0, 0),
                counter,
                target_pos: target,
                payload: Payload::Transfer {
                    net_id: NetId(99),
                    body: walker(150.0, 0.0),
                },
            });
        }
        keys.sort();
        node.exchange();
        let inbox = node.region(b).expect("region").inbox();
        let got: Vec<(u64, u64)> = inbox.iter().map(|m| (m.sent_at.0, m.counter)).collect();
        assert_eq!(got, keys.to_vec());
    }

    #[test]
    fn merge_moves_entities_sections_and_messages() {
        let (mut node, a, b) = two_regions();
        node.spawn(a, NetId(1), walker(40.0, 0.0)).expect("spawn");
        node.spawn(b, NetId(2), walker(200.0, 0.0)).expect("spawn");
        let sections_before = node.region(a).expect("r").sections().len()
            + node.region(b).expect("r").sections().len();
        node.try_merge(a, b).expect("adjacent");
        assert_eq!(node.regions().len(), 1);
        let merged = node.region(a).expect("survivor");
        assert_eq!(merged.entity_count(), 2);
        assert_eq!(merged.sections().len(), sections_before);
        assert_eq!(merged.territory(), arena());
        assert_eq!(node.owner_of(NetId(2)), Some(a));
        assert!(node.region(b).is_none());
        assert_eq!(node.entity_count() + node.in_flight(), 2);
    }

    #[test]
    fn split_defers_while_an_entity_is_next_to_the_line() {
        let (mut node, a, _b) = two_regions();
        node.try_merge(a, _b).expect("merge");
        // Section x = 3 is line - 1 for a split at 4.
        node.spawn(a, NetId(1), walker(60.0, 0.0)).expect("spawn");
        assert_eq!(
            node.try_split(a, Axis::X, 4).expect("valid line"),
            SplitOutcome::Deferred
        );
        // Move the body away from the line (section x = 2).
        let e = node.regions[0].index[&NetId(1)];
        node.regions[0]
            .world
            .get_mut::<Body>(e)
            .expect("body")
            .position
            .x = 40.0;
        let outcome = node.try_split(a, Axis::X, 4).expect("valid line");
        let SplitOutcome::Split(new_id) = outcome else {
            panic!("split should succeed");
        };
        assert_eq!(node.regions().len(), 2);
        assert!(node.region(a).expect("first").contains(NetId(1)));
        assert_eq!(node.owner_of(NetId(1)), Some(a));
        assert_eq!(node.region(a).expect("first").sections().len(), 32);
        assert_eq!(node.region(new_id).expect("second").sections().len(), 96);
    }

    #[test]
    fn structure_errors_are_reported() {
        let (mut node, a, b) = two_regions();
        assert!(matches!(
            node.add_region(SectionRect::new(0, 0, 3, 3).expect("valid")),
            Err(SimError::BadTerritory)
        ));
        assert!(matches!(
            node.spawn(a, NetId(1), walker(200.0, 0.0)),
            Err(SimError::OutOfRegion(_))
        ));
        node.spawn(a, NetId(1), walker(40.0, 0.0)).expect("spawn");
        assert!(matches!(
            node.spawn(b, NetId(1), walker(200.0, 0.0)),
            Err(SimError::DuplicateNetId(_))
        ));
        assert!(matches!(
            node.try_merge(a, RegionId(999)),
            Err(SimError::UnknownRegion(_))
        ));
        assert!(matches!(
            node.try_split(a, Axis::X, 1),
            Err(SimError::TooNarrow)
        ));
    }
}
