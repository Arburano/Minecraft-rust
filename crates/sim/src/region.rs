//! A region: one `bevy_ecs::World`, a territory of sections and entities,
//! ticked by one thread at a time (ADR-0008).
//!
//! The prototype simulates a flat world: one layer of solid sections at
//! y = 0, bodies walk on its top surface (`GROUND_Y`). Physics is generic
//! kinematics (integration, ground clamp, arena-edge bounce); crossing a
//! territory border stages a transfer message instead of reading the
//! neighbouring region — region code never touches another region's data.

use std::collections::HashMap;

use bevy_ecs::prelude::{Component, Entity, World};
use engine_core::Tick;
use glam::Vec3;
use world::coords::{BlockPos, SectionPos};
use world::{BlockStateId, SECTION_SIZE, Section};

use crate::ids::{NetId, RegionId};
use crate::message::{Message, Payload};
use crate::territory::SectionRect;

/// Top surface of the flat prototype ground (section layer y = 0).
pub const GROUND_Y: f32 = SECTION_SIZE as f32;

/// Generic solid block state of the prototype ground. The engine does not
/// name specific blocks; gameplay block ids belong to `mods/core`.
const SOLID: BlockStateId = BlockStateId(1);

/// Kinematic body: the only replicated component of the prototype.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Body {
    /// Block-space position.
    pub position: Vec3,
    /// Blocks per second.
    pub velocity: Vec3,
}

/// One simulation region. Owned by the node; `&mut` access is confined to a
/// single thread during `Simulate` (no locks around the world).
pub struct Region {
    pub(crate) id: RegionId,
    pub(crate) territory: SectionRect,
    pub(crate) sections: HashMap<SectionPos, Section>,
    pub(crate) world: World,
    pub(crate) index: HashMap<NetId, Entity>,
    pub(crate) inbox: Vec<Message>,
    pub(crate) outbox: Vec<Message>,
    pub(crate) intents: Vec<(NetId, Vec3)>,
    scratch: Vec<(NetId, Entity)>,
    counter: u64,
}

impl Region {
    /// A region owning every section of `territory`, filled with the flat
    /// prototype ground.
    #[must_use]
    pub fn new(id: RegionId, territory: SectionRect) -> Self {
        let sections = territory
            .sections()
            .map(|p| (p, Section::uniform(SOLID)))
            .collect();
        Self {
            id,
            territory,
            sections,
            world: World::new(),
            index: HashMap::new(),
            inbox: Vec::new(),
            outbox: Vec::new(),
            intents: Vec::new(),
            scratch: Vec::new(),
            counter: 0,
        }
    }

    /// Stable region id.
    #[must_use]
    pub fn id(&self) -> RegionId {
        self.id
    }

    /// The territory rectangle (buffer ring included).
    #[must_use]
    pub fn territory(&self) -> SectionRect {
        self.territory
    }

    /// Owned voxel sections.
    #[must_use]
    pub fn sections(&self) -> &HashMap<SectionPos, Section> {
        &self.sections
    }

    /// Number of entities currently in this region (in-flight transfers are
    /// not counted here).
    #[must_use]
    pub fn entity_count(&self) -> usize {
        self.index.len()
    }

    /// Whether the entity is present in this region.
    #[must_use]
    pub fn contains(&self, net_id: NetId) -> bool {
        self.index.contains_key(&net_id)
    }

    /// The body of an entity, if present.
    #[must_use]
    pub fn body(&self, net_id: NetId) -> Option<Body> {
        let &e = self.index.get(&net_id)?;
        self.world.get::<Body>(e).copied()
    }

    /// All resident `NetId`s in stable order (test/hash helper; allocates).
    #[must_use]
    pub fn net_ids(&self) -> Vec<NetId> {
        let mut ids: Vec<_> = self.index.keys().copied().collect();
        ids.sort_unstable();
        ids
    }

    /// Spawns an entity with a stable identity — the transfer primitive
    /// (merge, split, inbox delivery). The caller guarantees the id is not
    /// already resident; the node checks uniqueness across regions.
    pub fn spawn(&mut self, net_id: NetId, body: Body) {
        let entity = self.world.spawn((net_id, body)).id();
        self.index.insert(net_id, entity);
    }

    /// Removes an entity and returns its body — the sending side of a
    /// transfer by stable `NetId` (merge, split, border crossing).
    pub fn take(&mut self, net_id: NetId) -> Option<Body> {
        let e = self.index.remove(&net_id)?;
        let body = self.world.get::<Body>(e).copied();
        self.world.despawn(e);
        body
    }

    /// Queues an incoming message for the next `Simulate`.
    pub(crate) fn deliver(&mut self, message: Message) {
        self.inbox.push(message);
    }

    /// Queues a velocity intent from `PreTick`.
    pub(crate) fn push_intent(&mut self, net_id: NetId, velocity: Vec3) {
        self.intents.push((net_id, velocity));
    }

    /// Messages staged during the last `Simulate`, awaiting `Exchange`.
    pub(crate) fn outbox(&self) -> &[Message] {
        &self.outbox
    }

    /// Pending incoming messages (in-flight transfers).
    pub(crate) fn inbox(&self) -> &[Message] {
        &self.inbox
    }

    /// One region's `Simulate`: inbox delivery, intents, then physics in
    /// stable `NetId` order (engine-core invariant 3). No allocations: the
    /// sort buffer and the message vectors are reused across ticks.
    pub(crate) fn simulate(&mut self, tick: Tick, arena: &SectionRect, dt: f32) {
        for i in 0..self.inbox.len() {
            let Payload::Transfer { net_id, body } = self.inbox[i].payload;
            self.spawn(net_id, body);
        }
        self.inbox.clear();

        for i in 0..self.intents.len() {
            let (net_id, velocity) = self.intents[i];
            if let Some(&e) = self.index.get(&net_id) {
                if let Some(mut body) = self.world.get_mut::<Body>(e) {
                    body.velocity = velocity;
                }
            }
        }
        self.intents.clear();

        self.scratch.clear();
        for entity in self.world.iter_entities() {
            if let Some(id) = entity.get::<NetId>() {
                self.scratch.push((*id, entity.id()));
            }
        }
        self.scratch.sort_unstable();

        // Playable block range: the arena without its outer buffer ring.
        let size = SECTION_SIZE as f32;
        let lo_x = (arena.min_x + 1) as f32 * size;
        let hi_x = (arena.max_x + 1) as f32 * size - size;
        let lo_z = (arena.min_z + 1) as f32 * size;
        let hi_z = (arena.max_z + 1) as f32 * size - size;

        for i in 0..self.scratch.len() {
            let (net_id, entity) = self.scratch[i];
            let Some(mut body) = self.world.get_mut::<Body>(entity) else {
                continue;
            };
            let velocity = body.velocity;
            body.position += velocity * dt;
            if body.position.y < GROUND_Y {
                body.position.y = GROUND_Y;
                body.velocity.y = 0.0;
            }
            if body.position.x < lo_x {
                body.position.x = lo_x + (lo_x - body.position.x);
                body.velocity.x = body.velocity.x.abs();
            } else if body.position.x > hi_x {
                body.position.x = hi_x - (body.position.x - hi_x);
                body.velocity.x = -body.velocity.x.abs();
            }
            if body.position.z < lo_z {
                body.position.z = lo_z + (lo_z - body.position.z);
                body.velocity.z = body.velocity.z.abs();
            } else if body.position.z > hi_z {
                body.position.z = hi_z - (body.position.z - hi_z);
                body.velocity.z = -body.velocity.z.abs();
            }
            body.position.x = body.position.x.clamp(lo_x, hi_x);
            body.position.z = body.position.z.clamp(lo_z, hi_z);
            let snapshot = *body;
            let section =
                BlockPos::new(snapshot.position.x as i32, 0, snapshot.position.z as i32).section();
            if !self.territory.contains_pos(section) {
                self.outbox.push(Message {
                    sent_at: tick,
                    sender: self.id,
                    sender_pos: (section.x, section.z),
                    counter: self.counter,
                    target_pos: section,
                    payload: Payload::Transfer {
                        net_id,
                        body: snapshot,
                    },
                });
                self.counter += 1;
                self.index.remove(&net_id);
                self.world.despawn(entity);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region() -> Region {
        Region::new(RegionId(1), SectionRect::new(0, 0, 7, 7).expect("valid"))
    }

    fn arena() -> SectionRect {
        SectionRect::new(0, 0, 15, 7).expect("valid")
    }

    #[test]
    fn ground_clamp_and_integration() {
        let mut r = region();
        r.spawn(
            NetId(1),
            Body {
                position: Vec3::new(40.0, 16.2, 40.0),
                velocity: Vec3::new(2.0, -10.0, 0.0),
            },
        );
        r.simulate(Tick(0), &arena(), 0.05);
        let b = r.body(NetId(1)).expect("resident");
        assert_eq!(b.position.x, 40.1);
        assert_eq!(b.position.y, GROUND_Y);
        assert_eq!(b.velocity.y, 0.0);
    }

    #[test]
    fn arena_edge_bounces() {
        let mut r = region();
        // Arena inner x range is [16, 240); push the body past the low edge.
        r.spawn(
            NetId(1),
            Body {
                position: Vec3::new(16.05, GROUND_Y, 40.0),
                velocity: Vec3::new(-2.0, 0.0, 0.0),
            },
        );
        r.simulate(Tick(0), &arena(), 0.05);
        let b = r.body(NetId(1)).expect("resident");
        assert!(b.position.x >= 16.0);
        assert!(b.velocity.x > 0.0);
    }

    #[test]
    fn crossing_the_territory_stages_a_transfer_and_despawns() {
        let mut r = region();
        // Territory x ends at section 7 (block 128); arena continues to 15.
        r.spawn(
            NetId(7),
            Body {
                position: Vec3::new(127.95, GROUND_Y, 40.0),
                velocity: Vec3::new(4.0, 0.0, 0.0),
            },
        );
        r.simulate(Tick(5), &arena(), 0.05);
        assert_eq!(r.entity_count(), 0);
        assert_eq!(r.outbox().len(), 1);
        let m = &r.outbox()[0];
        assert_eq!(m.sent_at, Tick(5));
        assert_eq!(m.sender, RegionId(1));
        assert_eq!(m.target_pos, SectionPos { x: 8, y: 0, z: 2 });
        assert_eq!(m.transferred(), Some(NetId(7)));
    }

    #[test]
    fn inbox_spawns_on_the_next_simulate() {
        let mut r = region();
        let body = Body {
            position: Vec3::new(40.0, GROUND_Y, 40.0),
            velocity: Vec3::ZERO,
        };
        r.deliver(Message {
            sent_at: Tick(0),
            sender: RegionId(9),
            sender_pos: (2, 2),
            counter: 0,
            target_pos: SectionPos { x: 2, y: 0, z: 2 },
            payload: Payload::Transfer {
                net_id: NetId(3),
                body,
            },
        });
        assert_eq!(r.entity_count(), 0);
        r.simulate(Tick(1), &arena(), 0.05);
        assert_eq!(r.body(NetId(3)), Some(body));
        assert!(r.inbox().is_empty());
    }

    #[test]
    fn intents_apply_only_to_resident_entities() {
        let mut r = region();
        r.spawn(
            NetId(1),
            Body {
                position: Vec3::new(40.0, GROUND_Y, 40.0),
                velocity: Vec3::ZERO,
            },
        );
        r.push_intent(NetId(1), Vec3::new(1.0, 0.0, 0.0));
        r.push_intent(NetId(2), Vec3::new(9.0, 0.0, 0.0));
        r.simulate(Tick(0), &arena(), 0.05);
        let b = r.body(NetId(1)).expect("resident");
        assert_eq!(b.velocity, Vec3::new(1.0, 0.0, 0.0));
        assert_eq!(b.position.x, 40.05);
    }
}
