//! The world thread: one region for now (T2-06 adds regional threading).
//!
//! At 20 TPS it takes messages from the connections, applies loaded and
//! generated columns, moves players by their intents, streams columns to each
//! player nearest first (at most [`MAX_STREAMS`] in flight per player), tells
//! clients to forget columns that left their view, and unloads columns no
//! player needs. The world map is owned by this thread only (no locks around
//! the world, AGENTS.md rule 3); connections talk to it through channels.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use engine_core::jobs::{CancelToken, JobPool, Priority};
use glam::DVec3;
use protocol::{
    Angle8, BlockXyz, ChunkStream, ColumnPayload, Correction, Intent, KickReason, Look, Message,
    NetId, Spawn,
};
use sim::physics::{MapBlocks, PhysicsConfig, PlayerBody, step};
use tokio::sync::mpsc::UnboundedSender;
use world::map::DirtyKind;
use world::storage::ColumnRecord;
use world::{Column, ColumnPos, Registry, WorldMap};
use worldgen::{GenContext, Pipeline};

use crate::input::{apply_fly_steps, correction, flags, physics_input, state_hash, wire_yaw};
use crate::players::{self, SavedPlayer};
use crate::replication::{Candidate, EntityState, Replicator, quantize};
use crate::storage::{Reply, Request, Storage};

/// Simulation rate.
pub const TICK_RATE: u8 = 20;
/// Column streams in flight per player (`networking-protocol.md`).
pub const MAX_STREAMS: usize = 32;
/// Columns are forgotten (client) and unloaded (server) beyond the view
/// distance plus this margin, so a player walking back and forth on a
/// border does not make them flicker.
pub const UNLOAD_MARGIN: i32 = 2;
/// Intents a player may queue; older ones are dropped (a flooding or
/// stalled client cannot grow server memory).
const MAX_QUEUED_INTENTS: usize = 40;
/// Intents applied per tick at most: one is normal, more catch up after
/// jitter, never so many that a client moves faster than it could.
const MAX_INTENTS_PER_TICK: usize = 3;
/// Column loads and generation jobs in flight at most.
const MAX_PENDING: usize = 96;
/// Where a new player appears (x, z); the height comes from the terrain.
const WORLD_SPAWN: [f64; 2] = [8.5, 8.5];
/// Columns around the world spawn stay loaded even with nobody online, so a
/// new player spawns without waiting for generation.
const SPAWN_KEEP: i32 = 2;

fn spawn_column() -> ColumnPos {
    ColumnPos {
        x: (WORLD_SPAWN[0] / 16.0).floor() as i32,
        z: (WORLD_SPAWN[1] / 16.0).floor() as i32,
    }
}

/// A message from a connection to the world thread.
pub enum ToWorld {
    /// A player passed the handshake.
    Join {
        /// Stable player ID (hex, also the save-file name).
        id: String,
        /// Display name.
        name: String,
        /// Messages to this player's connection.
        out: UnboundedSender<ToConn>,
    },
    /// Intents from an input datagram.
    Intents {
        /// The player.
        id: String,
        /// Newest last.
        intents: Vec<Intent>,
        /// The newest entity snapshot the client reconstructed.
        ack_snapshot: u32,
    },
    /// A column stream finished (`ok`) or failed (resend it).
    Delivered {
        /// The player.
        id: String,
        /// The column.
        pos: ColumnPos,
        /// Whether the stream was sent completely.
        ok: bool,
    },
    /// The connection ended.
    Leave {
        /// The player.
        id: String,
    },
}

/// A message from the world thread to one connection.
pub enum ToConn {
    /// Player control starts here.
    Spawn(Spawn),
    /// Send a column on its own stream.
    Column {
        /// The column.
        pos: ColumnPos,
        /// The encoded [`ChunkStream`] message, shared between players.
        bytes: Arc<[u8]>,
        /// QUIC stream priority (higher is sent first).
        priority: i32,
    },
    /// Tell the client to drop a column.
    Forget(ColumnPos),
    /// An encoded `EntityDatagram` (sent as a QUIC datagram).
    Datagram(Vec<u8>),
    /// End the session.
    Kick(KickReason, String),
}

/// What the world thread needs to start.
pub struct WorldSetup {
    /// Block registry.
    pub registry: Arc<Registry>,
    /// Generator context.
    pub context: Arc<GenContext>,
    /// Movement profile.
    pub physics: PhysicsConfig,
    /// The world folder (player files).
    pub dir: PathBuf,
    /// Column radius streamed to each player.
    pub view_distance: i32,
    /// Generation worker threads.
    pub threads: usize,
}

struct Player {
    name: String,
    net_id: NetId,
    /// Pending correction: resent every tick until the client's hashes agree.
    correction: Option<Correction>,
    replicator: Replicator,
    out: UnboundedSender<ToConn>,
    body: PlayerBody,
    yaw: f64,
    pitch: f64,
    fly_speed: f64,
    spawned: bool,
    intents: VecDeque<Intent>,
    last_tick: Option<u32>,
    sent: HashSet<ColumnPos>,
    in_flight: usize,
}

impl Player {
    fn column(&self) -> ColumnPos {
        ColumnPos {
            x: (self.body.pos.x / 16.0).floor() as i32,
            z: (self.body.pos.z / 16.0).floor() as i32,
        }
    }

    fn saved(&self) -> SavedPlayer {
        SavedPlayer {
            pos: self.body.pos.to_array(),
            yaw: self.yaw,
            pitch: self.pitch,
            flying: self.body.flying,
        }
    }
}

/// Columns within `radius` of `center`, nearest first (Chebyshev rings,
/// Euclidean order inside), the order in which they are loaded and streamed.
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

fn chebyshev(a: ColumnPos, b: ColumnPos) -> i32 {
    (a.x - b.x).abs().max((a.z - b.z).abs())
}

enum Job {
    Generated(Option<Column>, ColumnPos),
}

/// The world thread's state.
pub struct World {
    registry: Arc<Registry>,
    map: WorldMap,
    context: Arc<GenContext>,
    pipeline: Arc<Pipeline>,
    pool: JobPool,
    physics: PhysicsConfig,
    swim: Vec<world::BlockStateId>,
    storage: Storage,
    dir: PathBuf,
    view: i32,
    pending: HashSet<ColumnPos>,
    encoded: HashMap<ColumnPos, Arc<[u8]>>,
    players: HashMap<String, Player>,
    unsaved: HashSet<ColumnPos>,
    dirty: Vec<world::coords::SectionPos>,
    next_net_id: u64,
    snapshot: u32,
    corrections: u64,
}

impl World {
    /// Creates the world state (the caller runs [`World::run`] on its thread).
    pub fn new(setup: WorldSetup, storage: Storage) -> Result<Self, String> {
        let pool =
            JobPool::with_low_priority_workers(setup.threads.max(1)).map_err(|e| e.to_string())?;
        let swim = setup
            .physics
            .swim_ids(&setup.registry)
            .map_err(|e| e.to_string())?;
        Ok(Self {
            map: WorldMap::new(Arc::clone(&setup.registry), setup.context.bounds),
            registry: setup.registry,
            context: setup.context,
            pipeline: Arc::new(Pipeline::overworld()),
            pool,
            physics: setup.physics,
            swim,
            storage,
            dir: setup.dir,
            view: setup.view_distance,
            pending: HashSet::new(),
            encoded: HashMap::new(),
            players: HashMap::new(),
            unsaved: HashSet::new(),
            dirty: Vec::new(),
            next_net_id: 1,
            snapshot: 0,
            corrections: 0,
        })
    }

    /// Ticks at 20 TPS until `stop` is set or the connections' sender is
    /// gone, then saves players and edited columns.
    pub fn run(mut self, inbox: Receiver<ToWorld>, stop: Arc<AtomicBool>) {
        let period = Duration::from_secs(1) / u32::from(TICK_RATE);
        let mut next = Instant::now();
        while !stop.load(Ordering::Relaxed) {
            if !self.tick(&inbox) {
                break;
            }
            next += period;
            let now = Instant::now();
            if next > now {
                std::thread::sleep(next - now);
            } else {
                // Behind (a slow tick): do not try to catch up with a burst.
                next = now;
            }
        }
        self.shutdown();
    }

    /// One tick. Returns `false` once no connection can reach the world.
    pub fn tick(&mut self, inbox: &Receiver<ToWorld>) -> bool {
        loop {
            match inbox.try_recv() {
                Ok(msg) => self.handle(msg),
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => return false,
            }
        }
        self.apply_columns();
        self.move_players();
        self.request_columns();
        self.stream();
        self.replicate();
        self.unload();
        true
    }

    fn handle(&mut self, msg: ToWorld) {
        match msg {
            ToWorld::Join { id, name, out } => {
                if self.players.contains_key(&id) {
                    let _ = out.send(ToConn::Kick(
                        KickReason::BadName,
                        format!("{name} is already playing on this server"),
                    ));
                    return;
                }
                let saved = players::load(&self.dir, &id);
                let net_id = NetId(self.next_net_id);
                self.next_net_id += 1;
                let mut p = Player {
                    name,
                    net_id,
                    correction: None,
                    replicator: Replicator::default(),
                    out,
                    body: PlayerBody {
                        pos: DVec3::new(WORLD_SPAWN[0], 0.0, WORLD_SPAWN[1]),
                        ..PlayerBody::default()
                    },
                    yaw: 0.0,
                    pitch: 0.0,
                    fly_speed: 1.0,
                    spawned: false,
                    intents: VecDeque::new(),
                    last_tick: None,
                    sent: HashSet::new(),
                    in_flight: 0,
                };
                if let Some(s) = saved {
                    p.body.pos = DVec3::from_array(s.pos);
                    p.body.flying = s.flying;
                    p.yaw = s.yaw;
                    p.pitch = s.pitch;
                    self.spawn(&mut p);
                } else if let Some(h) = self
                    .map
                    .height(WORLD_SPAWN[0] as i32, WORLD_SPAWN[1] as i32)
                {
                    p.body.pos.y = f64::from(h);
                    self.spawn(&mut p);
                }
                self.players.insert(id, p);
            }
            ToWorld::Intents {
                id,
                intents,
                ack_snapshot,
            } => {
                if let Some(p) = self.players.get_mut(&id)
                    && p.spawned
                {
                    p.replicator.ack(ack_snapshot);
                    for i in intents {
                        let newer = p.last_tick.is_none_or(|t| i.tick > t)
                            && p.intents.back().is_none_or(|b| i.tick > b.tick);
                        if newer {
                            p.intents.push_back(i);
                        }
                    }
                    while p.intents.len() > MAX_QUEUED_INTENTS {
                        p.intents.pop_front();
                    }
                }
            }
            ToWorld::Delivered { id, pos, ok } => {
                if let Some(p) = self.players.get_mut(&id) {
                    p.in_flight = p.in_flight.saturating_sub(1);
                    if !ok {
                        p.sent.remove(&pos);
                    }
                }
            }
            ToWorld::Leave { id } => {
                if let Some(p) = self.players.remove(&id)
                    && p.spawned
                    && let Err(e) = players::save(&self.dir, &id, &p.saved())
                {
                    eprintln!("server: saving player {}: {e}", p.name);
                }
            }
        }
    }

    fn spawn(&self, p: &mut Player) {
        p.spawned = true;
        let b = self.map.bounds();
        let _ = p.out.send(ToConn::Spawn(Spawn {
            tick: 0,
            position: p.body.pos.to_array(),
            yaw: p.yaw as f32,
            pitch: p.pitch as f32,
            min_section_y: b.min_section_y,
            section_count: b.section_count,
            flying: p.body.flying,
            view_distance: self.view.clamp(2, 255) as u8,
        }));
    }

    fn apply_columns(&mut self) {
        let mut generate = Vec::new();
        while let Ok(reply) = self.storage.rx.try_recv() {
            match reply {
                Reply::Loaded(pos, Some(column)) => {
                    self.pending.remove(&pos);
                    let _ = self.map.insert_column(column);
                }
                Reply::Loaded(pos, None) => generate.push(pos),
                Reply::Error(e) => eprintln!("server: {e}"),
            }
        }
        for pos in generate {
            let (pipeline, ctx, registry) = (
                Arc::clone(&self.pipeline),
                Arc::clone(&self.context),
                Arc::clone(&self.registry),
            );
            self.pool
                .submit(Priority::Visible, CancelToken::new(), move |_| {
                    Job::Generated(pipeline.generate(&ctx, pos).into_column(&registry), pos)
                });
        }
        let done: Vec<Job> = self
            .pool
            .drain_completed()
            .filter_map(|j| j.downcast::<Job>())
            .collect();
        for Job::Generated(column, pos) in done {
            self.pending.remove(&pos);
            match column {
                Some(c) => {
                    let _ = self.map.insert_column(c);
                }
                None => eprintln!("server: generating column {pos:?} failed"),
            }
        }
        // New players appear on the ground once the spawn column exists.
        let mut ready = Vec::new();
        for (id, p) in &self.players {
            if !p.spawned
                && let Some(h) = self
                    .map
                    .height(WORLD_SPAWN[0] as i32, WORLD_SPAWN[1] as i32)
            {
                ready.push((id.clone(), h));
            }
        }
        for (id, h) in ready {
            if let Some(mut p) = self.players.remove(&id) {
                p.body.pos.y = f64::from(h);
                self.spawn(&mut p);
                self.players.insert(id, p);
            }
        }
    }

    fn move_players(&mut self) {
        let world = MapBlocks {
            map: &self.map,
            swim: &self.swim,
        };
        for p in self.players.values_mut().filter(|p| p.spawned) {
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
            for _ in 0..MAX_INTENTS_PER_TICK {
                let Some(i) = p.intents.pop_front() else {
                    break;
                };
                p.fly_speed =
                    apply_fly_steps(p.fly_speed, i.fly_speed_steps, self.physics.fly_speed_range);
                let input = physics_input(&i, p.fly_speed);
                step(&mut p.body, &input, &self.physics, &world);
                // Reconciliation: a correction only when the prediction differs.
                if state_hash(&p.body, p.fly_speed) == i.state_hash {
                    if p.correction.is_some_and(|c| i.tick > c.tick) {
                        p.correction = None; // the client has applied it
                    }
                } else {
                    if p.correction.is_none() {
                        self.corrections += 1;
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

    fn request_columns(&mut self) {
        let mut centers: Vec<ColumnPos> = self.players.values().map(Player::column).collect();
        centers.sort_by_key(|c| (c.x, c.z));
        centers.dedup();
        // Round-robin over players by ring, so one player's far columns do
        // not starve another's near ones. The spawn area comes first.
        let mut lists = vec![columns_by_distance(spawn_column(), SPAWN_KEEP)];
        lists.extend(centers.iter().map(|&c| columns_by_distance(c, self.view)));
        let longest = lists.iter().map(Vec::len).max().unwrap_or(0);
        'outer: for i in 0..longest {
            for list in &lists {
                if self.pending.len() >= MAX_PENDING {
                    break 'outer;
                }
                if let Some(&c) = list.get(i)
                    && self.map.column(c).is_none()
                    && self.pending.insert(c)
                {
                    let _ = self.storage.tx.send(Request::Load(c));
                }
            }
        }
    }

    fn stream(&mut self) {
        let view = self.view;
        for p in self.players.values_mut().filter(|p| p.spawned) {
            let center = p.column();
            // Forget what left the view (with a margin).
            let far: Vec<ColumnPos> = p
                .sent
                .iter()
                .copied()
                .filter(|&c| chebyshev(c, center) > view + UNLOAD_MARGIN)
                .collect();
            for c in far {
                p.sent.remove(&c);
                let _ = p.out.send(ToConn::Forget(c));
            }
            if p.in_flight >= MAX_STREAMS {
                continue;
            }
            for c in columns_by_distance(center, view) {
                if p.in_flight >= MAX_STREAMS {
                    break;
                }
                if p.sent.contains(&c) {
                    continue;
                }
                let Some(column) = self.map.column(c) else {
                    continue;
                };
                let bytes = match self.encoded.get(&c) {
                    Some(b) => Arc::clone(b),
                    None => match encode(column) {
                        Ok(b) => {
                            self.encoded.insert(c, Arc::clone(&b));
                            b
                        }
                        Err(e) => {
                            eprintln!("server: encoding column {c:?}: {e}");
                            continue;
                        }
                    },
                };
                let d = chebyshev(c, center);
                if p.out
                    .send(ToConn::Column {
                        pos: c,
                        bytes,
                        priority: -d,
                    })
                    .is_ok()
                {
                    p.sent.insert(c);
                    p.in_flight += 1;
                }
            }
        }
    }

    /// Sends every spawned player this tick's datagram: their
    /// reconciliation data and the deltas of the other players in view.
    fn replicate(&mut self) {
        self.snapshot = self.snapshot.wrapping_add(1).max(1);
        let snapshot = self.snapshot;
        let radius = f64::from(self.view * 16);
        let everyone: Vec<(NetId, DVec3, EntityState)> = self
            .players
            .values()
            .filter(|p| p.spawned)
            .map(|p| {
                let a = |r: f64| Angle8::from_radians(r as f32);
                (
                    p.net_id,
                    p.body.pos,
                    EntityState {
                        kind: 0,
                        pos: quantize(p.body.pos.to_array()),
                        look: Look {
                            yaw: a(p.yaw),
                            pitch: a(p.pitch),
                            head_yaw: a(p.yaw),
                        },
                        flags: flags(&p.body),
                    },
                )
            })
            .collect();
        for p in self.players.values_mut().filter(|p| p.spawned) {
            let me = p.body.pos;
            let candidates = everyone
                .iter()
                .filter(|(id, _, _)| *id != p.net_id)
                .filter_map(|&(id, pos, state)| {
                    let d = (pos - me).length();
                    let h = ((pos.x - me.x).powi(2) + (pos.z - me.z).powi(2)).sqrt();
                    (h <= radius).then_some(Candidate {
                        id,
                        state,
                        distance: d,
                    })
                })
                .collect();
            let origin = BlockXyz {
                x: me.x.floor() as i32,
                y: me.y.floor() as i32,
                z: me.z.floor() as i32,
            };
            let d = p
                .replicator
                .build(snapshot, origin, p.last_tick, p.correction, candidates);
            if let Ok(bytes) = d.encode() {
                let _ = p.out.send(ToConn::Datagram(bytes));
            }
        }
    }

    /// Corrections issued so far (metrics and tests).
    #[must_use]
    pub fn corrections(&self) -> u64 {
        self.corrections
    }

    fn unload(&mut self) {
        self.dirty.clear();
        self.map.take_dirty(DirtyKind::Save, &mut self.dirty);
        for s in &self.dirty {
            self.unsaved.insert(ColumnPos::of_section(*s));
        }
        let centers: Vec<ColumnPos> = self.players.values().map(Player::column).collect();
        let keep = self.view + UNLOAD_MARGIN;
        let gone: Vec<ColumnPos> = self
            .map
            .column_positions()
            .filter(|&c| {
                chebyshev(c, spawn_column()) > SPAWN_KEEP
                    && centers.iter().all(|&p| chebyshev(c, p) > keep)
            })
            .collect();
        let mut records = Vec::new();
        for c in gone {
            self.encoded.remove(&c);
            if let Some(col) = self.map.remove_column(c)
                && self.unsaved.remove(&c)
            {
                records.push(record(&col));
            }
        }
        if !records.is_empty() {
            let _ = self.storage.tx.send(Request::Save(records));
        }
    }

    fn shutdown(&mut self) {
        for (id, p) in self.players.drain() {
            let _ = p.out.send(ToConn::Kick(
                KickReason::ShuttingDown,
                "The server is shutting down".into(),
            ));
            if p.spawned
                && let Err(e) = players::save(&self.dir, &id, &p.saved())
            {
                eprintln!("server: saving player {}: {e}", p.name);
            }
        }
        let records: Vec<ColumnRecord> = self
            .unsaved
            .iter()
            .filter_map(|&c| self.map.column(c))
            .map(record)
            .collect();
        if !records.is_empty() {
            let _ = self.storage.tx.send(Request::Save(records));
        }
        self.storage.stop();
    }

    /// Columns loaded on the server (tests and metrics).
    #[must_use]
    pub fn loaded_columns(&self) -> usize {
        self.map.len()
    }
}

fn record(col: &Column) -> ColumnRecord {
    ColumnRecord {
        pos: col.pos(),
        bounds: col.bounds(),
        sections: col.sections().to_vec(),
        light: Vec::new(),
        entities: Vec::new(),
    }
}

fn encode(col: &Column) -> Result<Arc<[u8]>, protocol::ProtocolError> {
    let payload = ColumnPayload::encode(&record(col))?;
    Ok(ChunkStream::Column(payload).encode()?.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_come_nearest_first() {
        let c = ColumnPos { x: 10, z: -4 };
        let list = columns_by_distance(c, 3);
        assert_eq!(list.len(), 49);
        assert_eq!(list[0], c);
        // The 3×3 around the center comes before anything farther.
        let first9: HashSet<_> = list[..9].iter().copied().collect();
        for dx in -1..=1 {
            for dz in -1..=1 {
                assert!(first9.contains(&ColumnPos {
                    x: c.x + dx,
                    z: c.z + dz
                }));
            }
        }
        let d = |p: &ColumnPos| (p.x - c.x).pow(2) + (p.z - c.z).pow(2);
        assert!(list.windows(2).all(|w| d(&w[0]) <= d(&w[1])));
    }
}
