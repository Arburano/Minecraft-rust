//! The world thread: one region for now (T2-06 adds regional threading).
//!
//! It owns the block map, the players and what each client has been sent.
//! Columns come from the storage thread (saved) or from generation jobs on
//! the job pool; jobs also pack the network record (palette sections +
//! zstd 1), so the world thread only moves finished bytes around. The
//! connection tasks talk to it through channels; nothing else touches the
//! world (AGENTS.md rule 3: no locks around the world).

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use bytes::Bytes;
use engine_core::jobs::{CancelToken, JobPool, Priority};
use glam::DVec3;
use protocol::messages::buttons;
use protocol::quant::axis_from_i8;
use protocol::{Angle16, ColumnData, FixedPos, InputFrame, Rejection, Spawn};
use sim::physics::{MapBlocks, PhysicsConfig, PlayerBody, PlayerInput, step};
use tokio::sync::{mpsc::UnboundedSender, oneshot};
use world::storage::ColumnRecord;
use world::{BlockStateId, Column, ColumnPos, Registry, WorldBounds, WorldMap};
use worldgen::{GenContext, Pipeline};

use crate::interest::{ClientView, chebyshev, dist2};
use crate::storage::{Loaded, PlayerSave, StorageMsg, StorageThread, load_player};

/// Simulation tick (20 TPS).
pub const TICK: Duration = Duration::from_millis(50);

/// Columns farther than `view + UNLOAD_MARGIN` from every player are dropped
/// from the server's memory.
const UNLOAD_MARGIN: i32 = 2;

/// Queued input frames per player at most (5 s; older ones are dropped).
const MAX_QUEUED_FRAMES: usize = 100;

/// Input frames simulated per player per tick at most (catch-up after the
/// player's area was loading; a client cannot run faster than this).
const MAX_FRAMES_PER_TICK: usize = 4;

/// Where a new player appears (column (0, 0), block centre).
const SPAWN_XZ: (f64, f64) = (8.5, 8.5);

/// What the world thread needs.
pub(crate) struct WorldConfig {
    pub registry: Arc<Registry>,
    pub bounds: WorldBounds,
    pub pipeline: Arc<Pipeline>,
    pub context: Arc<GenContext>,
    pub physics: PhysicsConfig,
    pub swim: Vec<BlockStateId>,
    pub view: i32,
    pub threads: usize,
    pub max_players: usize,
    pub dir: PathBuf,
}

/// A request from a connection task.
pub(crate) enum WorldMsg {
    /// A player joins: `admitted` is answered at once (the client number,
    /// or why not), `placed` once the spawn point is known (a new player
    /// waits for the spawn column, which is generated meanwhile).
    Join {
        player: [u8; 16],
        name: String,
        out: UnboundedSender<ToClient>,
        admitted: oneshot::Sender<Result<u32, Rejection>>,
        placed: oneshot::Sender<Spawn>,
    },
    /// Input frames from a datagram.
    Input {
        client: u32,
        frames: Vec<InputFrame>,
    },
    /// The connection ended.
    Leave { client: u32 },
    /// The storage thread answered a load.
    Loaded(Loaded),
    /// A generation or packing job finished.
    Job(JobOut),
    /// Save everything and stop.
    Shutdown(Sender<()>),
}

/// To a connection task.
#[derive(Debug)]
pub(crate) enum ToClient {
    /// The player's column changed (drop pending columns out of range).
    Center(ColumnPos),
    /// A framed column record.
    Column(ColumnPos, Bytes),
}

struct Player {
    id: [u8; 16],
    name: String,
    body: PlayerBody,
    yaw: f64,
    pitch: f64,
    /// Newest input tick applied or queued.
    last_input: Option<u32>,
    frames: VecDeque<InputFrame>,
    view: ClientView,
    out: UnboundedSender<ToClient>,
    /// Still waiting for the spawn column (new players only).
    placed: Option<oneshot::Sender<Spawn>>,
}

enum Slot {
    /// Asked the storage thread.
    Loading,
    /// A job generates or packs it.
    Working(CancelToken),
    /// In the map; the framed network record.
    Ready(Bytes),
}

pub(crate) enum JobOut {
    Ready(Column, Bytes),
    Failed(ColumnPos),
}

struct World {
    cfg: WorldConfig,
    map: WorldMap,
    slots: HashMap<ColumnPos, Slot>,
    players: HashMap<u32, Player>,
    next_client: u32,
    tick: u32,
    pool: JobPool,
    storage: StorageThread,
    /// The world's own channel: jobs send their results here, so a finished
    /// column wakes the thread at once instead of waiting for the next tick.
    me: Sender<WorldMsg>,
    /// Columns loading or in jobs.
    busy: usize,
    max_busy: usize,
}

/// Runs the world until [`WorldMsg::Shutdown`] or until every sender is gone.
pub(crate) fn run(
    cfg: WorldConfig,
    storage: StorageThread,
    rx: Receiver<WorldMsg>,
    me: Sender<WorldMsg>,
) {
    let pool = match JobPool::new(cfg.threads.max(1)) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("world: cannot start the job pool: {e}");
            return;
        }
    };
    let map = WorldMap::new(Arc::clone(&cfg.registry), cfg.bounds);
    let max_busy = cfg.threads.max(1) * 4;
    let mut w = World {
        cfg,
        map,
        slots: HashMap::new(),
        players: HashMap::new(),
        next_client: 1,
        tick: 0,
        pool,
        storage,
        me,
        busy: 0,
        max_busy,
    };
    let mut next = Instant::now() + TICK;
    loop {
        let now = Instant::now();
        if now >= next {
            w.tick();
            next += TICK;
            if next < Instant::now() {
                // Far behind (a stall): do not run a burst of catch-up ticks.
                next = Instant::now() + TICK;
            }
            continue;
        }
        match rx.recv_timeout(next - now) {
            Ok(WorldMsg::Shutdown(done)) => {
                w.shutdown();
                let _ = done.send(());
                return;
            }
            Ok(msg) => w.handle(msg),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                w.shutdown();
                return;
            }
        }
    }
}

fn column_of(p: DVec3) -> ColumnPos {
    ColumnPos {
        x: (p.x / 16.0).floor() as i32,
        z: (p.z / 16.0).floor() as i32,
    }
}

impl World {
    fn handle(&mut self, msg: WorldMsg) {
        match msg {
            WorldMsg::Join {
                player,
                name,
                out,
                admitted,
                placed,
            } => self.join(player, name, out, admitted, placed),
            WorldMsg::Input { client, frames } => {
                if let Some(p) = self.players.get_mut(&client) {
                    for f in frames {
                        if p.last_input.is_none_or(|last| f.tick > last) {
                            p.last_input = Some(f.tick);
                            p.frames.push_back(f);
                        }
                    }
                    while p.frames.len() > MAX_QUEUED_FRAMES {
                        p.frames.pop_front();
                    }
                }
            }
            WorldMsg::Leave { client } => {
                if let Some(p) = self.players.remove(&client) {
                    self.save_player(&p);
                    println!("server: {} left", p.name);
                }
            }
            WorldMsg::Loaded(Loaded { pos, column }) => {
                if !matches!(self.slots.get(&pos), Some(Slot::Loading)) {
                    return; // unloaded meanwhile
                }
                let column = column.unwrap_or_else(|e| {
                    eprintln!("server: column {pos:?} unreadable ({e}); generating it again");
                    None
                });
                self.submit(pos, column);
            }
            WorldMsg::Job(out) => {
                self.finished(out);
                // Keep the pipeline full between ticks (login latency).
                self.schedule();
            }
            WorldMsg::Shutdown(_) => {}
        }
    }

    fn join(
        &mut self,
        id: [u8; 16],
        name: String,
        out: UnboundedSender<ToClient>,
        admitted: oneshot::Sender<Result<u32, Rejection>>,
        placed: oneshot::Sender<Spawn>,
    ) {
        if self.players.values().any(|p| p.id == id) {
            let _ = admitted.send(Err(Rejection::AlreadyConnected));
            return;
        }
        if self.players.len() >= self.cfg.max_players {
            let _ = admitted.send(Err(Rejection::ServerFull));
            return;
        }
        let client = self.next_client;
        self.next_client = self.next_client.wrapping_add(1).max(1);
        if admitted.send(Ok(client)).is_err() {
            return; // the connection is already gone
        }
        let saved = load_player(&self.cfg.dir, id);
        let body = PlayerBody {
            pos: saved.map_or(DVec3::new(SPAWN_XZ.0, 0.0, SPAWN_XZ.1), |s| {
                DVec3::from_array(s.pos)
            }),
            // `Spawn` carries no flight state, so a returning player starts
            // walking on both sides (flight replication comes with T2-03).
            flying: false,
            ..PlayerBody::default()
        };
        let center = column_of(body.pos);
        let _ = out.send(ToClient::Center(center));
        let mut player = Player {
            id,
            name,
            body,
            yaw: saved.map_or(0.0, |s| s.yaw),
            pitch: saved.map_or(0.0, |s| s.pitch),
            last_input: None,
            frames: VecDeque::new(),
            view: ClientView::new(center, self.cfg.view),
            out,
            placed: Some(placed),
        };
        println!("server: {} joins", player.name);
        if saved.is_some() {
            self.answer_join(client, &mut player);
        }
        self.players.insert(client, player);
        self.deliver_ready(client);
        // Start loading the spawn area now, during the handshake round trips.
        self.schedule();
        self.place_new_players();
    }

    fn answer_join(&self, client: u32, p: &mut Player) {
        if let Some(placed) = p.placed.take() {
            let spawn = Spawn {
                pos: FixedPos::from_blocks(p.body.pos.to_array()),
                yaw: Angle16::from_radians(p.yaw),
                pitch: Angle16::from_radians(p.pitch),
                tick: self.tick,
                entity: client,
                min_section_y: self.cfg.bounds.min_section_y,
                section_count: self.cfg.bounds.section_count,
            };
            let _ = placed.send(spawn);
        }
    }

    /// New players stand on the ground of the spawn column once it exists.
    fn place_new_players(&mut self) {
        let Some(h) = self.map.height(SPAWN_XZ.0 as i32, SPAWN_XZ.1 as i32) else {
            return;
        };
        let waiting: Vec<u32> = self
            .players
            .iter()
            .filter(|(_, p)| p.placed.is_some())
            .map(|(&c, _)| c)
            .collect();
        for client in waiting {
            if let Some(mut p) = self.players.remove(&client) {
                p.body.pos.y = f64::from(h);
                self.answer_join(client, &mut p);
                self.players.insert(client, p);
            }
        }
    }

    /// Sends every ready column a client is missing.
    fn deliver_ready(&mut self, client: u32) {
        let Some(p) = self.players.get_mut(&client) else {
            return;
        };
        for pos in p.view.in_range() {
            if let Some(Slot::Ready(frame)) = self.slots.get(&pos)
                && p.view.wants(pos)
            {
                p.view.mark(pos);
                let _ = p.out.send(ToClient::Column(pos, frame.clone()));
            }
        }
    }

    fn tick(&mut self) {
        engine_core::zone!("server::tick");
        self.tick = self.tick.wrapping_add(1);
        // Results travel on the world channel; the pool's own completion
        // queue only holds the jobs' empty return values.
        for _ in self.pool.drain_completed() {}
        self.simulate_players();
        self.schedule();
        if self.tick % 20 == 0 {
            self.unload();
        }
    }

    /// Applies a finished job: the column goes into the map and out to the
    /// players who want it.
    fn finished(&mut self, out: JobOut) {
        match out {
            JobOut::Ready(column, frame) => {
                let pos = column.pos();
                if !matches!(self.slots.get(&pos), Some(Slot::Working(_))) {
                    return; // unloaded meanwhile
                }
                self.busy = self.busy.saturating_sub(1);
                let _ = self.map.insert_column(column);
                self.slots.insert(pos, Slot::Ready(frame.clone()));
                for p in self.players.values_mut() {
                    if p.view.wants(pos) {
                        p.view.mark(pos);
                        let _ = p.out.send(ToClient::Column(pos, frame.clone()));
                    }
                }
                self.place_new_players();
            }
            JobOut::Failed(pos) => {
                if matches!(self.slots.get(&pos), Some(Slot::Working(_))) {
                    self.busy = self.busy.saturating_sub(1);
                    self.slots.remove(&pos);
                }
            }
        }
    }

    /// Generates (`column == None`) or packs a loaded column on the pool.
    fn submit(&mut self, pos: ColumnPos, column: Option<Column>) {
        let token = CancelToken::new();
        let me = self.me.clone();
        let (registry, bounds) = (Arc::clone(&self.cfg.registry), self.cfg.bounds);
        let (pipeline, ctx) = (
            Arc::clone(&self.cfg.pipeline),
            Arc::clone(&self.cfg.context),
        );
        // The spawn area of a joining player is what everyone waits for.
        let near = self
            .players
            .values()
            .any(|p| chebyshev(pos, p.view.center()) <= 1);
        let class = if near {
            Priority::Visible
        } else {
            Priority::Prefetch
        };
        self.pool.submit(class, token.clone(), move |_| {
            let column = match column {
                Some(c) => Some(c),
                None => pipeline.generate(&ctx, pos).into_column(&registry),
            };
            let out = match column {
                None => JobOut::Failed(pos),
                Some(column) => match ColumnData::from_sections(pos, bounds, column.sections())
                    .ok()
                    .and_then(|d| d.encode_frame().ok())
                {
                    Some(frame) => JobOut::Ready(column, Bytes::from(frame)),
                    None => JobOut::Failed(pos),
                },
            };
            let _ = me.send(WorldMsg::Job(out));
        });
        self.slots.insert(pos, Slot::Working(token));
    }

    /// Asks for the nearest missing columns of every player.
    fn schedule(&mut self) {
        if self.busy >= self.max_busy || self.players.is_empty() {
            return;
        }
        let mut wanted: Vec<(i64, ColumnPos)> = Vec::new();
        for p in self.players.values() {
            for pos in p.view.in_range() {
                if !self.slots.contains_key(&pos) {
                    wanted.push((dist2(pos, p.view.center()), pos));
                }
            }
        }
        wanted.sort_unstable_by_key(|&(d, p)| (d, p.x, p.z));
        wanted.dedup_by_key(|&mut (_, p)| p);
        for (_, pos) in wanted {
            if self.busy >= self.max_busy {
                break;
            }
            if self.slots.contains_key(&pos) {
                continue; // wanted by two players at different distances
            }
            if self.storage.tx.send(StorageMsg::Load(pos)).is_err() {
                // No storage thread: generate.
                self.submit(pos, None);
            } else {
                self.slots.insert(pos, Slot::Loading);
            }
            self.busy += 1;
        }
    }

    fn simulate_players(&mut self) {
        let world = MapBlocks {
            map: &self.map,
            swim: &self.cfg.swim,
        };
        let mut moved: Vec<u32> = Vec::new();
        for (&client, p) in &mut self.players {
            if p.placed.is_some() {
                p.frames.clear();
                continue;
            }
            let c = column_of(p.body.pos);
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
            // Frames wait while the ground is still loading here (the client
            // only sends input when its own area is ready), then catch up a
            // few per tick.
            if ready {
                for _ in 0..MAX_FRAMES_PER_TICK {
                    let Some(f) = p.frames.pop_front() else {
                        break;
                    };
                    p.yaw = f.yaw.to_radians();
                    p.pitch = f.pitch.to_signed_radians();
                    step(&mut p.body, &input_of(&f), &self.cfg.physics, &world);
                }
            }
            let c = column_of(p.body.pos);
            if p.view.set_center(c) {
                let _ = p.out.send(ToClient::Center(c));
                moved.push(client);
            }
        }
        for client in moved {
            self.deliver_ready(client);
        }
    }

    /// Drops columns no player needs (edited ones would be saved first; edits
    /// arrive with T2-04).
    fn unload(&mut self) {
        let keep = self.cfg.view + UNLOAD_MARGIN;
        let centers: Vec<ColumnPos> = self.players.values().map(|p| p.view.center()).collect();
        let far = |pos: ColumnPos| centers.iter().all(|&c| chebyshev(pos, c) > keep);
        let gone: Vec<ColumnPos> = self.slots.keys().copied().filter(|&p| far(p)).collect();
        for pos in gone {
            match self.slots.remove(&pos) {
                Some(Slot::Working(token)) => {
                    token.cancel();
                    self.busy = self.busy.saturating_sub(1);
                }
                Some(Slot::Loading) => self.busy = self.busy.saturating_sub(1),
                _ => {}
            }
            self.map.remove_column(pos);
        }
    }

    fn save_player(&self, p: &Player) {
        if p.placed.is_some() {
            return; // never placed
        }
        let save = PlayerSave {
            pos: p.body.pos.to_array(),
            yaw: p.yaw,
            pitch: p.pitch,
            flying: p.body.flying,
        };
        let _ = self.storage.tx.send(StorageMsg::SavePlayer(p.id, save));
    }

    fn shutdown(&mut self) {
        for p in self.players.values() {
            self.save_player(p);
        }
        // No edits yet (T2-04), so no columns to save; the message keeps the
        // writer path exercised.
        let _ = self
            .storage
            .tx
            .send(StorageMsg::Save(Vec::<ColumnRecord>::new()));
        let (tx, rx) = std::sync::mpsc::channel();
        if self.storage.tx.send(StorageMsg::Flush(tx)).is_ok() {
            let _ = rx.recv_timeout(Duration::from_secs(10));
        }
        // Stop the storage thread: its channel closes with a dropped sender.
        let (dead, _) = std::sync::mpsc::channel();
        drop(std::mem::replace(&mut self.storage.tx, dead));
        if let Some(h) = self.storage.handle.take() {
            let _ = h.join();
        }
    }
}

/// The simulated input of a quantized frame (the client predicts with the
/// same dequantized values).
#[must_use]
pub fn input_of(f: &InputFrame) -> PlayerInput {
    PlayerInput {
        forward: axis_from_i8(f.forward),
        strafe: axis_from_i8(f.strafe),
        jump: f.buttons & buttons::JUMP != 0,
        sprint: f.buttons & buttons::SPRINT != 0,
        descend: f.buttons & buttons::DESCEND != 0,
        toggle_fly: f.buttons & buttons::TOGGLE_FLY != 0,
        fly_speed: if f.fly_speed == 0 {
            1.0
        } else {
            f64::from(f.fly_speed) / 16.0
        },
        yaw: f.yaw.to_radians(),
    }
}
