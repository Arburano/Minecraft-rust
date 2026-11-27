//! The node: the world thread that owns all regions (T2-06, ADR-0008,
//! `server-regions.md`, `engine-core.md` tick phases).
//!
//! Each tick at 20 TPS:
//! 1. **PreTick** (sequential): messages from connections are routed to the
//!    owning region (intents, edits, stream acks); joins and leaves; loaded
//!    and generated columns enter the node's column cache and, inside a
//!    region's territory, that region's live map.
//! 2. **PostTick of the previous tick / restructure** (sequential): the
//!    partition is recomputed ([`crate::partition`]): players whose areas
//!    touch share a region, others split off. Players (ECS entities) and live
//!    columns move to their region; columns no territory covers are released
//!    (saved if edited). A region is never restructured while it ticks.
//! 3. **Simulate** (regions in parallel on the region pool): each region
//!    moves its players, applies edits, streams columns and replicates. It
//!    reads only its own state and the node's immutable shared state.
//! 4. **Exchange** (sequential): block changes go to players of other regions
//!    that hold the column, edited columns are re-encoded into the cache,
//!    column requests go to storage and generation, and the player snapshot
//!    for the next tick is taken.
//!
//! Territories: a player's area is the columns within the simulation radius
//! plus a 1-column buffer; regions never border each other directly. The
//! columns between the simulation radius and the view radius are kept only
//! encoded in the node's cache (the "LRU of compressed columns for sending").

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use engine_core::jobs::{CancelToken, JobPool, Priority};
use glam::DVec3;
use protocol::{BlockChange, Intent, KickReason, MAX_BLOCK_CHANGES, NetId, ServerEvent};
use rayon::prelude::*;
use sim::physics::{PhysicsConfig, PlayerBody};
use tokio::sync::mpsc::UnboundedSender;
use world::coords::BlockPos;
use world::storage::ColumnRecord;
use world::{Column, ColumnPos, Registry, WorldBounds};
use worldgen::{GenContext, Pipeline};

use crate::columns::{decode, encode, record};
use crate::entities::{Walker, astar};
use crate::partition::{Member, RegionId, partition};
use crate::players;
use crate::region::{EditRequest, Player, PlayerSnapshot, Region, Shared, chebyshev};
use crate::replication::Replicator;
use crate::storage::{Reply, Request, Storage};

/// Simulation rate.
pub const TICK_RATE: u8 = 20;
/// Column streams in flight per player (`networking-protocol.md`; 32, see T2-02).
pub const MAX_STREAMS: usize = 32;
/// Columns are forgotten (client) and dropped from the cache (server)
/// beyond the view distance plus this margin, so a player walking back and
/// forth on a border does not make them flicker.
pub const UNLOAD_MARGIN: i32 = 2;
/// Column loads and generation jobs in flight at most.
const MAX_PENDING: usize = 128;
/// Cached columns decoded into a region's live map per tick at most.
const MAX_DECODES_PER_TICK: usize = 64;
/// Where a new player appears (x, z); the height comes from the terrain.
const WORLD_SPAWN: [f64; 2] = [8.5, 8.5];
/// Columns around the world spawn stay live even with nobody online, so a
/// new player spawns without waiting for generation.
const SPAWN_KEEP: i32 = 2;
/// The partition key of the spawn anchor.
const ANCHOR: &str = "";

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
    /// A block edit (predicted on the client).
    Edit {
        /// The player.
        id: String,
        /// The client's sequence number, echoed in the ack.
        seq: u32,
        /// The cell.
        pos: BlockPos,
        /// What to do.
        edit: Edit,
        /// `false` if the connection's rate limit refused it.
        allowed: bool,
    },
    /// The connection ended.
    Leave {
        /// The player.
        id: String,
    },
}

/// A block edit request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edit {
    /// Break the block (set air).
    Dig,
    /// Place this state (server numbering).
    Place(u16),
}

/// A message from the world thread to one connection.
pub enum ToConn {
    /// Player control starts here.
    Spawn(protocol::Spawn),
    /// Send a column on its own stream.
    Column {
        /// The column.
        pos: ColumnPos,
        /// The encoded `ChunkStream` message, shared between players.
        bytes: Arc<[u8]>,
        /// QUIC stream priority (higher is sent first).
        priority: i32,
    },
    /// An event on the events stream (column unloads, block changes, acks).
    Event(ServerEvent),
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
    /// Column radius simulated around each player (≤ view distance).
    pub sim_distance: i32,
    /// Generation worker threads.
    pub threads: usize,
    /// Region tick threads.
    pub region_threads: usize,
}

/// A column with its encoded message.
type Loaded = (Column, Arc<[u8]>);

enum Job {
    Generated(ColumnPos, Option<(Column, Arc<[u8]>)>),
    /// A cached column decoded to become live again.
    Decoded(ColumnPos, Option<Column>),
    /// A path for an entity (T2-07).
    Path(NetId, Option<Vec<BlockPos>>),
}

/// Tick timing of the last tick (metrics, the T2-06 benchmark).
#[derive(Clone, Copy, Debug, Default)]
pub struct TickTimes {
    /// The whole tick.
    pub total: Duration,
    /// `PreTick`: messages and arriving columns.
    pub pre: Duration,
    /// The restructure (partition, moves).
    pub restructure: Duration,
    /// `Exchange`.
    pub exchange: Duration,
    /// The parallel `Simulate` phase.
    pub simulate: Duration,
    /// The slowest region's own tick.
    pub slowest_region: Duration,
    /// Regions.
    pub regions: usize,
}

/// The world thread's state.
pub struct World {
    registry: Arc<Registry>,
    context: Arc<GenContext>,
    pipeline: Arc<Pipeline>,
    jobs: JobPool,
    pool: rayon::ThreadPool,
    physics: PhysicsConfig,
    swim: Vec<world::BlockStateId>,
    storage: Storage,
    dir: PathBuf,
    view: i32,
    sim: i32,
    bounds: WorldBounds,
    /// Encoded messages of every loaded column (live or not).
    cache: HashMap<ColumnPos, Arc<[u8]>>,
    pending: HashSet<ColumnPos>,
    regions: Vec<Region>,
    /// Which region each player is in.
    owner: HashMap<String, RegionId>,
    anchor_region: RegionId,
    snapshot_players: Vec<PlayerSnapshot>,
    next_region_id: u64,
    next_net_id: u64,
    snapshot: u32,
    corrections: u64,
    edit_count: u64,
    times: TickTimes,
    ticks: u64,
    /// View offsets, nearest first.
    offsets: Vec<(i32, i32)>,
    /// Territory: column → (owning region, how many member areas cover it).
    coverage: HashMap<ColumnPos, (RegionId, u32)>,
    /// The members the last restructure saw, with their regions.
    last_members: Vec<(String, ColumnPos, RegionId)>,
    /// Territory columns that are not live yet (decoded from the cache a
    /// few per tick, or requested).
    to_fill: Vec<ColumnPos>,
    /// Cached columns being decoded on the job pool.
    decoding: HashSet<ColumnPos>,
    /// Which region each simulated entity is in.
    walker_owner: HashMap<NetId, RegionId>,
    /// Entities outside every territory: stored, not simulated.
    dormant: BTreeMap<NetId, Walker>,
    entity_stats: EntityStats,
    log: Option<Arc<crate::metrics::TickLog>>,
}

/// Entity simulation counters since the last [`World::entity_stats`].
#[derive(Clone, Debug, Default)]
pub struct EntityStats {
    /// Simulated entities now.
    pub live: usize,
    /// Stored entities outside every territory.
    pub dormant: usize,
    /// Path requests waiting for their job now.
    pub in_flight: usize,
    /// Physics steps of entities.
    pub simulated: u64,
    /// AI decisions.
    pub decisions: u64,
    /// Path requests made.
    pub requested: u64,
    /// Path jobs finished.
    pub paths_done: u64,
    /// The most new requests one region made in one tick.
    pub max_requests_per_region: usize,
    /// Ticks from request to path, per finished request.
    pub latency_ticks: Vec<u64>,
}

impl World {
    /// Creates the world state (the caller runs [`World::run`] on its thread).
    pub fn new(setup: WorldSetup, storage: Storage) -> Result<Self, String> {
        let jobs =
            JobPool::with_low_priority_workers(setup.threads.max(1)).map_err(|e| e.to_string())?;
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(setup.region_threads.max(1))
            .thread_name(|i| format!("region-{i}"))
            .build()
            .map_err(|e| e.to_string())?;
        let swim = setup
            .physics
            .swim_ids(&setup.registry)
            .map_err(|e| e.to_string())?;
        let bounds = setup.context.bounds;
        let anchor = Region::new(RegionId(1), Arc::clone(&setup.registry), bounds);
        Ok(Self {
            registry: setup.registry,
            context: setup.context,
            pipeline: Arc::new(Pipeline::overworld()),
            jobs,
            pool,
            physics: setup.physics,
            swim,
            storage,
            dir: setup.dir,
            view: setup.view_distance,
            sim: setup.sim_distance.clamp(2, setup.view_distance.max(2)),
            bounds,
            cache: HashMap::new(),
            pending: HashSet::new(),
            regions: vec![anchor],
            owner: HashMap::new(),
            anchor_region: RegionId(1),
            snapshot_players: Vec::new(),
            next_region_id: 2,
            next_net_id: 1,
            snapshot: 0,
            corrections: 0,
            edit_count: 0,
            times: TickTimes::default(),
            ticks: 0,
            offsets: crate::region::columns_by_distance(
                ColumnPos { x: 0, z: 0 },
                setup.view_distance,
            )
            .into_iter()
            .map(|c| (c.x, c.z))
            .collect(),
            coverage: HashMap::new(),
            last_members: Vec::new(),
            to_fill: Vec::new(),
            decoding: HashSet::new(),
            walker_owner: HashMap::new(),
            dormant: BTreeMap::new(),
            entity_stats: EntityStats::default(),
            log: None,
        })
    }

    /// Records every tick into `log` from now on.
    pub fn set_log(&mut self, log: Arc<crate::metrics::TickLog>) {
        self.log = Some(log);
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
        let start = Instant::now();
        // PreTick.
        loop {
            match inbox.try_recv() {
                Ok(msg) => self.handle(msg),
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => return false,
            }
        }
        self.receive_columns();
        let pre = start.elapsed();
        // Restructure between ticks.
        let t = Instant::now();
        self.restructure();
        self.relocate_walkers();
        let restructure = t.elapsed();
        // Simulate.
        let simulate = Instant::now();
        self.snapshot = self.snapshot.wrapping_add(1).max(1);
        {
            let shared = Shared {
                registry: &self.registry,
                physics: &self.physics,
                swim: &self.swim,
                view: self.view,
                cache: &self.cache,
                snapshot: self.snapshot,
                spawn: WORLD_SPAWN,
                offsets: &self.offsets,
                players: &self.snapshot_players,
                tick: self.ticks,
            };
            let regions = &mut self.regions;
            self.pool
                .install(|| regions.par_iter_mut().for_each(|r| r.tick(&shared)));
        }
        let simulate = simulate.elapsed();
        // Exchange.
        let t = Instant::now();
        self.exchange();
        let exchange = t.elapsed();
        self.ticks += 1;
        self.times = TickTimes {
            total: start.elapsed(),
            pre,
            restructure,
            exchange,
            simulate,
            slowest_region: self
                .regions
                .iter()
                .map(|r| r.out.time)
                .max()
                .unwrap_or_default(),
            regions: self.regions.len(),
        };
        if let Some(log) = &self.log {
            log.push(
                self.times.total,
                self.owner.len(),
                self.regions.iter().filter(|r| !r.index.is_empty()).count(),
            );
        }
        true
    }

    fn region_mut(&mut self, id: RegionId) -> Option<&mut Region> {
        self.regions.iter_mut().find(|r| r.id == id)
    }

    fn player_region(&mut self, player: &str) -> Option<&mut Region> {
        let id = *self.owner.get(player)?;
        self.region_mut(id)
    }

    fn handle(&mut self, msg: ToWorld) {
        match msg {
            ToWorld::Join { id, name, out } => {
                if self.owner.contains_key(&id) {
                    let _ = out.send(ToConn::Kick(
                        KickReason::BadName,
                        format!("{name} is already playing on this server"),
                    ));
                    return;
                }
                let net_id = NetId(self.next_net_id);
                self.next_net_id += 1;
                let mut p = Player {
                    id: id.clone(),
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
                    complete_at: None,
                };
                if let Some(s) = players::load(&self.dir, &id) {
                    p.body.pos = DVec3::from_array(s.pos);
                    p.body.flying = s.flying;
                    p.yaw = s.yaw;
                    p.pitch = s.pitch;
                    Region::send_spawn(&mut p, self.bounds, self.view);
                }
                // A new player waits in the spawn region (its region's tick
                // spawns it on the ground); the restructure moves a returning
                // one to where it stands.
                let anchor = self.anchor_region;
                self.owner.insert(id, anchor);
                if let Some(r) = self.region_mut(anchor) {
                    r.insert(p);
                }
            }
            ToWorld::Intents {
                id,
                intents,
                ack_snapshot,
            } => {
                if let Some(r) = self.player_region(&id)
                    && let Some(mut p) = r.player_mut(&id)
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
                    while p.intents.len() > crate::region::MAX_QUEUED_INTENTS {
                        p.intents.pop_front();
                    }
                }
            }
            ToWorld::Delivered { id, pos, ok } => {
                if let Some(r) = self.player_region(&id)
                    && let Some(mut p) = r.player_mut(&id)
                {
                    p.in_flight = p.in_flight.saturating_sub(1);
                    if !ok {
                        p.sent.remove(&pos);
                        p.complete_at = None;
                    }
                }
            }
            ToWorld::Edit {
                id,
                seq,
                pos,
                edit,
                allowed,
            } => {
                if let Some(r) = self.player_region(&id) {
                    r.edits.push(EditRequest {
                        id,
                        seq,
                        pos,
                        edit,
                        allowed,
                    });
                }
            }
            ToWorld::Leave { id } => {
                let Some(rid) = self.owner.remove(&id) else {
                    return;
                };
                if let Some(r) = self.region_mut(rid)
                    && let Some(p) = r.take(&id)
                    && p.spawned
                    && let Err(e) = players::save(&self.dir, &id, &p.saved())
                {
                    eprintln!("server: saving player {}: {e}", p.name);
                }
            }
        }
    }

    /// Loaded and generated columns: into the cache, and live into the
    /// region whose territory covers them.
    fn receive_columns(&mut self) {
        let mut arrived: Vec<(ColumnPos, Option<Loaded>)> = Vec::new();
        while let Ok(reply) = self.storage.rx.try_recv() {
            match reply {
                Reply::Loaded(pos, Some(c)) => arrived.push((pos, Some(c))),
                Reply::Loaded(pos, None) => {
                    let (pipeline, ctx, registry) = (
                        Arc::clone(&self.pipeline),
                        Arc::clone(&self.context),
                        Arc::clone(&self.registry),
                    );
                    self.jobs
                        .submit(Priority::Visible, CancelToken::new(), move |_| {
                            let col = pipeline.generate(&ctx, pos).into_column(&registry);
                            Job::Generated(pos, col.and_then(|c| encode(&c).ok().map(|b| (c, b))))
                        });
                }
                Reply::Error(e) => eprintln!("server: {e}"),
            }
        }
        for job in self
            .jobs
            .drain_completed()
            .filter_map(|j| j.downcast::<Job>())
            .collect::<Vec<_>>()
        {
            match job {
                Job::Generated(pos, c) => arrived.push((pos, c)),
                Job::Path(id, path) => self.apply_path(id, path),
                Job::Decoded(pos, c) => {
                    self.decoding.remove(&pos);
                    if let Some(col) = c
                        && let Some(&(rid, _)) = self.coverage.get(&pos)
                        && let Some(r) = self.region_mut(rid)
                        && r.map.column(pos).is_none()
                    {
                        let _ = r.map.insert_column(col);
                    }
                }
            }
        }
        if arrived.is_empty() {
            return;
        }
        for (pos, c) in arrived {
            self.pending.remove(&pos);
            let Some((column, bytes)) = c else {
                eprintln!("server: column {pos:?} could not be loaded or generated");
                continue;
            };
            self.cache.insert(pos, bytes);
            if let Some(&(rid, _)) = self.coverage.get(&pos)
                && let Some(r) = self.region_mut(rid)
                && r.map.column(pos).is_none()
            {
                let _ = r.map.insert_column(column);
            }
        }
    }

    /// The partition members: the spawn anchor and every player.
    fn members(&self) -> Vec<Member> {
        let mut m = vec![Member {
            key: ANCHOR.into(),
            column: spawn_column(),
            radius: SPAWN_KEEP,
            region: Some(self.anchor_region),
        }];
        for r in &self.regions {
            for id in r.player_ids() {
                if let Some(p) = r.player(id) {
                    m.push(Member {
                        key: id.to_owned(),
                        column: p.column(),
                        radius: self.sim + 1,
                        region: Some(r.id),
                    });
                }
            }
        }
        m.sort_by(|a, b| a.key.cmp(&b.key));
        m
    }

    /// Recomputes the partition and moves players and live columns
    /// (merge and split, `PostTick`). When only positions changed and every
    /// member keeps its region, the territories are updated incrementally:
    /// a member that crossed a column border adds and removes only the edge
    /// strips of its area.
    pub fn restructure(&mut self) {
        let members = self.members();
        let same_keys = members.len() == self.last_members.len()
            && members
                .iter()
                .zip(&self.last_members)
                .all(|(m, l)| m.key == l.0);
        if same_keys
            && members
                .iter()
                .zip(&self.last_members)
                .all(|(m, l)| m.column == l.1)
        {
            self.fill_territories();
            return;
        }
        let assigned = partition(&members, &mut self.next_region_id);
        let same_partition = same_keys
            && members
                .iter()
                .zip(&assigned)
                .all(|(m, &a)| m.region == Some(a));
        if same_partition {
            self.shift_territories(&members);
        } else {
            self.repartition(&members, &assigned);
        }
        self.last_members = members
            .iter()
            .zip(&assigned)
            .map(|(m, &a)| (m.key.clone(), m.column, a))
            .collect();
        self.fill_territories();
    }

    fn area(m: &Member) -> impl Iterator<Item = ColumnPos> + '_ {
        (-m.radius..=m.radius).flat_map(move |dx| {
            (-m.radius..=m.radius).map(move |dz| ColumnPos {
                x: m.column.x + dx,
                z: m.column.z + dz,
            })
        })
    }

    /// Positions changed, regions did not: move area strips.
    fn shift_territories(&mut self, members: &[Member]) {
        let mut uncovered: Vec<ColumnPos> = Vec::new();
        for (m, last) in members.iter().zip(self.last_members.clone()) {
            if m.column == last.1 {
                continue;
            }
            let rid = m.region.unwrap_or_default();
            let old = Member {
                column: last.1,
                ..m.clone()
            };
            for c in Self::area(&old) {
                if !m.covers(c)
                    && let Some(e) = self.coverage.get_mut(&c)
                {
                    e.1 -= 1;
                    if e.1 == 0 {
                        uncovered.push(c);
                    }
                }
            }
            for c in Self::area(m) {
                if old.covers(c) {
                    continue;
                }
                let e = self.coverage.entry(c).or_insert((rid, 0));
                if e.1 == 0 {
                    self.to_fill.push(c);
                }
                e.1 += 1;
            }
        }
        let mut released = Vec::new();
        uncovered.sort_by_key(|c| (c.x, c.z));
        uncovered.dedup();
        for c in uncovered {
            let Some(&(rid, 0)) = self.coverage.get(&c) else {
                continue; // covered again by another member
            };
            self.coverage.remove(&c);
            if let Some(r) = self.regions.iter_mut().find(|r| r.id == rid)
                && let Some(col) = r.map.remove_column(c)
                && r.unsaved.remove(&c)
            {
                released.push(record(&col));
            }
        }
        if !released.is_empty() {
            let _ = self.storage.tx.send(Request::Save(released));
        }
    }

    /// Regions merged, split, appeared or a member joined or left: move
    /// players and live columns to the new partition.
    fn repartition(&mut self, members: &[Member], assigned: &[RegionId]) {
        let wanted: BTreeMap<RegionId, ()> = assigned.iter().map(|&r| (r, ())).collect();
        for &rid in wanted.keys() {
            if !self.regions.iter().any(|r| r.id == rid) {
                self.regions
                    .push(Region::new(rid, Arc::clone(&self.registry), self.bounds));
            }
        }
        // Players move (ECS entity → ECS entity, by stable ID).
        for (m, &to) in members.iter().zip(assigned) {
            if m.key == ANCHOR {
                self.anchor_region = to;
                continue;
            }
            let from = m.region.unwrap_or(to);
            if from == to {
                continue;
            }
            let p = self.region_mut(from).and_then(|r| r.take(&m.key));
            if let (Some(p), Some(r)) = (p, self.region_mut(to)) {
                r.insert(p);
                self.owner.insert(m.key.clone(), to);
            }
        }
        // The territory from scratch.
        self.coverage.clear();
        for (m, &to) in members.iter().zip(assigned) {
            let m = Member {
                region: Some(to),
                ..m.clone()
            };
            for c in Self::area(&m) {
                self.coverage.entry(c).or_insert((to, 0)).1 += 1;
            }
        }
        // Live columns follow the territories.
        let mut moving: Vec<(RegionId, Column, bool)> = Vec::new();
        let mut released: Vec<ColumnRecord> = Vec::new();
        for r in &mut self.regions {
            let mut gone: Vec<ColumnPos> = r
                .map
                .column_positions()
                .filter(|c| self.coverage.get(c).map(|e| e.0) != Some(r.id))
                .collect();
            gone.sort_by_key(|c| (c.x, c.z));
            for c in gone {
                let Some(col) = r.map.remove_column(c) else {
                    continue;
                };
                let unsaved = r.unsaved.remove(&c);
                match self.coverage.get(&c) {
                    // Edited columns stay edited with their new owner.
                    Some(&(to, _)) => moving.push((to, col, unsaved)),
                    None if unsaved => released.push(record(&col)),
                    None => {}
                }
            }
        }
        for (to, col, unsaved) in moving {
            let pos = col.pos();
            if let Some(r) = self.region_mut(to) {
                let _ = r.map.insert_column(col);
                if unsaved {
                    r.unsaved.insert(pos);
                }
            }
        }
        if !released.is_empty() {
            let _ = self.storage.tx.send(Request::Save(released));
        }
        // Regions with nobody (and not the spawn's) disappear.
        let anchor = self.anchor_region;
        self.regions
            .retain(|r| r.id == anchor || !r.index.is_empty() || !r.map.is_empty());
        self.regions.sort_by_key(|r| r.id);
        let mut all: Vec<ColumnPos> = self.coverage.keys().copied().collect();
        all.sort_by_key(|c| (c.x, c.z));
        self.to_fill = all;
    }

    /// Makes territory columns live: decoded from the cache (bounded per
    /// tick) or requested from storage and generation.
    fn fill_territories(&mut self) {
        if self.to_fill.is_empty() {
            return;
        }
        let list = std::mem::take(&mut self.to_fill);
        let mut decoded = 0;
        let mut wanted = Vec::new();
        for c in list {
            let Some(&(rid, _)) = self.coverage.get(&c) else {
                continue; // no longer in any territory
            };
            let Some(r) = self.regions.iter_mut().find(|r| r.id == rid) else {
                continue;
            };
            if r.map.column(c).is_some() {
                continue;
            }
            if self.decoding.contains(&c) {
                continue;
            }
            match self.cache.get(&c) {
                Some(bytes) if decoded < MAX_DECODES_PER_TICK => {
                    // Decoding takes ≈ 0.1–0.3 ms: on the job pool, not in the tick.
                    let (bytes, registry) = (Arc::clone(bytes), Arc::clone(&self.registry));
                    self.decoding.insert(c);
                    self.jobs
                        .submit(Priority::Visible, CancelToken::new(), move |_| {
                            Job::Decoded(c, decode(&bytes, &registry))
                        });
                    decoded += 1;
                }
                // Over this tick's budget: next tick.
                Some(_) => self.to_fill.push(c),
                // It arrives through `receive_columns`, which makes it live.
                None => wanted.push(c),
            }
        }
        let left = self.request(wanted);
        self.to_fill.extend(left);
    }

    /// Requests columns from storage (generation if never saved). Returns
    /// those left over because too many are already in flight.
    fn request(&mut self, columns: Vec<ColumnPos>) -> Vec<ColumnPos> {
        let mut left = Vec::new();
        for c in columns {
            if self.cache.contains_key(&c) || self.pending.contains(&c) {
                continue;
            }
            if self.pending.len() >= MAX_PENDING {
                left.push(c);
                continue;
            }
            self.pending.insert(c);
            let _ = self.storage.tx.send(Request::Load(c));
        }
        left
    }

    /// A finished path job: the entity follows it from this tick on (it may
    /// have moved to another region meanwhile, or gone dormant).
    fn apply_path(&mut self, id: NetId, path: Option<Vec<BlockPos>>) {
        self.entity_stats.paths_done += 1;
        let tick = self.ticks;
        let Some(&rid) = self.walker_owner.get(&id) else {
            if let Some(w) = self.dormant.get_mut(&id) {
                w.waiting = false;
            }
            return;
        };
        if let Some(r) = self.region_mut(rid)
            && let Some(mut w) = r.walker_mut(id)
        {
            let latency = tick.saturating_sub(w.requested_at);
            w.waiting = false;
            w.path = path.unwrap_or_default();
            w.next = 0;
            self.entity_stats.latency_ticks.push(latency);
        }
    }

    /// Entities follow the territories: into the region covering their
    /// column, or dormant (stored, not simulated) outside every territory.
    fn relocate_walkers(&mut self) {
        let mut moves: Vec<(RegionId, NetId, Option<RegionId>)> = Vec::new();
        for r in &self.regions {
            for w in r.walkers() {
                let c = ColumnPos::of_block(w.cell());
                let to = self.coverage.get(&c).map(|e| e.0);
                if to != Some(r.id) {
                    moves.push((r.id, w.net_id, to));
                }
            }
        }
        for (from, id, to) in moves {
            let Some(w) = self.region_mut(from).and_then(|r| r.take_walker(id)) else {
                continue;
            };
            match to.and_then(|to| self.region_mut(to).map(|r| (to, r))) {
                Some((to, r)) => {
                    r.insert_walker(w);
                    self.walker_owner.insert(id, to);
                }
                None => {
                    self.walker_owner.remove(&id);
                    self.dormant.insert(id, w);
                }
            }
        }
        if self.dormant.is_empty() {
            return;
        }
        let wake: Vec<(NetId, RegionId)> = self
            .dormant
            .iter()
            .filter_map(|(&id, w)| {
                let c = ColumnPos::of_block(w.cell());
                self.coverage.get(&c).map(|e| (id, e.0))
            })
            .collect();
        for (id, to) in wake {
            if let Some(w) = self.dormant.remove(&id)
                && let Some(r) = self.region_mut(to)
            {
                r.insert_walker(w);
                self.walker_owner.insert(id, to);
            }
        }
    }

    /// Spawns test entities around `center` (on the ground once their
    /// column is live). Returns their IDs.
    pub fn spawn_walkers(&mut self, center: DVec3, count: usize, radius: f64) -> Vec<NetId> {
        let mut ids = Vec::with_capacity(count);
        for i in 0..count {
            let id = NetId(self.next_net_id);
            self.next_net_id += 1;
            // A deterministic spread (golden angle spiral).
            let a = i as f64 * 2.399_963;
            let d = radius * ((i as f64 + 0.5) / count.max(1) as f64).sqrt();
            let pos = DVec3::new(center.x + d * a.cos(), center.y, center.z + d * a.sin());
            self.dormant.insert(id, Walker::new(id, pos, false));
            ids.push(id);
        }
        self.relocate_walkers();
        ids
    }

    /// Where an entity is, if it exists.
    #[must_use]
    pub fn walker_pos(&self, id: NetId) -> Option<DVec3> {
        if let Some(w) = self.dormant.get(&id) {
            return Some(w.body.pos);
        }
        let rid = self.walker_owner.get(&id)?;
        self.regions
            .iter()
            .find(|r| r.id == *rid)?
            .walkers()
            .find(|w| w.net_id == id)
            .map(|w| w.body.pos)
    }

    /// Entity simulation counters (the T2-07 measurement); `take` drains
    /// the latency samples.
    pub fn entity_stats(&mut self) -> EntityStats {
        let s = std::mem::take(&mut self.entity_stats);
        EntityStats {
            live: self.regions.iter().map(|r| r.walkers.len()).sum(),
            dormant: self.dormant.len(),
            in_flight: self
                .regions
                .iter()
                .flat_map(Region::walkers)
                .filter(|w| w.waiting)
                .count(),
            ..s
        }
    }

    fn exchange(&mut self) {
        // Block changes for players of other regions, and fresh cache bytes
        // of edited columns.
        let mut all_changes: Vec<(RegionId, ColumnPos, BlockChange)> = Vec::new();
        let mut edited: BTreeMap<ColumnPos, RegionId> = BTreeMap::new();
        let mut needs: Vec<ColumnPos> = Vec::new();
        let mut paths: Vec<crate::entities::PathRequest> = Vec::new();
        for r in &mut self.regions {
            self.entity_stats.max_requests_per_region = self
                .entity_stats
                .max_requests_per_region
                .max(r.out.path_requests.len());
            self.entity_stats.decisions += r.out.decisions;
            self.entity_stats.simulated += r.out.simulated;
            paths.append(&mut r.out.path_requests);
            self.corrections += r.out.corrections;
            self.edit_count += r.out.changes.len() as u64;
            for &(c, b) in &r.out.changes {
                all_changes.push((r.id, c, b));
                edited.insert(c, r.id);
            }
            needs.append(&mut r.out.needs);
        }
        for (&c, &rid) in &edited {
            if let Some(r) = self.regions.iter().find(|r| r.id == rid)
                && let Some(col) = r.map.column(c)
                && let Ok(bytes) = encode(col)
            {
                self.cache.insert(c, bytes);
            }
        }
        if !all_changes.is_empty() {
            for r in &self.regions {
                for id in r.player_ids() {
                    let Some(p) = r.player(id) else { continue };
                    let mine: Vec<BlockChange> = all_changes
                        .iter()
                        .filter(|(from, c, _)| *from != r.id && p.sent.contains(c))
                        .map(|(_, _, b)| *b)
                        .collect();
                    for chunk in mine.chunks(MAX_BLOCK_CHANGES) {
                        let _ = p
                            .out
                            .send(ToConn::Event(ServerEvent::BlockChanges(chunk.to_vec())));
                    }
                }
            }
        }
        // Columns players should see, nearest first per region order.
        needs.sort_by_key(|c| (c.x, c.z));
        needs.dedup();
        // Leftovers are asked for again next tick.
        let _ = self.request(needs);
        // Path jobs: near entities first (`Visible`), the rest `Prefetch`.
        self.entity_stats.requested += paths.len() as u64;
        for req in paths {
            let class = if req.near {
                Priority::Visible
            } else {
                Priority::Prefetch
            };
            self.jobs.submit(class, CancelToken::new(), move |_| {
                Job::Path(req.net_id, astar(&req.grid, req.start, req.goal))
            });
        }
        // The snapshot for the next tick (other regions read it).
        self.snapshot_players.clear();
        for r in &self.regions {
            for id in r.player_ids() {
                if let Some(p) = r.player(id).filter(|p| p.spawned) {
                    self.snapshot_players.push(PlayerSnapshot::of(p, r.id));
                }
            }
            for w in r.walkers() {
                self.snapshot_players
                    .push(PlayerSnapshot::of_walker(w, r.id));
            }
        }
        // Drop cached columns nobody can see (every second).
        if self.ticks % u64::from(TICK_RATE) == 0 {
            self.evict();
        }
    }

    fn evict(&mut self) {
        let keep = self.view + UNLOAD_MARGIN;
        let centers: Vec<ColumnPos> = self
            .regions
            .iter()
            .flat_map(|r| {
                r.player_ids()
                    .filter_map(|id| r.player(id))
                    .map(Player::column)
            })
            .collect();
        let live: HashSet<ColumnPos> = self
            .regions
            .iter()
            .flat_map(|r| r.map.column_positions())
            .collect();
        // Players hashed into cells of `keep + 1` columns: a cached column
        // only checks the players of its own and the 8 neighboring cells.
        let cell = keep + 1;
        let key = |c: ColumnPos| (c.x.div_euclid(cell), c.z.div_euclid(cell));
        let mut grid: HashMap<(i32, i32), Vec<ColumnPos>> = HashMap::new();
        for p in centers {
            grid.entry(key(p)).or_default().push(p);
        }
        self.cache.retain(|c, _| {
            if live.contains(c) || chebyshev(*c, spawn_column()) <= SPAWN_KEEP {
                return true;
            }
            let (gx, gz) = key(*c);
            (-1..=1).any(|dx| {
                (-1..=1).any(|dz| {
                    grid.get(&(gx + dx, gz + dz))
                        .is_some_and(|ps| ps.iter().any(|&p| chebyshev(*c, p) <= keep))
                })
            })
        });
    }

    fn shutdown(&mut self) {
        let mut records = Vec::new();
        for r in &mut self.regions {
            let ids: Vec<String> = r.player_ids().map(str::to_owned).collect();
            for id in ids {
                if let Some(p) = r.take(&id) {
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
            }
            for &c in &r.unsaved {
                if let Some(col) = r.map.column(c) {
                    records.push(record(col));
                }
            }
        }
        if !records.is_empty() {
            let _ = self.storage.tx.send(Request::Save(records));
        }
        self.storage.stop();
    }

    /// Corrections issued so far (metrics and tests).
    #[must_use]
    pub fn corrections(&self) -> u64 {
        self.corrections
    }

    /// Accepted block edits so far (metrics and tests).
    #[must_use]
    pub fn edits(&self) -> u64 {
        self.edit_count
    }

    /// Timing of the last tick.
    #[must_use]
    pub fn times(&self) -> TickTimes {
        self.times
    }

    /// Regions with their sorted player IDs, by region ID.
    #[must_use]
    pub fn regions(&self) -> Vec<(RegionId, Vec<String>)> {
        self.regions
            .iter()
            .map(|r| (r.id, r.player_ids().map(str::to_owned).collect()))
            .collect()
    }

    /// Players across all regions.
    #[must_use]
    pub fn player_count(&self) -> usize {
        self.regions.iter().map(|r| r.index.len()).sum()
    }

    /// Live columns across all regions and cached columns.
    #[must_use]
    pub fn columns(&self) -> (usize, usize) {
        (
            self.regions.iter().map(Region::live_columns).sum(),
            self.cache.len(),
        )
    }

    /// A hash of the partition and every player's simulated state
    /// (reproducibility: same inputs and partitioning → same hash).
    #[must_use]
    pub fn state_hash(&self) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        let mut eat = |bytes: &[u8]| {
            for &b in bytes {
                h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
            }
        };
        for r in &self.regions {
            eat(&r.id.0.to_le_bytes());
            for id in r.player_ids() {
                eat(id.as_bytes());
                if let Some(p) = r.player(id) {
                    for v in p.body.pos.to_array().iter().chain(&p.body.vel.to_array()) {
                        eat(&v.to_bits().to_le_bytes());
                    }
                }
            }
        }
        h
    }

    /// Adds a spawned player without a connection (tests and benchmarks).
    #[doc(hidden)]
    pub fn insert_player_for_test(&mut self, id: &str, pos: DVec3, out: UnboundedSender<ToConn>) {
        let p = Player {
            id: id.to_owned(),
            name: id.to_owned(),
            net_id: NetId(self.next_net_id),
            correction: None,
            replicator: Replicator::default(),
            out,
            body: PlayerBody {
                pos,
                ..PlayerBody::default()
            },
            yaw: 0.0,
            pitch: 0.0,
            fly_speed: 1.0,
            spawned: true,
            intents: VecDeque::new(),
            last_tick: None,
            sent: HashSet::new(),
            in_flight: 0,
            complete_at: None,
        };
        self.next_net_id += 1;
        let anchor = self.anchor_region;
        self.owner.insert(id.to_owned(), anchor);
        if let Some(r) = self.region_mut(anchor) {
            r.insert(p);
        }
    }

    /// Moves a player without simulation (tests of the partition).
    #[doc(hidden)]
    pub fn teleport_for_test(&mut self, id: &str, pos: DVec3) {
        if let Some(r) = self.player_region(id)
            && let Some(mut p) = r.player_mut(id)
        {
            p.body.pos = pos;
        }
    }

    /// Where a player is.
    #[must_use]
    pub fn player_pos(&self, id: &str) -> Option<DVec3> {
        let rid = self.owner.get(id)?;
        self.regions
            .iter()
            .find(|r| r.id == *rid)?
            .player(id)
            .map(|p| p.body.pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::region::columns_by_distance;

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
