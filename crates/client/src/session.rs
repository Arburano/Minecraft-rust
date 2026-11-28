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
use world::storage::StorageError;
use world::{BlockStateId, Column, ColumnPos, ModSet, Registry, WorldBounds, WorldMap};
use worldgen::{GenContext, Pipeline, WorldgenConfig};

use auth::AdmissionPolicy;
use protocol::{Correction, InputDatagram, Intent};
use server::input::{apply_fly_steps, corrected, intent, physics_input, state_hash};
use server::{Server, ServerConfig};

use crate::edit::{BatchSync, Editor, Placer, dig, pick, place};
use crate::loading::{LoadConfig, Loader, Viewer};
use crate::others::{OtherEntity, Others};
use crate::remote::{ConnectOptions, Focus, Remote, RemoteError, RemoteEvent};

/// Columns around a headless player that are kept (physics needs them).
const HEADLESS_RADIUS: i32 = 2;
use crate::worlds::{WorldInfo, WorldsError, now};

/// How a session is set up.
#[derive(Clone, Debug)]
pub struct SessionConfig {
    /// The mods of the build (ADR-0017); the embedded server runs the same.
    pub mods: ModSet,
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
    /// No light, meshes or loader: columns near the player go straight into
    /// the map, the rest are dropped on arrival (headless bots, T2-05).
    pub headless: bool,
    /// Network runtime shared with other sessions (bots); `None`: a private one.
    pub runtime: Option<tokio::runtime::Handle>,
    /// The player's name in single-player (offline identity; its ID names
    /// the player file of the embedded server).
    pub player_name: String,
}

impl SessionConfig {
    /// Defaults for a build's mods.
    #[must_use]
    pub fn new(mods: ModSet) -> Self {
        Self {
            mods,
            radius: 12,
            threads: 3,
            bounds: WorldBounds::default(),
            autosave: Duration::from_secs(30),
            light_sections_per_frame: 2,
            headless: false,
            runtime: None,
            player_name: "Player".into(),
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

/// Where the last [`Session::update`] spent its time (T1-25 spike
/// attribution). Filled on every call; reading it costs nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UpdateCost {
    /// The whole call.
    pub total: Duration,
    /// Receiving from the server and the loader tick together.
    pub loader: Duration,
    /// Receiving from the server (part of `loader`): polling the link,
    /// applying datagrams, columns, block changes and edit answers, and
    /// reconciling corrections. (Before T2-08 this was reading saved columns.)
    pub source: Duration,
    /// Columns handed to the loader this call.
    pub source_calls: u32,
    /// Relighting edited sections.
    pub light: Duration,
    /// Edited-column bookkeeping.
    pub saves: Duration,
    /// Columns that left the map.
    pub unloaded: u32,
    /// The autosave request, if it ran in this call (zero otherwise; the
    /// embedded server saves on its own thread).
    pub autosave: Duration,
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
    /// Columns received.
    pub columns: u64,
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
    /// Headless: the columns to decode (around the player).
    focus: Option<Arc<Focus>>,
    /// Headless: columns received outside the focus, still compressed.
    packed: HashMap<ColumnPos, protocol::ColumnPayload>,
    /// The newest intent tick the server has applied.
    acked_tick: Option<u32>,
}

/// A running game: a client of a dedicated server, or of the server
/// embedded in this process for single-player (one code path, T2-08).
pub struct Session {
    registry: Arc<Registry>,
    info: WorldInfo,
    link: Box<Link>,
    /// Single-player: the embedded server and the world folder.
    embedded: Option<(Server, PathBuf)>,
    map: WorldMap,
    light: WorldLight,
    loader: Loader,
    editor: Editor,
    sync: BatchSync,
    pool: Option<JobPool>,
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
    cost: UpdateCost,
}

impl Session {
    /// Opens the world in `dir` (created with [`crate::worlds::create`]).
    pub fn open(dir: &Path, config: SessionConfig) -> Result<Self, SessionError> {
        let info = WorldInfo::load(dir)?;
        let name = config.player_name.clone();
        migrate_world(dir, &info, &name)?;
        let view = config.radius + 2;
        let mut sc = ServerConfig::new(
            "127.0.0.1:0"
                .parse()
                .map_err(|e| SessionError::Data(format!("{e}")))?,
            AdmissionPolicy {
                allow_offline: true,
            },
            dir.to_owned(),
            config.mods.clone(),
        );
        sc.seed = Some(info.seed);
        sc.view_distance = view;
        sc.sim_distance = view.min(server::DEFAULT_SIM_DISTANCE);
        sc.threads = config.threads.max(1);
        sc.region_threads = 2;
        sc.bounds = config.bounds;
        sc.lan_announce = false;
        sc.server_name = info.name.clone();
        let server = Server::start(sc).map_err(|e| SessionError::Data(e.to_string()))?;
        let addr = server.local_addr();
        let mut session = Self::connect(addr, &name, config)?;
        session.info = info;
        session.embedded = Some((server, dir.to_owned()));
        Ok(session)
    }

    /// Joins the server at `addr` as `name` (offline mode; development and
    /// private tests only, `docs/legal.md` rule 7).
    pub fn connect(
        addr: SocketAddr,
        name: &str,
        config: SessionConfig,
    ) -> Result<Self, SessionError> {
        let data = ModData::load(&config.mods)?;
        let focus = config
            .headless
            .then(|| Arc::new(Focus::new(HEADLESS_RADIUS)));
        let remote = Remote::connect_with(
            addr,
            name,
            Arc::clone(&data.registry),
            ConnectOptions {
                runtime: config.runtime.clone(),
                focus: focus.clone(),
            },
        )?;
        if let Some(f) = &focus {
            f.set(ColumnPos {
                x: (remote.spawn.position[0] / 16.0).floor() as i32,
                z: (remote.spawn.position[2] / 16.0).floor() as i32,
            });
        }
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
                fly_speed: spawn.fly_speed,
            },
        };
        // The server streams 2 columns beyond what the client renders (light
        // and meshing of the border need them). The settings keep the render
        // distance ≥ 8 (PRD §14).
        let radius = config.radius.min(i32::from(spawn.view_distance) - 2).max(1);
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
            focus,
            packed: HashMap::new(),
            acked_tick: None,
        };
        Self::build(
            data,
            info,
            Box::new(link),
            spawn.bounds(),
            radius,
            false,
            config,
        )
    }

    fn build(
        data: ModData,
        info: WorldInfo,
        link: Box<Link>,
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
        let pool = if config.headless {
            None
        } else {
            Some(
                JobPool::with_low_priority_workers(config.threads.max(1))
                    .map_err(|e| data_err(&e))?,
            )
        };
        let editor = if config.headless {
            Editor::headless(&map)
        } else {
            Editor::new(&map)
        };
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
            link,
            embedded: None,
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
            cost: UpdateCost::default(),
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

    /// Whether the player is at the edge of the loaded world: some column
    /// around where it will be in half a second is not here yet. Movement
    /// there is held (unloaded cells are solid, on the server too), so the
    /// game says why (T2-10).
    #[must_use]
    pub fn terrain_loading(&self) -> bool {
        if !self.is_playing() {
            return false;
        }
        let ahead = self.body.pos + self.body.vel * 10.0;
        let c = ColumnPos {
            x: (ahead.x / 16.0).floor() as i32,
            z: (ahead.z / 16.0).floor() as i32,
        };
        (-1..=1).any(|dx| {
            (-1..=1).any(|dz| {
                self.map
                    .column(ColumnPos {
                        x: c.x + dx,
                        z: c.z + dz,
                    })
                    .is_none()
            })
        })
    }

    /// Whether this session plays on a server.
    #[must_use]
    pub fn is_remote(&self) -> bool {
        self.embedded.is_none()
    }

    fn ready_around(&self, p: DVec3) -> bool {
        let c = ColumnPos {
            x: (p.x / 16.0).floor() as i32,
            z: (p.z / 16.0).floor() as i32,
        };
        (-1..=1).all(|dx| {
            (-1..=1).all(|dz| {
                let n = ColumnPos {
                    x: c.x + dx,
                    z: c.z + dz,
                };
                if self.config.headless {
                    self.map.column(n).is_some()
                } else {
                    self.loader.is_ready(&self.map, n)
                }
            })
        })
    }

    /// Edits one cell on behalf of the player (bots; no pick or reach check
    /// on this side — the server checks).
    pub fn edit_cell(&mut self, pos: BlockPos, state: Option<BlockStateId>) -> bool {
        self.edit_for_test(pos, state)
    }

    /// One frame: stream the world around the player, place a new player on
    /// the ground once the spawn column exists, relight a capped number of
    /// sections, keep edited columns that leave the map, autosave.
    pub fn update(&mut self) -> Result<(), SessionError> {
        let t0 = Instant::now();
        let mut cost = UpdateCost::default();
        let viewer = Viewer {
            pos: self.body.pos.to_array(),
            velocity: (self.body.vel * 20.0).to_array(),
        };
        {
            let link = &mut self.link;
            let now = Instant::now();
            let dt = now.duration_since(link.last_frame);
            link.last_frame = now;
            link.offset *= (-dt.as_secs_f64() / SMOOTHING.as_secs_f64()).exp();
            let mut corrections: Vec<Correction> = Vec::new();
            for e in link.remote.poll() {
                match e {
                    RemoteEvent::Datagram(d) => {
                        if let Some(t) = d.ack_tick {
                            link.acked_tick = Some(link.acked_tick.map_or(t, |a| a.max(t)));
                        }
                        link.others.apply(&d, now);
                        if let Some(c) = d.correction {
                            corrections.push(c);
                        }
                    }
                    RemoteEvent::Column(mut c) => {
                        link.stats.columns += 1;
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
                    RemoteEvent::Packed(pos, payload) => {
                        link.stats.columns += 1;
                        link.forgotten.remove(&pos);
                        link.packed.insert(pos, payload);
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
                        link.packed.remove(&c);
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
            cost.source = t0.elapsed();
            link.stats.offset = link.offset.length();
            link.stats.others = link.others.len();
            if let Some(focus) = &link.focus {
                // Headless: columns near the player go straight into the
                // map; no light, meshes or loader.
                let center = ColumnPos {
                    x: (self.body.pos.x / 16.0).floor() as i32,
                    z: (self.body.pos.z / 16.0).floor() as i32,
                };
                focus.set(center);
                let near = |c: ColumnPos| {
                    (c.x - center.x).abs() <= HEADLESS_RADIUS + 1
                        && (c.z - center.z).abs() <= HEADLESS_RADIUS + 1
                };
                for (_, col) in link.incoming.drain() {
                    if near(col.pos()) {
                        let _ = self.map.insert_column(col);
                    }
                }
                // Columns the player walked up to: decode them now.
                let wanted: Vec<ColumnPos> = link
                    .packed
                    .keys()
                    .copied()
                    .filter(|&c| near(c) && self.map.column(c).is_none())
                    .collect();
                for c in wanted {
                    if let Some(p) = link.packed.remove(&c)
                        && let Some(mut col) = link.remote.unpack(&p)
                    {
                        for (pos, id) in link.late.remove(&c).unwrap_or_default() {
                            col.set(
                                pos.x.rem_euclid(16) as usize,
                                pos.y,
                                pos.z.rem_euclid(16) as usize,
                                id,
                                &self.registry,
                            );
                        }
                        let _ = self.map.insert_column(col);
                    }
                }
                // Decoded columns stay until the server forgets them
                // (bounded by its view radius).
                let far: Vec<ColumnPos> = self
                    .map
                    .column_positions()
                    .filter(|&c| link.forgotten.contains(&c))
                    .collect();
                for c in far {
                    self.map.remove_column(c);
                    link.forgotten.remove(&c);
                }
            } else if let Some(pool) = &self.pool {
                let incoming = &mut link.incoming;
                let calls = &mut cost.source_calls;
                let mut source = |c: ColumnPos| {
                    let col = incoming.remove(&c);
                    *calls += u32::from(col.is_some());
                    col
                };
                self.loader.tick_with_source(
                    &viewer,
                    &mut self.map,
                    &mut self.light,
                    pool,
                    Some(&mut source),
                );
            }
        }
        let t1 = Instant::now();
        cost.loader = t1 - t0;
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
        let t2 = Instant::now();
        cost.light = t2 - t1;
        let gone = self.gone.len();
        let result = self.track_saves();
        cost.unloaded = (self.gone.len() - gone) as u32;
        let t3 = Instant::now();
        cost.saves = t3 - t2;
        let result = result.and_then(|()| {
            if self.last_save.elapsed() >= self.config.autosave && self.embedded.is_some() {
                self.save()?;
                cost.autosave = t3.elapsed();
            }
            Ok(())
        });
        cost.total = t0.elapsed();
        self.cost = cost;
        result
    }

    /// Where the last [`Session::update`] spent its time.
    #[must_use]
    pub fn last_update_cost(&self) -> &UpdateCost {
        &self.cost
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
        {
            let link = &mut self.link;
            // A column the server still keeps for us is kept for when the
            // player comes back; the server never resends it otherwise.
            for col in self.unloaded.drain(..) {
                self.gone.push(col.pos());
                if !link.forgotten.remove(&col.pos()) {
                    link.incoming.insert(col.pos(), col);
                }
            }
            self.unsaved.clear();
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
        {
            let link = &mut self.link;
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
        }
    }

    /// Whether the player is flying.
    #[must_use]
    pub fn is_flying(&self) -> bool {
        self.body.flying
    }

    /// The fly speed multiplier, including wheel notches not yet sent (the
    /// server applies them with the next tick's intent, the same way).
    #[must_use]
    pub fn fly_speed(&self) -> f64 {
        apply_fly_steps(
            self.fly_speed,
            self.link.fly_steps.clamp(-128, 127) as i8,
            self.physics.fly_speed_range,
        )
    }

    /// Sets the fly speed multiplier as close as whole wheel notches
    /// (×1.25 each) get, within the movement profile's range.
    pub fn set_fly_speed(&mut self, speed: f64) {
        let [lo, hi] = self.physics.fly_speed_range;
        let target = speed.clamp(lo, hi);
        let steps = (target / self.fly_speed()).ln() / 1.25f64.ln();
        self.adjust_fly_speed(steps.round() as i32);
    }

    /// Changes the fly speed by mouse-wheel notches (×1.25 per notch), within
    /// the movement profile's range. Applied at the next tick, on both sides.
    pub fn adjust_fly_speed(&mut self, notches: i32) {
        self.link.fly_steps = (self.link.fly_steps + notches).clamp(-128, 127);
    }

    /// Turns the view (radians); pitch is clamped to straight up/down.
    pub fn look(&mut self, dyaw: f64, dpitch: f64) {
        self.yaw = (self.yaw + dyaw).rem_euclid(std::f64::consts::TAU);
        self.pitch = (self.pitch + dpitch).clamp(-1.55, 1.55);
    }

    /// Other players on a server, as drawn now (empty in single-player).
    #[must_use]
    pub fn others(&self) -> Vec<OtherEntity> {
        self.link.others.at(Instant::now()).collect()
    }

    /// Reconciliation counters on a server.
    #[must_use]
    pub fn net_stats(&self) -> Option<NetStats> {
        let link = &self.link;
        Some(NetStats {
            bytes: link.remote.bytes(),
            pending_edits: link.pending.len(),
            ..link.stats
        })
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
        let offset = self.link.offset;
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
        let link = &mut self.link;
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
        let Some((_, dir)) = &self.embedded else {
            return Ok(()); // a dedicated server saves its world
        };
        // The embedded server saves blocks and the player (autosave, leave,
        // shutdown); `world.toml` keeps the menu's view of the world.
        if self.spawned {
            self.info.player.pos = Some(self.body.pos.to_array());
        }
        self.info.player.yaw = self.yaw;
        self.info.player.flying = self.body.flying;
        self.info.player.fly_speed = self.fly_speed();
        self.info.player.pitch = self.pitch;
        self.info.last_played = now();
        self.info.save(dir)?;
        self.last_save = Instant::now();
        Ok(())
    }

    /// Saves and closes: leaves the server; in single-player the embedded
    /// server then saves the world and stops.
    pub fn close(mut self) -> Result<(), SessionError> {
        // Let the server apply the intents and answer the edits already
        // sent (else the last steps or a block placed just before quitting
        // could be lost), at most a moment.
        let deadline = Instant::now() + Duration::from_secs(2);
        let last_sent = self.link.tick.checked_sub(1);
        while (!self.link.pending.is_empty()
            || last_sent.is_some_and(|t| self.link.acked_tick.is_none_or(|a| a < t)))
            && self.link.ended.is_none()
            && Instant::now() < deadline
        {
            if self.update().is_err() {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        self.save()?;
        let embedded = self.embedded.take();
        let Self { link, .. } = self;
        link.remote.leave();
        if let Some((server, _)) = embedded {
            server.shutdown();
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

/// Worlds saved before T2-08 keep the player in `world.toml`; the embedded
/// server reads `players/<player id>.toml`. Copy it over once, so a
/// single-player world opens where it was left (the seed is passed to the
/// server, which writes its own `server.toml`).
fn migrate_world(dir: &Path, info: &WorldInfo, name: &str) -> Result<(), SessionError> {
    let id = auth::Identity::offline(name)
        .map_err(|e| SessionError::Data(e.to_string()))?
        .id()
        .to_string();
    if server::players::load(dir, &id).is_some() {
        return Ok(());
    }
    if let Some(pos) = info.player.pos {
        server::players::save(
            dir,
            &id,
            &server::players::SavedPlayer {
                pos,
                yaw: info.player.yaw,
                pitch: info.player.pitch,
                flying: info.player.flying,
                fly_speed: info.player.fly_speed,
            },
        )
        .map_err(|e| SessionError::Data(format!("player file: {e}")))?;
    }
    Ok(())
}

/// The `core` mod's data a session needs.
struct ModData {
    registry: Arc<Registry>,
    gen_config: WorldgenConfig,
    physics: PhysicsConfig,
    swim: Vec<BlockStateId>,
}

impl ModData {
    fn load(mods: &ModSet) -> Result<Self, SessionError> {
        let data_err = |e: &dyn std::fmt::Display| SessionError::Data(e.to_string());
        let b = mods.registry_builder().map_err(|e| data_err(&e))?;
        let registry = Arc::new(b.build().map_err(|e| data_err(&e))?);
        let src = mods
            .read_slot(
                &world::modset::worldgen_slot("overworld"),
                "worldgen/overworld.toml",
            )
            .map_err(|e| data_err(&e))?;
        let gen_config = WorldgenConfig::from_toml(&src, &registry).map_err(|e| data_err(&e))?;
        let physics_src = mods
            .read_slot(world::modset::MOVEMENT_SLOT, "movement/profile.toml")
            .map_err(|e| data_err(&e))?;
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
