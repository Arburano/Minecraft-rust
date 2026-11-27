//! One region (T2-06, ADR-0008): a separate `bevy_ecs::World` holding its
//! players, plus the live columns of its territory.
//!
//! A region's tick reads and writes only its own players and columns, and
//! reads the node's shared state (registry, physics, the encoded-column
//! cache) immutably, so regions tick in parallel without locks. What crosses
//! regions — block changes seen by players of other regions, cache updates,
//! column requests — goes out in [`RegionOutput`] and is handled by the node
//! in `Exchange`, between region ticks.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::Arc;

use bevy_ecs::prelude::{Component, Entity};
use glam::DVec3;
use protocol::{
    Angle8, BlockChange, BlockXyz, Correction, Intent, Look, MAX_BLOCK_CHANGES, Message, NetId,
    ServerEvent, Spawn,
};
use sim::physics::{MapBlocks, PhysicsConfig, PlayerBody, step};
use tokio::sync::mpsc::UnboundedSender;
use world::coords::BlockPos;
use world::map::DirtyKind;
use world::registry::Collision;
use world::{BlockStateId, ColumnPos, Registry, WorldMap};

use crate::entities::{
    Grid, MAX_PATH_REQUESTS_PER_TICK, MAX_PATHS_IN_FLIGHT, NEAR_BLOCKS, PathRequest, WALKER_KIND,
    Walker, decision_interval,
};
use crate::game::{Edit, MAX_STREAMS, ToConn, UNLOAD_MARGIN};
use crate::input::{apply_fly_steps, correction, flags, physics_input, state_hash, wire_yaw};
use crate::partition::RegionId;
use crate::players::SavedPlayer;
use crate::replication::{Candidate, EntityState, Replicator, quantize};

/// Intents a player may queue; older ones are dropped (a flooding or
/// stalled client cannot grow server memory).
pub(crate) const MAX_QUEUED_INTENTS: usize = 40;
/// Intents applied per tick at most: one is normal, more catch up after
/// jitter, never so many that a client moves faster than it could.
const MAX_INTENTS_PER_TICK: usize = 3;
/// How far beyond the movement profile's reach an edit may be: the server's
/// view of the player lags the client's by up to a round trip.
const REACH_TOLERANCE: f64 = 1.5;

/// A connected player: an entity of its region's ECS world.
#[derive(Component)]
pub struct Player {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) net_id: NetId,
    /// Pending correction: resent every tick until the client's hashes agree.
    pub(crate) correction: Option<Correction>,
    pub(crate) replicator: Replicator,
    pub(crate) out: UnboundedSender<ToConn>,
    pub(crate) body: PlayerBody,
    pub(crate) yaw: f64,
    pub(crate) pitch: f64,
    pub(crate) fly_speed: f64,
    pub(crate) spawned: bool,
    pub(crate) intents: VecDeque<Intent>,
    pub(crate) last_tick: Option<u32>,
    pub(crate) sent: HashSet<ColumnPos>,
    pub(crate) in_flight: usize,
    /// The column the player had every column around when last checked
    /// (no scan needed while it stays there).
    pub(crate) complete_at: Option<ColumnPos>,
}

impl Player {
    /// The column the player stands in.
    #[must_use]
    pub fn column(&self) -> ColumnPos {
        ColumnPos {
            x: (self.body.pos.x / 16.0).floor() as i32,
            z: (self.body.pos.z / 16.0).floor() as i32,
        }
    }

    pub(crate) fn saved(&self) -> SavedPlayer {
        SavedPlayer {
            pos: self.body.pos.to_array(),
            yaw: self.yaw,
            pitch: self.pitch,
            flying: self.body.flying,
        }
    }
}

/// A queued edit.
pub struct EditRequest {
    /// The editing player.
    pub id: String,
    /// Their sequence number.
    pub seq: u32,
    /// The cell.
    pub pos: BlockPos,
    /// What to do.
    pub edit: Edit,
    /// `false` if the connection's rate limit refused it.
    pub allowed: bool,
}

/// What the node shares with every region during `Simulate` (read-only).
pub struct Shared<'a> {
    /// Block registry.
    pub registry: &'a Registry,
    /// Movement profile.
    pub physics: &'a PhysicsConfig,
    /// Swim blocks.
    pub swim: &'a [BlockStateId],
    /// Streamed column radius.
    pub view: i32,
    /// Encoded column messages of every loaded column.
    pub cache: &'a HashMap<ColumnPos, Arc<[u8]>>,
    /// This tick's snapshot number.
    pub snapshot: u32,
    /// World spawn (x, z) for new players.
    pub spawn: [f64; 2],
    /// Column offsets within the view radius, nearest first.
    pub offsets: &'a [(i32, i32)],
    /// Every player and entity at the end of the previous tick (those of
    /// other regions are replicated from this snapshot: one tick late, no
    /// lock).
    pub players: &'a [PlayerSnapshot],
    /// The node's tick counter.
    pub tick: u64,
}

/// A player as replicated, with its region.
#[derive(Clone, Copy, Debug)]
pub struct PlayerSnapshot {
    /// Network identity.
    pub net_id: NetId,
    /// Feet position.
    pub pos: DVec3,
    /// Replicated state.
    pub state: EntityState,
    /// Owning region.
    pub region: RegionId,
}

impl PlayerSnapshot {
    /// The snapshot of a walker.
    #[must_use]
    pub fn of_walker(w: &Walker, region: RegionId) -> Self {
        let yaw = Angle8::from_radians(w.yaw as f32);
        Self {
            net_id: w.net_id,
            pos: w.body.pos,
            state: EntityState {
                kind: WALKER_KIND,
                pos: quantize(w.body.pos.to_array()),
                look: Look {
                    yaw,
                    pitch: Angle8(0),
                    head_yaw: yaw,
                },
                flags: flags(&w.body),
            },
            region,
        }
    }

    /// The snapshot of a spawned player.
    #[must_use]
    pub fn of(p: &Player, region: RegionId) -> Self {
        let a = |r: f64| Angle8::from_radians(r as f32);
        Self {
            net_id: p.net_id,
            pos: p.body.pos,
            state: EntityState {
                kind: 0,
                pos: quantize(p.body.pos.to_array()),
                look: Look {
                    yaw: a(p.yaw),
                    pitch: a(p.pitch),
                    head_yaw: a(p.yaw),
                },
                flags: flags(&p.body),
            },
            region,
        }
    }
}

/// What a region's tick hands back to the node.
#[derive(Default)]
pub struct RegionOutput {
    /// Columns its players should see that the node has not loaded.
    pub needs: Vec<ColumnPos>,
    /// Accepted block changes (players of other regions may have the column).
    pub changes: Vec<(ColumnPos, BlockChange)>,
    /// Corrections issued.
    pub corrections: u64,
    /// New path requests (at most `MAX_PATH_REQUESTS_PER_TICK`).
    pub path_requests: Vec<PathRequest>,
    /// AI decisions taken.
    pub decisions: u64,
    /// Entities simulated (physics) this tick.
    pub simulated: u64,
    /// Time the tick took.
    pub time: std::time::Duration,
}

/// One region.
pub struct Region {
    pub(crate) id: RegionId,
    pub(crate) ecs: bevy_ecs::world::World,
    /// Players by ID (sorted: deterministic order everywhere).
    pub(crate) index: BTreeMap<String, Entity>,
    pub(crate) map: WorldMap,
    pub(crate) edits: Vec<EditRequest>,
    pub(crate) unsaved: HashSet<ColumnPos>,
    pub(crate) out: RegionOutput,
    /// Simulated entities by network ID.
    pub(crate) walkers: BTreeMap<NetId, Entity>,
    dirty: Vec<world::coords::SectionPos>,
}

impl Region {
    /// An empty region.
    #[must_use]
    pub fn new(id: RegionId, registry: Arc<Registry>, bounds: world::WorldBounds) -> Self {
        Self {
            id,
            ecs: bevy_ecs::world::World::new(),
            index: BTreeMap::new(),
            map: WorldMap::new(registry, bounds),
            edits: Vec::new(),
            unsaved: HashSet::new(),
            out: RegionOutput::default(),
            walkers: BTreeMap::new(),
            dirty: Vec::new(),
        }
    }

    /// Adds a simulated entity.
    pub fn insert_walker(&mut self, w: Walker) {
        let id = w.net_id;
        let e = self.ecs.spawn(w).id();
        self.walkers.insert(id, e);
    }

    /// Removes a simulated entity.
    pub fn take_walker(&mut self, id: NetId) -> Option<Walker> {
        let e = self.walkers.remove(&id)?;
        let mut entity = self.ecs.get_entity_mut(e).ok()?;
        let w = entity.take::<Walker>();
        entity.despawn();
        w
    }

    /// A simulated entity, to change.
    pub fn walker_mut(&mut self, id: NetId) -> Option<bevy_ecs::world::Mut<'_, Walker>> {
        let e = *self.walkers.get(&id)?;
        self.ecs.get_mut::<Walker>(e)
    }

    /// Simulated entities (sorted by ID).
    pub fn walkers(&self) -> impl Iterator<Item = &Walker> {
        self.walkers
            .values()
            .filter_map(|&e| self.ecs.get::<Walker>(e))
    }

    /// The region's ID.
    #[must_use]
    pub fn id(&self) -> RegionId {
        self.id
    }

    /// Player IDs in this region, sorted.
    pub fn player_ids(&self) -> impl Iterator<Item = &str> {
        self.index.keys().map(String::as_str)
    }

    /// Live columns.
    #[must_use]
    pub fn live_columns(&self) -> usize {
        self.map.len()
    }

    /// Adds a player entity.
    pub fn insert(&mut self, p: Player) {
        let id = p.id.clone();
        let e = self.ecs.spawn(p).id();
        self.index.insert(id, e);
    }

    /// Removes a player entity and returns its component.
    pub fn take(&mut self, id: &str) -> Option<Player> {
        let e = self.index.remove(id)?;
        let mut entity = self.ecs.get_entity_mut(e).ok()?;
        let p = entity.take::<Player>();
        entity.despawn();
        p
    }

    /// A player, read-only.
    #[must_use]
    pub fn player(&self, id: &str) -> Option<&Player> {
        self.ecs.get::<Player>(*self.index.get(id)?)
    }

    /// A player, to change.
    pub fn player_mut(&mut self, id: &str) -> Option<bevy_ecs::world::Mut<'_, Player>> {
        let e = *self.index.get(id)?;
        self.ecs.get_mut::<Player>(e)
    }

    /// Sends a player its spawn.
    pub(crate) fn send_spawn(p: &mut Player, bounds: world::WorldBounds, view: i32) {
        p.spawned = true;
        let _ = p.out.send(ToConn::Spawn(Spawn {
            tick: 0,
            position: p.body.pos.to_array(),
            yaw: p.yaw as f32,
            pitch: p.pitch as f32,
            min_section_y: bounds.min_section_y,
            section_count: bounds.section_count,
            flying: p.body.flying,
            view_distance: view.clamp(2, 255) as u8,
        }));
    }

    /// One `Simulate` step of this region.
    pub fn tick(&mut self, sh: &Shared<'_>) {
        let start = std::time::Instant::now();
        self.out = RegionOutput::default();
        let ids: Vec<Entity> = self.index.values().copied().collect();
        self.spawn_waiting(&ids, sh);
        self.move_players(&ids, sh);
        self.tick_walkers(&ids, sh);
        let edited = self.apply_edits(sh);
        self.stream(&ids, sh, &edited);
        self.replicate(&ids, sh);
        self.dirty.clear();
        self.map.take_dirty(DirtyKind::Save, &mut self.dirty);
        for s in &self.dirty {
            self.unsaved.insert(ColumnPos::of_section(*s));
        }
        // Meshing and light are the clients' (ADR-0011).
        self.map.take_dirty(DirtyKind::Mesh, &mut self.dirty);
        self.map.take_dirty(DirtyKind::Light, &mut self.dirty);
        self.out.time = start.elapsed();
    }

    /// Entities: physics and path following every tick, decisions at the
    /// LOD interval, path requests within the per-tick budget.
    fn tick_walkers(&mut self, players: &[Entity], sh: &Shared<'_>) {
        if self.walkers.is_empty() {
            return;
        }
        let mut eyes: Vec<DVec3> = players
            .iter()
            .filter_map(|&e| self.ecs.get::<Player>(e))
            .filter(|p| p.spawned)
            .map(|p| p.body.pos)
            .collect();
        eyes.extend(
            sh.players
                .iter()
                .filter(|s| s.state.kind == 0 && s.region != self.id)
                .map(|s| s.pos),
        );
        let mut in_flight = self.walkers().filter(|w| w.waiting).count();
        let world = MapBlocks {
            map: &self.map,
            swim: sh.swim,
        };
        let ids: Vec<Entity> = self.walkers.values().copied().collect();
        for e in ids {
            let Some(mut w) = self.ecs.get_mut::<Walker>(e) else {
                continue;
            };
            let w = &mut *w;
            if !w.placed {
                let c = w.cell();
                match self.map.height(c.x, c.z) {
                    Some(h) => {
                        w.body.pos.y = f64::from(h);
                        w.placed = true;
                    }
                    None => continue, // its column is not live yet
                }
            }
            // Decisions at the LOD interval.
            if sh.tick >= w.next_decision && !w.waiting {
                let nearest = eyes
                    .iter()
                    .map(|p| p.distance(w.body.pos))
                    .fold(f64::INFINITY, f64::min);
                w.next_decision = sh.tick + decision_interval(nearest);
                self.out.decisions += 1;
                let stuck = w.stuck > 40;
                if (w.idle() || stuck)
                    && w.body.on_ground
                    && self.out.path_requests.len() < MAX_PATH_REQUESTS_PER_TICK
                    && in_flight < MAX_PATHS_IN_FLIGHT
                {
                    let start = w.cell();
                    let goal = w.pick_goal();
                    self.out.path_requests.push(PathRequest {
                        net_id: w.net_id,
                        start,
                        goal,
                        grid: Grid::snapshot(&self.map, sh.registry, start),
                        near: nearest <= NEAR_BLOCKS,
                    });
                    w.waiting = true;
                    w.requested_at = sh.tick;
                    w.path.clear();
                    w.next = 0;
                    w.stuck = 0;
                    in_flight += 1;
                }
            }
            // Every tick: follow the path and move.
            let input = w.steer();
            step(&mut w.body, &input, sh.physics, &world);
            self.out.simulated += 1;
        }
    }

    fn spawn_waiting(&mut self, ids: &[Entity], sh: &Shared<'_>) {
        let Some(h) = self.map.height(sh.spawn[0] as i32, sh.spawn[1] as i32) else {
            return;
        };
        let bounds = self.map.bounds();
        for &e in ids {
            if let Some(mut p) = self.ecs.get_mut::<Player>(e)
                && !p.spawned
            {
                p.body.pos.y = f64::from(h);
                Self::send_spawn(&mut p, bounds, sh.view);
            }
        }
    }

    fn move_players(&mut self, ids: &[Entity], sh: &Shared<'_>) {
        let world = MapBlocks {
            map: &self.map,
            swim: sh.swim,
        };
        for &e in ids {
            let Some(mut p) = self.ecs.get_mut::<Player>(e) else {
                continue;
            };
            if !p.spawned {
                continue;
            }
            // Hold the player until the ground around them is loaded (the
            // client does the same before it sends any intent).
            let c = p.column();
            let ready = (-1..=1).all(|dx| {
                (-1..=1).all(|dz| {
                    self.map
                        .column(ColumnPos {
                            x: c.x + dx,
                            z: c.z + dz,
                        })
                        .is_some()
                })
            });
            if !ready {
                continue;
            }
            let p = &mut *p;
            for _ in 0..MAX_INTENTS_PER_TICK {
                let Some(i) = p.intents.pop_front() else {
                    break;
                };
                p.fly_speed =
                    apply_fly_steps(p.fly_speed, i.fly_speed_steps, sh.physics.fly_speed_range);
                step(
                    &mut p.body,
                    &physics_input(&i, p.fly_speed),
                    sh.physics,
                    &world,
                );
                // Reconciliation: a correction only when the prediction differs.
                if state_hash(&p.body, p.fly_speed) == i.state_hash {
                    if p.correction.is_some_and(|c| i.tick > c.tick) {
                        p.correction = None; // the client has applied it
                    }
                } else {
                    if p.correction.is_none() {
                        self.out.corrections += 1;
                    }
                    p.correction = Some(correction(i.tick, &p.body, p.fly_speed));
                }
                p.yaw = wire_yaw(&i);
                p.pitch = f64::from(i.pitch.to_signed_radians());
                p.last_tick = Some(i.tick);
                if p.intents.len() <= 1 {
                    // Normal pace: one per tick; catch up only when behind.
                    break;
                }
            }
        }
    }

    /// Validates and applies this tick's edits in arrival order, then sends
    /// this region's players the changes in columns they have, followed by
    /// the acks of their own edits (the events stream is ordered, so a
    /// client sees the authoritative state before it resolves its
    /// prediction). Returns the edited columns.
    fn apply_edits(&mut self, sh: &Shared<'_>) -> HashSet<ColumnPos> {
        let mut edited = HashSet::new();
        if self.edits.is_empty() {
            return edited;
        }
        let edits = std::mem::take(&mut self.edits);
        let mut acks: HashMap<String, Vec<ServerEvent>> = HashMap::new();
        for r in edits {
            let accepted = r.allowed && self.check_edit(&r.id, r.pos, r.edit, sh);
            if accepted {
                let state = match r.edit {
                    Edit::Dig => BlockStateId::AIR,
                    Edit::Place(s) => BlockStateId(s),
                };
                if self.map.set_block(r.pos, state).is_ok() {
                    let c = ColumnPos::of_block(r.pos);
                    edited.insert(c);
                    self.out.changes.push((
                        c,
                        BlockChange {
                            pos: r.pos.into(),
                            state: state.0,
                        },
                    ));
                }
            }
            acks.entry(r.id).or_default().push(ServerEvent::BlockAck {
                seq: r.seq,
                accepted,
            });
        }
        for (id, &e) in &self.index {
            let Some(p) = self.ecs.get::<Player>(e) else {
                continue;
            };
            let mine: Vec<BlockChange> = self
                .out
                .changes
                .iter()
                .filter(|(c, _)| p.sent.contains(c))
                .map(|(_, b)| *b)
                .collect();
            for chunk in mine.chunks(MAX_BLOCK_CHANGES) {
                let _ = p
                    .out
                    .send(ToConn::Event(ServerEvent::BlockChanges(chunk.to_vec())));
            }
            for a in acks.remove(id).unwrap_or_default() {
                let _ = p.out.send(ToConn::Event(a));
            }
        }
        edited
    }

    /// Whether `id` may make this edit: spawned, within reach, the cell
    /// loaded, digging something, placing a real block into air or a fluid
    /// and not into another player.
    fn check_edit(&self, id: &str, pos: BlockPos, edit: Edit, sh: &Shared<'_>) -> bool {
        let Some(p) = self.player(id).filter(|p| p.spawned) else {
            return false;
        };
        let Some(current) = self.map.get_block(pos) else {
            return false; // not loaded or outside the world height
        };
        let eye = p.body.eye(sh.physics);
        let center = DVec3::new(
            f64::from(pos.x) + 0.5,
            f64::from(pos.y) + 0.5,
            f64::from(pos.z) + 0.5,
        );
        if eye.distance(center) > sh.physics.reach + REACH_TOLERANCE {
            return false;
        }
        match edit {
            Edit::Dig => current != BlockStateId::AIR,
            Edit::Place(s) => {
                let state = BlockStateId(s);
                if state == BlockStateId::AIR
                    || !sh.registry.contains(state)
                    || (current != BlockStateId::AIR && !sh.swim.contains(&current))
                {
                    return false;
                }
                if sh.registry.props(state).collision == Collision::None {
                    return true;
                }
                // The editor checked itself on its side; here no other player
                // may end up inside the block. Players of other regions are
                // farther than the buffer.
                let min = DVec3::new(f64::from(pos.x), f64::from(pos.y), f64::from(pos.z));
                let max = min + DVec3::ONE;
                self.index
                    .iter()
                    .filter(|(other, _)| other.as_str() != id)
                    .filter_map(|(_, &e)| self.ecs.get::<Player>(e))
                    .filter(|q| q.spawned)
                    .all(|q| {
                        let b = q.body.aabb(sh.physics);
                        !(0..3).all(|a| b.max[a] > min[a] && b.min[a] < max[a])
                    })
            }
        }
    }

    fn stream(&mut self, ids: &[Entity], sh: &Shared<'_>, edited: &HashSet<ColumnPos>) {
        let view = sh.view;
        let mut needs = HashSet::new();
        for &e in ids {
            let Some(mut p) = self.ecs.get_mut::<Player>(e) else {
                continue;
            };
            if !p.spawned {
                continue;
            }
            let center = p.column();
            if p.complete_at == Some(center) {
                continue;
            }
            // Forget what left the view (with a margin).
            let far: Vec<ColumnPos> = p
                .sent
                .iter()
                .copied()
                .filter(|&c| chebyshev(c, center) > view + UNLOAD_MARGIN)
                .collect();
            for c in far {
                p.sent.remove(&c);
                let _ = p
                    .out
                    .send(ToConn::Event(ServerEvent::ForgetColumn(c.into())));
            }
            let mut all_sent = true;
            for &(dx, dz) in sh.offsets {
                let c = ColumnPos {
                    x: center.x + dx,
                    z: center.z + dz,
                };
                if p.sent.contains(&c) {
                    continue;
                }
                all_sent = false;
                let Some(bytes) = sh.cache.get(&c) else {
                    needs.insert(c);
                    continue;
                };
                // A column edited this tick is sent next tick, once the node
                // has re-encoded it (its changes went only to holders).
                if edited.contains(&c) || p.in_flight >= MAX_STREAMS {
                    continue;
                }
                if p.out
                    .send(ToConn::Column {
                        pos: c,
                        bytes: Arc::clone(bytes),
                        priority: -chebyshev(c, center),
                    })
                    .is_ok()
                {
                    p.sent.insert(c);
                    p.in_flight += 1;
                }
            }
            if all_sent {
                p.complete_at = Some(center);
            }
        }
        let mut needs: Vec<ColumnPos> = needs.into_iter().collect();
        needs.sort_by_key(|c| (c.x, c.z));
        self.out.needs = needs;
    }

    /// Sends every spawned player this tick's datagram: their reconciliation
    /// data and the deltas of the other players in view — this region's as
    /// they are now, other regions' from the previous tick's snapshot.
    fn replicate(&mut self, ids: &[Entity], sh: &Shared<'_>) {
        let radius = f64::from(sh.view * 16);
        let mut everyone: Vec<(NetId, DVec3, EntityState)> = ids
            .iter()
            .filter_map(|&e| self.ecs.get::<Player>(e))
            .filter(|p| p.spawned)
            .map(|p| {
                let s = PlayerSnapshot::of(p, self.id);
                (s.net_id, s.pos, s.state)
            })
            .collect();
        everyone.extend(self.walkers().map(|w| {
            let s = PlayerSnapshot::of_walker(w, self.id);
            (s.net_id, s.pos, s.state)
        }));
        let own: HashSet<NetId> = everyone.iter().map(|(id, _, _)| *id).collect();
        everyone.extend(
            sh.players
                .iter()
                .filter(|s| s.region != self.id && !own.contains(&s.net_id))
                .map(|s| (s.net_id, s.pos, s.state)),
        );
        for &e in ids {
            let Some(mut p) = self.ecs.get_mut::<Player>(e) else {
                continue;
            };
            if !p.spawned {
                continue;
            }
            let me = p.body.pos;
            let own = p.net_id;
            let candidates = everyone
                .iter()
                .filter(|(id, _, _)| *id != own)
                .filter_map(|&(id, pos, state)| {
                    let h = ((pos.x - me.x).powi(2) + (pos.z - me.z).powi(2)).sqrt();
                    (h <= radius).then_some(Candidate {
                        id,
                        state,
                        distance: (pos - me).length(),
                    })
                })
                .collect();
            let origin = BlockXyz {
                x: me.x.floor() as i32,
                y: me.y.floor() as i32,
                z: me.z.floor() as i32,
            };
            let p = &mut *p;
            let d = p
                .replicator
                .build(sh.snapshot, origin, p.last_tick, p.correction, candidates);
            if let Ok(bytes) = d.encode() {
                let _ = p.out.send(ToConn::Datagram(bytes));
            }
        }
    }
}

/// Columns within `radius` of `center`, nearest first (Euclidean order,
/// ties by position), the order in which they are loaded and streamed.
#[must_use]
pub fn columns_by_distance(center: ColumnPos, radius: i32) -> Vec<ColumnPos> {
    let mut v: Vec<(i32, ColumnPos)> = Vec::with_capacity(((2 * radius + 1) as usize).pow(2));
    for dx in -radius..=radius {
        for dz in -radius..=radius {
            v.push((
                dx * dx + dz * dz,
                ColumnPos {
                    x: center.x + dx,
                    z: center.z + dz,
                },
            ));
        }
    }
    v.sort_by_key(|&(d, c)| (d, c.x, c.z));
    v.into_iter().map(|(_, c)| c).collect()
}

/// Chebyshev distance between columns.
#[must_use]
pub fn chebyshev(a: ColumnPos, b: ColumnPos) -> i32 {
    (a.x - b.x).abs().max((a.z - b.z).abs())
}
