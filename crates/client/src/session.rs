//! A game session: the world, the player, edits and saving. Single-player
//! generates and saves locally; on a server (T2-02) columns come from the
//! network and the player's intents go to the server, which runs the same
//! physics. No window or GPU here, so the whole play loop runs in tests; the
//! app feeds input and draws what this reports.
//!
//! Timing: [`Session::update`] once per frame (loading, relighting, saving),
//! [`Session::fixed_tick`] at 20 TPS (physics).

use std::collections::{HashMap, HashSet, VecDeque};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use engine_core::jobs::JobPool;
use glam::{DVec3, IVec3};
use light::{SectionLight, WorldLight};
use mesh::PackedBatch;
use sim::physics::{MapBlocks, PhysicsConfig, PlayerBody, PlayerInput, interpolate, step};
use sim::raycast::RayHit;
use world::coords::{BlockPos, SectionPos};
use world::map::DirtyKind;
use world::registry::blocks_from_dir;
use world::storage::{ColumnRecord, StorageError, WorldStore};
use world::{BlockStateId, Column, ColumnPos, Registry, RegistryBuilder, WorldBounds, WorldMap};
use worldgen::{GenContext, Pipeline, WorldgenConfig};

use protocol::{Correction, InputDatagram, Intent};
use server::input::{apply_fly_steps, corrected, intent, physics_input, state_hash};

use crate::edit::{BatchSync, Editor, Placer, dig, pick, place};
use crate::loading::{LoadConfig, Loader, Viewer};
use crate::others::{OtherEntity, Others};
use crate::remote::{Remote, RemoteError, RemoteEvent};
use crate::worlds::{WorldInfo, WorldsError, now};

/// How a session is set up.
#[derive(Clone, Debug)]
pub struct SessionConfig {
    /// The `core` mod's data folder (`mods/core/data`).
    pub data: PathBuf,
    /// Full-data render distance in chunks.
    pub radius: i32,
    /// Worker threads.
    pub threads: usize,
    /// Vertical bounds of the dimension.
    pub bounds: WorldBounds,
    /// Autosave interval.
    pub autosave: Duration,
    /// Sections relit per frame at most (deferred light).
    pub light_sections_per_frame: usize,
}

impl SessionConfig {
    /// Defaults for `data`.
    #[must_use]
    pub fn new(data: PathBuf) -> Self {
        Self {
            data,
            radius: 12,
            threads: 3,
            bounds: WorldBounds::default(),
            autosave: Duration::from_secs(30),
            light_sections_per_frame: 2,
        }
    }
}

/// Session errors.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    /// Mod data could not be read.
    #[error("mod data: {0}")]
    Data(String),
    /// The save could not be read or written.
    #[error("save: {0}")]
    Storage(#[from] StorageError),
    /// `world.toml`.
    #[error(transparent)]
    World(#[from] WorldsError),
    /// Joining a server failed.
    #[error("{0}")]
    Join(#[from] RemoteError),
    /// The server ended the session.
    #[error("{0}")]
    Disconnected(String),
}

/// Intents repeated in each input datagram against loss.
const INTENT_COPIES: usize = 3;
/// Predicted ticks kept for replay after a correction (6.4 s).
const PREDICTION_HISTORY: usize = 128;
/// A correction moving the player farther than this (blocks) is applied at
/// once (a teleport); smaller ones are smoothed out.
const SNAP_DISTANCE: f64 = 2.0;
/// Time constant of the correction smoothing.
const SMOOTHING: Duration = Duration::from_millis(100);
/// Block changes buffered for a column that has not arrived yet.
const LATE_CHANGES_PER_COLUMN: usize = 4096;

/// One predicted tick: the intent sent and the state after it.
struct Predicted {
    intent: Intent,
    after: PlayerBody,
    fly_after: f64,
}

/// Reconciliation counters (tests, debug overlay).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NetStats {
    /// Corrections that changed the predicted state.
    pub corrections: u64,
    /// The largest correction, in blocks (before smoothing).
    pub max_correction: f64,
    /// The visual offset still being smoothed out, in blocks.
    pub offset: f64,
    /// Other entities known.
    pub others: usize,
    /// UDP payload bytes received and sent so far.
    pub bytes: (u64, u64),
    /// Own edits the server refused (rolled back).
    pub rejected_edits: u64,
    /// Own edits still waiting for the server's answer.
    pub pending_edits: usize,
}

/// A server session's state.
struct Link {
    remote: Remote,
    /// Columns received and not yet taken by the loader (or unloaded by it
    /// while the server still keeps them for us).
    incoming: HashMap<ColumnPos, Column>,
    /// Columns the server forgot while they were in the map: dropped when
    /// the loader unloads them.
    forgotten: HashSet<ColumnPos>,
    tick: u32,
    recent: VecDeque<Intent>,
    fly_steps: i32,
    ended: Option<String>,
    history: VecDeque<Predicted>,
    last_correction: Option<u32>,
    /// Drawn position minus simulated position, decaying to zero.
    offset: DVec3,
    last_frame: Instant,
    others: Others,
    stats: NetStats,
    next_seq: u32,
    /// Our edits waiting for the server's answer.
    pending: HashMap<u32, BlockPos>,
    /// For cells with pending edits: the server's state and how many of our
    /// edits there are unanswered.
    shadow: HashMap<BlockPos, (BlockStateId, u32)>,
    /// Changes for columns that have not arrived yet.
    late: HashMap<ColumnPos, Vec<(BlockPos, BlockStateId)>>,
}

/// Where the world comes from.
enum Backend {
    /// Single-player: generate and save locally.
    Local { store: WorldStore, dir: PathBuf },
    /// A server.
    Remote(Box<Link>),
}

/// A running single-player world.
pub struct Session {
    registry: Arc<Registry>,
    info: WorldInfo,
    backend: Backend,
    map: WorldMap,
    light: WorldLight,
    loader: Loader,
    editor: Editor,
    sync: BatchSync,
    pool: JobPool,
    physics: PhysicsConfig,
    swim: Vec<BlockStateId>,
    body: PlayerBody,
    prev: PlayerBody,
    yaw: f64,
    pitch: f64,
    fly_speed: f64,
    spawned: bool,
    unsaved: HashSet<ColumnPos>,
    dirty: Vec<SectionPos>,
    unloaded: Vec<Column>,
    /// Columns unloaded since the last `render_updates` (their GPU light goes).
    gone: Vec<ColumnPos>,
    light_changed: Vec<SectionPos>,
    last_save: Instant,
    config: SessionConfig,
}

impl Session {
    /// Opens the world in `dir` (created with [`crate::worlds::create`]).
    pub fn open(dir: &Path, config: SessionConfig) -> Result<Self, SessionError> {
        let data = ModData::load(&config.data)?;
        let info = WorldInfo::load(dir)?;
        let store = WorldStore::open(dir, &data.registry, config.bounds)?;
        let bounds = config.bounds;
        let radius = config.radius;
        Self::build(
            data,
            info,
            Backend::Local {
                store,
                dir: dir.to_owned(),
            },
            bounds,
            radius,
            true,
            config,
        )
    }

    /// Joins the server at `addr` as `name` (offline mode; development and
    /// private tests only, `docs/legal.md` rule 7).
    pub fn connect(
        addr: SocketAddr,
        name: &str,
        config: SessionConfig,
    ) -> Result<Self, SessionError> {
        let data = ModData::load(&config.data)?;
        let remote = Remote::connect(addr, name, Arc::clone(&data.registry))?;
        let spawn = remote.spawn.clone();
        let info = WorldInfo {
            name: remote.welcome.server_name.clone(),
            seed: 0,
            generator: "server".into(),
            created: 0,
            last_played: now(),
            player: crate::worlds::PlayerSave {
                pos: Some(spawn.position),
                yaw: f64::from(spawn.yaw),
                pitch: f64::from(spawn.pitch),
                flying: spawn.flying,
                fly_speed: 1.0,
            },
        };
        // The server streams 2 columns beyond what the client renders (light
        // and meshing of the border need them); never render below 8.
        let radius = config.radius.min(i32::from(spawn.view_distance) - 2).max(8);
        let link = Link {
            remote,
            incoming: HashMap::new(),
            forgotten: HashSet::new(),
            tick: 0,
            recent: VecDeque::new(),
            fly_steps: 0,
            ended: None,
            history: VecDeque::new(),
            last_correction: None,
            offset: DVec3::ZERO,
            last_frame: Instant::now(),
            others: Others::default(),
            stats: NetStats::default(),
            next_seq: 1,
            pending: HashMap::new(),
            shadow: HashMap::new(),
            late: HashMap::new(),
        };
        Self::build(
            data,
            info,
            Backend::Remote(Box::new(link)),
            spawn.bounds(),
            radius,
            false,
            config,
        )
    }

    fn build(
        data: ModData,
        info: WorldInfo,
        backend: Backend,
        bounds: WorldBounds,
        radius: i32,
        generate: bool,
        config: SessionConfig,
    ) -> Result<Self, SessionError> {
        let data_err = |e: &dyn std::fmt::Display| SessionError::Data(e.to_string());
        let ModData {
            registry,
            gen_config,
            physics,
            swim,
        } = data;
        let ctx = Arc::new(GenContext::new(info.seed, gen_config, bounds));
        let mut loader = Loader::new(
            LoadConfig {
                radius,
                generate,
                ..LoadConfig::default()
            },
            Arc::clone(&registry),
            Arc::new(Pipeline::overworld()),
            ctx,
        );
        loader.track_mesh_changes();
        loader.track_unloads();
        let map = WorldMap::new(Arc::clone(&registry), bounds);
        let mut light = WorldLight::new();
        light.track_changes();
        let pool =
            JobPool::with_low_priority_workers(config.threads.max(1)).map_err(|e| data_err(&e))?;
        let editor = Editor::new(&map);
        let (spawned, body) = match info.player.pos {
            Some(p) => (
                true,
                PlayerBody {
                    pos: DVec3::from_array(p),
                    flying: info.player.flying,
                    ..PlayerBody::default()
                },
            ),
            // The spawn height is known once the spawn column is generated.
            None => (
                false,
                PlayerBody {
                    pos: DVec3::new(8.5, 100.0, 8.5),
                    ..PlayerBody::default()
                },
            ),
        };
        Ok(Self {
            registry,
            backend,
            yaw: info.player.yaw,
            fly_speed: if info.player.fly_speed > 0.0 {
                info.player.fly_speed
            } else {
                1.0
            },
            pitch: info.player.pitch,
            info,
            map,
            light,
            loader,
            editor,
            sync: BatchSync::new(),
            pool,
            physics,
            swim,
            body,
            prev: body,
            spawned,
            unsaved: HashSet::new(),
            dirty: Vec::new(),
            unloaded: Vec::new(),
            gone: Vec::new(),
            light_changed: Vec::new(),
            last_save: Instant::now(),
            config,
        })
    }

    /// The block registry.
    #[must_use]
    pub fn registry(&self) -> &Arc<Registry> {
        &self.registry
    }

    /// The world map.
    #[must_use]
    pub fn map(&self) -> &WorldMap {
        &self.map
    }

    /// The loader (meshes, visibility, readiness).
    #[must_use]
    pub fn loader(&self) -> &Loader {
        &self.loader
    }

    /// World metadata.
    #[must_use]
    pub fn info(&self) -> &WorldInfo {
        &self.info
    }

    /// Movement config from the datapack.
    #[must_use]
    pub fn physics(&self) -> &PhysicsConfig {
        &self.physics
    }

    /// The player's body after the last tick.
    #[must_use]
    pub fn player(&self) -> &PlayerBody {
        &self.body
    }

    /// Whether the player has a position in a loaded world and moves.
    #[must_use]
    pub fn is_playing(&self) -> bool {
        self.spawned && self.ready_around(self.body.pos)
    }

    /// Whether this session plays on a server.
    #[must_use]
    pub fn is_remote(&self) -> bool {
        matches!(self.backend, Backend::Remote(_))
    }

    fn ready_around(&self, p: DVec3) -> bool {
        let c = ColumnPos {
            x: (p.x / 16.0).floor() as i32,
            z: (p.z / 16.0).floor() as i32,
        };
        (-1..=1).all(|dx| {
            (-1..=1).all(|dz| {
                self.loader.is_ready(
                    &self.map,
                    ColumnPos {
                        x: c.x + dx,
                        z: c.z + dz,
                    },
                )
            })
        })
    }

    /// One frame: stream the world around the player, place a new player on
    /// the ground once the spawn column exists, relight a capped number of
    /// sections, keep edited columns that leave the map, autosave.
    pub fn update(&mut self) -> Result<(), SessionError> {
        let viewer = Viewer {
            pos: self.body.pos.to_array(),
            velocity: (self.body.vel * 20.0).to_array(),
        };
        match &mut self.backend {
            Backend::Local { store, .. } => {
                let registry = &self.registry;
                let mut source = |c: ColumnPos| {
                    store
                        .load_column(c, registry)
                        .ok()
                        .flatten()
                        .map(|(col, _)| col)
                };
                self.loader.tick_with_source(
                    &viewer,
                    &mut self.map,
                    &mut self.light,
                    &self.pool,
                    Some(&mut source),
                );
            }
            Backend::Remote(link) => {
                let now = Instant::now();
                let dt = now.duration_since(link.last_frame);
                link.last_frame = now;
                link.offset *= (-dt.as_secs_f64() / SMOOTHING.as_secs_f64()).exp();
                let mut corrections: Vec<Correction> = Vec::new();
                for e in link.remote.poll() {
                    match e {
                        RemoteEvent::Datagram(d) => {
                            link.others.apply(&d, now);
                            if let Some(c) = d.correction {
                                corrections.push(c);
                            }
                        }
                        RemoteEvent::Column(mut c) => {
                            // Changes that arrived before their column.
                            for (pos, id) in link.late.remove(&c.pos()).unwrap_or_default() {
                                c.set(
                                    pos.x.rem_euclid(16) as usize,
                                    pos.y,
                                    pos.z.rem_euclid(16) as usize,
                                    id,
                                    &self.registry,
                                );
                            }
                            link.forgotten.remove(&c.pos());
                            link.incoming.insert(c.pos(), c);
                        }
                        RemoteEvent::Blocks(changes) => {
                            for (pos, id) in changes {
                                if let Some(truth) = link.shadow.get_mut(&pos) {
                                    // Our own edit is pending here: remember the
                                    // server's state for when it resolves.
                                    truth.0 = id;
                                    continue;
                                }
                                let c = ColumnPos::of_block(pos);
                                if self.map.column(c).is_some() {
                                    if self.map.get_block(pos) != Some(id) {
                                        let _ = self.editor.set_block(
                                            &mut self.map,
                                            &mut self.loader,
                                            pos,
                                            id,
                                        );
                                    }
                                } else if let Some(col) = link.incoming.get_mut(&c) {
                                    col.set(
                                        pos.x.rem_euclid(16) as usize,
                                        pos.y,
                                        pos.z.rem_euclid(16) as usize,
                                        id,
                                        &self.registry,
                                    );
                                } else {
                                    let list = link.late.entry(c).or_default();
                                    if list.len() < LATE_CHANGES_PER_COLUMN {
                                        list.push((pos, id));
                                    }
                                }
                            }
                        }
                        RemoteEvent::EditAck { seq, accepted } => {
                            let Some(pos) = link.pending.remove(&seq) else {
                                continue;
                            };
                            if !accepted {
                                link.stats.rejected_edits += 1;
                            }
                            let resolved = match link.shadow.get_mut(&pos) {
                                Some(t) => {
                                    t.1 = t.1.saturating_sub(1);
                                    (t.1 == 0).then_some(t.0)
                                }
                                None => None,
                            };
                            if let Some(truth) = resolved {
                                link.shadow.remove(&pos);
                                // The server's state wins: this keeps an accepted
                                // edit and rolls back a refused one.
                                if self.map.get_block(pos).is_some_and(|b| b != truth) {
                                    let _ = self.editor.set_block(
                                        &mut self.map,
                                        &mut self.loader,
                                        pos,
                                        truth,
                                    );
                                }
                            }
                        }
                        RemoteEvent::Forget(c) => {
                            link.late.remove(&c);
                            link.incoming.remove(&c);
                            if self.map.column(c).is_some() {
                                link.forgotten.insert(c);
                            }
                        }
                        RemoteEvent::Ended(m) => link.ended = Some(m),
                    }
                }
                if let Some(m) = &link.ended {
                    return Err(SessionError::Disconnected(m.clone()));
                }
                for c in corrections {
                    reconcile(
                        link,
                        &c,
                        &mut self.body,
                        &mut self.prev,
                        &mut self.fly_speed,
                        &self.physics,
                        &MapBlocks {
                            map: &self.map,
                            swim: &self.swim,
                        },
                    );
                }
                link.stats.offset = link.offset.length();
                link.stats.others = link.others.len();
                let incoming = &mut link.incoming;
                let mut source = |c: ColumnPos| incoming.remove(&c);
                self.loader.tick_with_source(
                    &viewer,
                    &mut self.map,
                    &mut self.light,
                    &self.pool,
                    Some(&mut source),
                );
            }
        }
        if !self.spawned
            && let Some(h) = self
                .map
                .height(self.body.pos.x as i32, self.body.pos.z as i32)
        {
            self.body.pos.y = f64::from(h);
            self.prev = self.body;
            self.spawned = true;
        }
        self.editor
            .process_light(
                &self.map,
                &mut self.light,
                self.config.light_sections_per_frame,
            )
            .ok();
        self.track_saves()?;
        if self.last_save.elapsed() >= self.config.autosave && !self.is_remote() {
            self.save()?;
        }
        Ok(())
    }

    /// Notes edited columns and writes the ones that were unloaded.
    fn track_saves(&mut self) -> Result<(), SessionError> {
        self.dirty.clear();
        self.map.take_dirty(DirtyKind::Save, &mut self.dirty);
        for s in &self.dirty {
            self.unsaved.insert(ColumnPos::of_section(*s));
        }
        self.unloaded.clear();
        self.loader.take_unloaded(&mut self.unloaded);
        if let Backend::Remote(link) = &mut self.backend {
            // A column the server still keeps for us is kept for when the
            // player comes back; the server never resends it otherwise.
            for col in self.unloaded.drain(..) {
                self.gone.push(col.pos());
                if !link.forgotten.remove(&col.pos()) {
                    link.incoming.insert(col.pos(), col);
                }
            }
            self.unsaved.clear();
            return Ok(());
        }
        let mut records = Vec::new();
        for col in &self.unloaded {
            self.gone.push(col.pos());
            if self.unsaved.remove(&col.pos()) {
                records.push(ColumnRecord {
                    pos: col.pos(),
                    bounds: col.bounds(),
                    sections: col.sections().to_vec(),
                    light: Vec::new(),
                    entities: Vec::new(),
                });
            }
        }
        if !records.is_empty()
            && let Backend::Local { store, .. } = &mut self.backend
        {
            store.save_all(&records)?;
        }
        Ok(())
    }

    /// One 20 TPS step of the player. The player stays put until the
    /// columns around it are ready (no falling through unloaded ground).
    pub fn fixed_tick(&mut self, input: &PlayerInput) {
        self.prev = self.body;
        if !self.is_playing() {
            return;
        }
        let world = MapBlocks {
            map: &self.map,
            swim: &self.swim,
        };
        if let Backend::Remote(link) = &mut self.backend {
            // Simulate exactly what the server will: the intent as it goes
            // on the wire (quantized yaw, wheel steps applied at the tick).
            let mut i = intent(link.tick, input, self.yaw, self.pitch, link.fly_steps);
            link.fly_steps = 0;
            self.fly_speed = apply_fly_steps(
                self.fly_speed,
                i.fly_speed_steps,
                self.physics.fly_speed_range,
            );
            step(
                &mut self.body,
                &physics_input(&i, self.fly_speed),
                &self.physics,
                &world,
            );
            i.state_hash = state_hash(&self.body, self.fly_speed);
            link.history.push_back(Predicted {
                intent: i,
                after: self.body,
                fly_after: self.fly_speed,
            });
            while link.history.len() > PREDICTION_HISTORY {
                link.history.pop_front();
            }
            link.recent.push_back(i);
            while link.recent.len() > INTENT_COPIES {
                link.recent.pop_front();
            }
            link.remote.send_input(InputDatagram {
                ack_snapshot: link.others.latest(),
                intents: link.recent.iter().copied().collect(),
            });
            link.tick = link.tick.wrapping_add(1);
            return;
        }
        let input = PlayerInput {
            yaw: self.yaw,
            fly_speed: self.fly_speed,
            ..*input
        };
        step(&mut self.body, &input, &self.physics, &world);
    }

    /// Whether the player is flying.
    #[must_use]
    pub fn is_flying(&self) -> bool {
        self.body.flying
    }

    /// The fly speed multiplier.
    #[must_use]
    pub fn fly_speed(&self) -> f64 {
        self.fly_speed
    }

    /// Sets the fly speed multiplier (clamped to the profile's range).
    pub fn set_fly_speed(&mut self, speed: f64) {
        if self.is_remote() {
            return; // the server would not know; use the wheel
        }
        let [lo, hi] = self.physics.fly_speed_range;
        self.fly_speed = speed.clamp(lo, hi);
    }

    /// Changes the fly speed by mouse-wheel notches (×1.25 per notch), within
    /// the movement profile's range.
    pub fn adjust_fly_speed(&mut self, notches: i32) {
        if let Backend::Remote(link) = &mut self.backend {
            // Applied at the next tick, on both sides.
            link.fly_steps += notches;
            return;
        }
        let [lo, hi] = self.physics.fly_speed_range;
        self.fly_speed = (self.fly_speed * 1.25f64.powi(notches)).clamp(lo, hi);
    }

    /// Turns the view (radians); pitch is clamped to straight up/down.
    pub fn look(&mut self, dyaw: f64, dpitch: f64) {
        self.yaw = (self.yaw + dyaw).rem_euclid(std::f64::consts::TAU);
        self.pitch = (self.pitch + dpitch).clamp(-1.55, 1.55);
    }

    /// Other players on a server, as drawn now (empty in single-player).
    #[must_use]
    pub fn others(&self) -> Vec<OtherEntity> {
        match &self.backend {
            Backend::Remote(link) => link.others.at(Instant::now()).collect(),
            Backend::Local { .. } => Vec::new(),
        }
    }

    /// Reconciliation counters on a server.
    #[must_use]
    pub fn net_stats(&self) -> Option<NetStats> {
        match &self.backend {
            Backend::Remote(link) => Some(NetStats {
                bytes: link.remote.bytes(),
                pending_edits: link.pending.len(),
                ..link.stats
            }),
            Backend::Local { .. } => None,
        }
    }

    /// Moves the predicted player without telling the server, to test
    /// reconciliation (a forced divergence).
    #[doc(hidden)]
    pub fn nudge_for_test(&mut self, by: DVec3) {
        self.body.pos += by;
    }

    /// Eye position interpolated `alpha` of the way into the current tick,
    /// and the view direction. On a server, corrections are smoothed out.
    #[must_use]
    pub fn camera(&self, alpha: f64) -> (DVec3, DVec3) {
        let offset = match &self.backend {
            Backend::Remote(link) => link.offset,
            Backend::Local { .. } => DVec3::ZERO,
        };
        let feet = interpolate(&self.prev, &self.body, alpha) + offset;
        let eye = feet + DVec3::new(0.0, self.physics.eye_height, 0.0);
        (eye, self.view_dir())
    }

    /// Unit view direction (yaw 0 looks towards −Z).
    #[must_use]
    pub fn view_dir(&self) -> DVec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        DVec3::new(-sy * cp, sp, -cy * cp)
    }

    /// The block under the crosshair.
    #[must_use]
    pub fn target(&self) -> Option<RayHit> {
        if !self.is_playing() {
            return None;
        }
        pick(
            &self.map,
            self.body.eye(&self.physics),
            self.view_dir(),
            self.physics.reach,
            &self.swim,
        )
    }

    /// Breaks the targeted block.
    pub fn dig(&mut self) -> bool {
        let Some(hit) = self.target() else {
            return false;
        };
        let old = self.map.get_block(hit.block);
        let done = dig(&mut self.editor, &mut self.map, &mut self.loader, &hit).is_ok();
        if done && let Some(old) = old {
            self.predicted(hit.block, old, None);
        }
        done
    }

    /// Sends a predicted edit to the server (single-player: nothing to do).
    /// `state` is `None` for digging.
    fn predicted(&mut self, pos: BlockPos, old: BlockStateId, state: Option<BlockStateId>) {
        let Backend::Remote(link) = &mut self.backend else {
            return;
        };
        let event = match state {
            None => Some(protocol::ClientEvent::Dig {
                seq: link.next_seq,
                pos: pos.into(),
            }),
            Some(id) => link
                .remote
                .server_state(id)
                .map(|state| protocol::ClientEvent::Place {
                    seq: link.next_seq,
                    pos: pos.into(),
                    state,
                }),
        };
        let Some(event) = event else {
            // The server does not know this block: undo at once.
            let _ = self
                .editor
                .set_block(&mut self.map, &mut self.loader, pos, old);
            return;
        };
        link.pending.insert(link.next_seq, pos);
        link.shadow.entry(pos).or_insert((old, 0)).1 += 1;
        link.next_seq = link.next_seq.wrapping_add(1).max(1);
        link.remote.send_event(event);
    }

    /// Edits one cell as if the player had picked it (no reach or pick
    /// check on this side: tests use it to make the server refuse an edit).
    #[doc(hidden)]
    pub fn edit_for_test(&mut self, pos: BlockPos, state: Option<BlockStateId>) -> bool {
        let Some(old) = self.map.get_block(pos) else {
            return false;
        };
        let id = state.unwrap_or(BlockStateId::AIR);
        if self
            .editor
            .set_block(&mut self.map, &mut self.loader, pos, id)
            .is_err()
        {
            return false;
        }
        self.predicted(pos, old, state);
        true
    }

    /// Light of the world (tests compare it with a fresh lighting).
    #[doc(hidden)]
    #[must_use]
    pub fn light_for_test(&self) -> &WorldLight {
        &self.light
    }

    /// Edits whose relighting is still queued.
    #[must_use]
    pub fn pending_light(&self) -> usize {
        self.editor.pending_light()
    }

    /// Places `id` against the targeted face.
    pub fn place(&mut self, id: BlockStateId) -> bool {
        let Some(hit) = self.target() else {
            return false;
        };
        let at = hit.place_pos();
        let old = self.map.get_block(at);
        let placer = Placer {
            body: &self.body,
            cfg: &self.physics,
            swim: &self.swim,
        };
        let done = matches!(
            place(
                &mut self.editor,
                &mut self.map,
                &mut self.loader,
                &hit,
                id,
                placer
            ),
            Ok(Some(_))
        );
        if done && let Some(old) = old {
            self.predicted(at, old, Some(id));
        }
        done
    }

    /// Hands the renderer what changed since the last call, in this order:
    /// sections of unloaded columns (their light must be dropped, else the
    /// GPU light atlas fills up and stale light shows through), re-packed
    /// render batches (an empty batch means "remove") and sections with new
    /// light.
    pub fn render_updates(
        &mut self,
        mut removed_section: impl FnMut(SectionPos),
        mut batch: impl FnMut(IVec3, &PackedBatch),
        mut section_light: impl FnMut(SectionPos, &SectionLight),
    ) {
        let b = self.map.bounds();
        for c in self.gone.drain(..) {
            // A column loaded again in the meantime keeps its light.
            if self.map.column(c).is_some() {
                continue;
            }
            for i in 0..b.section_count as i32 {
                removed_section(SectionPos {
                    x: c.x,
                    y: b.min_section_y + i,
                    z: c.z,
                });
            }
        }
        self.sync.collect(&mut self.loader);
        self.sync.upload(&mut batch);
        self.light_changed.clear();
        self.light.take_changed(&mut self.light_changed);
        for &sp in &self.light_changed {
            if let Some(l) = self.light.get(sp) {
                section_light(sp, l);
            }
        }
    }

    /// Writes edited columns that are still loaded and `world.toml`.
    pub fn save(&mut self) -> Result<(), SessionError> {
        self.track_saves()?;
        let Backend::Local { store, dir } = &mut self.backend else {
            return Ok(()); // the server saves its world
        };
        let records: Vec<ColumnRecord> = self
            .unsaved
            .iter()
            .filter_map(|&c| self.map.column(c))
            .map(|col| ColumnRecord {
                pos: col.pos(),
                bounds: col.bounds(),
                sections: col.sections().to_vec(),
                light: Vec::new(),
                entities: Vec::new(),
            })
            .collect();
        if !records.is_empty() {
            store.save_all(&records)?;
        }
        self.unsaved.clear();
        if self.spawned {
            self.info.player.pos = Some(self.body.pos.to_array());
        }
        self.info.player.yaw = self.yaw;
        self.info.player.flying = self.body.flying;
        self.info.player.fly_speed = self.fly_speed;
        self.info.player.pitch = self.pitch;
        self.info.last_played = now();
        self.info.save(dir)?;
        self.last_save = Instant::now();
        Ok(())
    }

    /// Saves and closes (on a server: leaves it).
    pub fn close(mut self) -> Result<(), SessionError> {
        self.save()?;
        if let Backend::Remote(link) = self.backend {
            link.remote.leave();
        }
        Ok(())
    }
}

/// Applies a server correction: back to the server's state after
/// `c.tick`, then the intents predicted since are replayed. The change in the
/// drawn position is smoothed out unless it is a teleport.
fn reconcile(
    link: &mut Link,
    c: &Correction,
    body: &mut PlayerBody,
    prev: &mut PlayerBody,
    fly_speed: &mut f64,
    physics: &PhysicsConfig,
    world: &MapBlocks<'_>,
) {
    if link.last_correction.is_some_and(|t| c.tick <= t) {
        return; // a repeat of one already applied
    }
    let (mut b, mut fly) = corrected(c);
    let at = link.history.iter().position(|p| p.intent.tick == c.tick);
    if let Some(i) = at
        && state_hash(&link.history[i].after, link.history[i].fly_after) == state_hash(&b, fly)
    {
        return; // the prediction already agrees (a repeated correction)
    }
    link.last_correction = Some(c.tick);
    let before = body.pos;
    let mut last_prev = b;
    if let Some(i) = at {
        link.history[i].after = b;
        link.history[i].fly_after = fly;
        for p in link.history.iter_mut().skip(i + 1) {
            last_prev = b;
            fly = apply_fly_steps(fly, p.intent.fly_speed_steps, physics.fly_speed_range);
            step(&mut b, &physics_input(&p.intent, fly), physics, world);
            p.after = b;
            p.fly_after = fly;
        }
    }
    let jump = before - b.pos;
    link.stats.corrections += 1;
    link.stats.max_correction = link.stats.max_correction.max(jump.length());
    link.offset += jump;
    if link.offset.length() > SNAP_DISTANCE {
        link.offset = DVec3::ZERO;
    }
    *body = b;
    *prev = last_prev;
    *fly_speed = fly;
}

/// The `core` mod's data a session needs.
struct ModData {
    registry: Arc<Registry>,
    gen_config: WorldgenConfig,
    physics: PhysicsConfig,
    swim: Vec<BlockStateId>,
}

impl ModData {
    fn load(dir: &Path) -> Result<Self, SessionError> {
        let data_err = |e: &dyn std::fmt::Display| SessionError::Data(e.to_string());
        let mut b = RegistryBuilder::new();
        b.add_mod(
            "core",
            blocks_from_dir(&dir.join("blocks")).map_err(|e| data_err(&e))?,
        );
        let registry = Arc::new(b.build().map_err(|e| data_err(&e))?);
        let src = std::fs::read_to_string(dir.join("worldgen/overworld.toml"))
            .map_err(|e| data_err(&e))?;
        let gen_config = WorldgenConfig::from_toml(&src, &registry).map_err(|e| data_err(&e))?;
        let physics_src =
            std::fs::read_to_string(dir.join("player/physics.toml")).map_err(|e| data_err(&e))?;
        let physics = PhysicsConfig::from_toml(&physics_src).map_err(|e| data_err(&e))?;
        let swim = physics.swim_ids(&registry).map_err(|e| data_err(&e))?;
        Ok(Self {
            registry,
            gen_config,
            physics,
            swim,
        })
    }
}
