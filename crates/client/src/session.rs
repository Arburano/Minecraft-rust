//! A game session: the world, the player, edits and saving. The world comes
//! either from a single-player save (generated or loaded, [`Session::open`])
//! or from a server ([`Session::connect`], T2-02: columns stream in over the
//! network; light and meshes are still made here, ADR-0011). No window or GPU
//! here, so the whole play loop runs in tests; the app feeds input and draws
//! what this reports.
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
use net::client::{JoinParams, NetClient, NetEvent, PendingClient, Trust};
use protocol::messages::buttons;
use protocol::quant::{axis_from_i8, axis_to_i8};
use protocol::{Angle16, ClaimedIdentity, ColumnData, Hello, InputDatagram, InputFrame};
use sim::physics::{MapBlocks, PhysicsConfig, PlayerBody, PlayerInput, interpolate, step};
use sim::raycast::RayHit;
use world::coords::SectionPos;
use world::map::DirtyKind;
use world::registry::IdTable;
use world::registry::blocks_from_dir;
use world::storage::{ColumnRecord, StorageError, WorldStore};
use world::{BlockStateId, Column, ColumnPos, Registry, RegistryBuilder, WorldBounds, WorldMap};
use worldgen::{GenContext, Pipeline, WorldgenConfig};

use crate::edit::{BatchSync, Editor, Placer, dig, pick, place};
use crate::loading::{LoadConfig, Loader, Viewer};
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
    /// Joining the server failed or the connection ended.
    #[error("{0}")]
    Net(String),
}

/// Input frames repeated in every datagram (a lost datagram costs nothing).
const INPUT_REDUNDANCY: usize = 3;

/// Where to play online.
#[derive(Clone, Debug)]
pub struct ConnectParams {
    /// Server address.
    pub addr: SocketAddr,
    /// The player.
    pub identity: auth::Identity,
    /// Limit for connecting and the handshake.
    pub timeout: Duration,
}

/// Where the world comes from.
enum Backend {
    /// Single-player: the save directory.
    Local { dir: PathBuf, store: WorldStore },
    /// A server.
    Remote(Box<Remote>),
}

struct Remote {
    net: NetClient<Column>,
    /// Columns received and not yet taken by the loader.
    received: HashMap<ColumnPos, Column>,
    /// Client tick of the next input frame (continues the server's tick).
    tick: u32,
    /// The last few input frames, newest last.
    frames: VecDeque<InputFrame>,
    joined: Instant,
    /// Time from connecting to controlling the player.
    to_control: Option<Duration>,
    connect_time: Duration,
    columns: u64,
}

/// Mod data a session needs.
struct Data {
    registry: Arc<Registry>,
    gen_config: WorldgenConfig,
    physics: PhysicsConfig,
    swim: Vec<BlockStateId>,
}

fn load_data(config: &SessionConfig) -> Result<Data, SessionError> {
    let data_err = |e: &dyn std::fmt::Display| SessionError::Data(e.to_string());
    let mut b = RegistryBuilder::new();
    b.add_mod(
        "core",
        blocks_from_dir(&config.data.join("blocks")).map_err(|e| data_err(&e))?,
    );
    let registry = Arc::new(b.build().map_err(|e| data_err(&e))?);
    let src = std::fs::read_to_string(config.data.join("worldgen/overworld.toml"))
        .map_err(|e| data_err(&e))?;
    let gen_config = WorldgenConfig::from_toml(&src, &registry).map_err(|e| data_err(&e))?;
    let physics_src = std::fs::read_to_string(config.data.join("player/physics.toml"))
        .map_err(|e| data_err(&e))?;
    let physics = PhysicsConfig::from_toml(&physics_src).map_err(|e| data_err(&e))?;
    let swim = physics.swim_ids(&registry).map_err(|e| data_err(&e))?;
    Ok(Data {
        registry,
        gen_config,
        physics,
        swim,
    })
}

/// Unpacks a column record into a column of the client's registry. Runs on
/// the network threads.
fn unpack_column(
    data: ColumnData,
    bounds: WorldBounds,
    table: &[BlockStateId],
    registry: &Registry,
) -> Result<Column, String> {
    if data.bounds() != bounds {
        return Err(format!(
            "column {:?} has bounds {:?}, the world has {bounds:?}",
            data.pos(),
            data.bounds()
        ));
    }
    let mut sections = data.sections().map_err(|e| e.to_string())?;
    let missing = registry.missing();
    for s in &mut sections {
        s.remap(table, missing);
    }
    Column::from_sections(data.pos(), bounds, sections, registry)
        .ok_or_else(|| "bad section count".to_string())
}

/// The quantized frame of one tick of input (what is sent to the server).
fn quantize(input: &PlayerInput, pitch: f64, tick: u32) -> InputFrame {
    let mut b = 0;
    for (on, bit) in [
        (input.jump, buttons::JUMP),
        (input.sprint, buttons::SPRINT),
        (input.descend, buttons::DESCEND),
        (input.toggle_fly, buttons::TOGGLE_FLY),
    ] {
        if on {
            b |= bit;
        }
    }
    InputFrame {
        tick,
        buttons: b,
        forward: axis_to_i8(input.forward),
        strafe: axis_to_i8(input.strafe),
        fly_speed: (input.fly_speed * 16.0).round().clamp(1.0, 255.0) as u8,
        yaw: Angle16::from_radians(input.yaw),
        pitch: Angle16::from_radians(pitch),
    }
}

/// The input the server simulates for a frame; the client predicts with the
/// same values so both agree (`server::world_thread::input_of`).
#[must_use]
pub fn dequantize(f: &InputFrame) -> PlayerInput {
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

/// A running world: single-player or on a server.
pub struct Session {
    registry: Arc<Registry>,
    backend: Backend,
    /// World metadata (on a server: its name, nothing saved).
    info: WorldInfo,
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
        let data_err = |e: &dyn std::fmt::Display| SessionError::Data(e.to_string());
        let Data {
            registry,
            gen_config,
            physics,
            swim,
        } = load_data(&config)?;
        let info = WorldInfo::load(dir)?;
        let store = WorldStore::open(dir, &registry, config.bounds)?;
        let ctx = Arc::new(GenContext::new(info.seed, gen_config, config.bounds));
        let mut loader = Loader::new(
            LoadConfig {
                radius: config.radius,
                ..LoadConfig::default()
            },
            Arc::clone(&registry),
            Arc::new(Pipeline::overworld()),
            ctx,
        );
        loader.track_mesh_changes();
        loader.track_unloads();
        let map = WorldMap::new(Arc::clone(&registry), config.bounds);
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
            backend: Backend::Local {
                dir: dir.to_owned(),
                store,
            },
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

    /// Joins a server (offline identity only until T0-10): handshake, then
    /// columns stream in; the player is placed when the server's `Spawn`
    /// arrives. Blocks for at most `params.timeout`.
    pub fn connect(
        params: &ConnectParams,
        mut config: SessionConfig,
    ) -> Result<Self, SessionError> {
        let started = Instant::now();
        let data_err = |e: &dyn std::fmt::Display| SessionError::Data(e.to_string());
        let Data {
            registry,
            gen_config,
            physics,
            swim,
        } = load_data(&config)?;
        let hello = Hello {
            client_version: env!("CARGO_PKG_VERSION").into(),
            identity: ClaimedIdentity::Offline {
                name: params.identity.name().as_str().to_owned(),
            },
            session_jwt: None,
        };
        let join = JoinParams {
            addr: params.addr,
            server_name: "localhost".into(),
            hello,
            // OPEN-QUESTION: 16
            trust: Trust::OfflineAnyCertificate,
            timeout: params.timeout,
        };
        let (pending, joined) =
            PendingClient::connect(&join).map_err(|e| SessionError::Net(e.to_string()))?;
        let table = registry.remap(&IdTable {
            names: joined.registry.id_table.clone(),
        });
        let bounds = config.bounds;
        let unpack_registry = Arc::clone(&registry);
        let net = pending.start(move |data| unpack_column(data, bounds, &table, &unpack_registry));
        // The server streams `view_distance` columns; light and meshes are
        // ready two columns less (they need neighbours).
        let view = i32::from(joined.welcome.server_info.view_distance);
        config.radius = (view - 2).max(1);
        let mut loader = Loader::new(
            LoadConfig {
                radius: config.radius,
                ..LoadConfig::default()
            },
            Arc::clone(&registry),
            Arc::new(Pipeline::overworld()),
            Arc::new(GenContext::new(0, gen_config, bounds)),
        );
        loader.set_remote(true);
        loader.track_mesh_changes();
        loader.track_unloads();
        let map = WorldMap::new(Arc::clone(&registry), bounds);
        let mut light = WorldLight::new();
        light.track_changes();
        let pool =
            JobPool::with_low_priority_workers(config.threads.max(1)).map_err(|e| data_err(&e))?;
        let editor = Editor::new(&map);
        let body = PlayerBody {
            pos: DVec3::new(8.5, 100.0, 8.5),
            ..PlayerBody::default()
        };
        let info = WorldInfo {
            name: joined.welcome.server_info.name.clone(),
            seed: 0,
            generator: "server".into(),
            created: 0,
            last_played: now(),
            player: crate::worlds::PlayerSave::default(),
        };
        Ok(Self {
            registry,
            backend: Backend::Remote(Box::new(Remote {
                net,
                received: HashMap::new(),
                tick: 0,
                frames: VecDeque::new(),
                joined: started,
                to_control: None,
                connect_time: started.elapsed(),
                columns: 0,
            })),
            yaw: 0.0,
            fly_speed: 1.0,
            pitch: 0.0,
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
            spawned: false,
            unsaved: HashSet::new(),
            dirty: Vec::new(),
            unloaded: Vec::new(),
            gone: Vec::new(),
            light_changed: Vec::new(),
            last_save: Instant::now(),
            config,
        })
    }

    /// On a server: the time from starting to connect until the player
    /// could move (the "login" metric), once reached.
    #[must_use]
    pub fn time_to_control(&self) -> Option<Duration> {
        match &self.backend {
            Backend::Remote(r) => r.to_control,
            Backend::Local { .. } => None,
        }
    }

    /// On a server: the time the connection and handshake took.
    #[must_use]
    pub fn connect_time(&self) -> Option<Duration> {
        match &self.backend {
            Backend::Remote(r) => Some(r.connect_time),
            Backend::Local { .. } => None,
        }
    }

    /// On a server: columns received so far.
    #[must_use]
    pub fn columns_received(&self) -> Option<u64> {
        match &self.backend {
            Backend::Remote(r) => Some(r.columns),
            Backend::Local { .. } => None,
        }
    }

    /// Whether the world comes from a server.
    #[must_use]
    pub fn is_remote(&self) -> bool {
        matches!(self.backend, Backend::Remote(_))
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
            Backend::Remote(remote) => {
                Self::receive(
                    remote,
                    &mut self.body,
                    &mut self.prev,
                    &mut self.yaw,
                    &mut self.pitch,
                    &mut self.spawned,
                )?;
                // Columns that arrive before `Spawn` are kept: the player's
                // position (and so what to keep) is not known yet.
                if !self.spawned {
                    return Ok(());
                }
                // `Spawn` may just have moved the player.
                let viewer = Viewer {
                    pos: self.body.pos.to_array(),
                    velocity: (self.body.vel * 20.0).to_array(),
                };
                // Keep what the loader keeps (it unloads beyond radius + 4).
                let c = viewer.column();
                let keep = self.config.radius + 4;
                remote
                    .received
                    .retain(|p, _| (p.x - c.x).abs() <= keep && (p.z - c.z).abs() <= keep);
                let received = &mut remote.received;
                let mut source = |c: ColumnPos| received.remove(&c);
                self.loader.tick_with_source(
                    &viewer,
                    &mut self.map,
                    &mut self.light,
                    &self.pool,
                    Some(&mut source),
                );
            }
        }
        if self.is_remote() {
            let ready = self.spawned && self.ready_around(self.body.pos);
            if ready
                && let Backend::Remote(r) = &mut self.backend
                && r.to_control.is_none()
            {
                r.to_control = Some(r.joined.elapsed());
            }
        } else if !self.spawned
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
        if !self.is_remote() {
            self.track_saves()?;
            if self.last_save.elapsed() >= self.config.autosave {
                self.save()?;
            }
        }
        Ok(())
    }

    /// Applies what the server sent since the last frame.
    fn receive(
        remote: &mut Remote,
        body: &mut PlayerBody,
        prev: &mut PlayerBody,
        yaw: &mut f64,
        pitch: &mut f64,
        spawned: &mut bool,
    ) -> Result<(), SessionError> {
        while let Some(event) = remote.net.try_event() {
            match event {
                NetEvent::Spawn(s) => {
                    *body = PlayerBody {
                        pos: DVec3::from_array(s.pos.to_blocks()),
                        ..PlayerBody::default()
                    };
                    *prev = *body;
                    *yaw = s.yaw.to_radians();
                    *pitch = s.pitch.to_signed_radians();
                    remote.tick = s.tick.wrapping_add(1);
                    *spawned = true;
                }
                NetEvent::Column(col) => {
                    remote.columns += 1;
                    remote.received.insert(col.pos(), col);
                }
                NetEvent::BadColumn(e) => {
                    return Err(SessionError::Net(format!(
                        "the server sent a bad column: {e}"
                    )));
                }
                NetEvent::Kicked(text) => {
                    return Err(SessionError::Net(format!("kicked from the server: {text}")));
                }
                NetEvent::Closed(why) => {
                    return Err(SessionError::Net(format!("disconnected: {why}")));
                }
            }
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
        let mut input = PlayerInput {
            yaw: self.yaw,
            fly_speed: self.fly_speed,
            ..*input
        };
        if let Backend::Remote(r) = &mut self.backend {
            // Send the quantized frame and predict with exactly what the
            // server will simulate.
            let frame = quantize(&input, self.pitch, r.tick);
            r.tick = r.tick.wrapping_add(1);
            r.frames.push_back(frame);
            while r.frames.len() > INPUT_REDUNDANCY {
                r.frames.pop_front();
            }
            r.net.send_input(&InputDatagram {
                ack_snapshot: 0,
                frames: r.frames.iter().copied().collect(),
            });
            input = dequantize(&frame);
        }
        let world = MapBlocks {
            map: &self.map,
            swim: &self.swim,
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
        let [lo, hi] = self.physics.fly_speed_range;
        self.fly_speed = speed.clamp(lo, hi);
    }

    /// Changes the fly speed by mouse-wheel notches (×1.25 per notch), within
    /// the movement profile's range.
    pub fn adjust_fly_speed(&mut self, notches: i32) {
        let [lo, hi] = self.physics.fly_speed_range;
        self.fly_speed = (self.fly_speed * 1.25f64.powi(notches)).clamp(lo, hi);
    }

    /// Turns the view (radians); pitch is clamped to straight up/down.
    pub fn look(&mut self, dyaw: f64, dpitch: f64) {
        self.yaw = (self.yaw + dyaw).rem_euclid(std::f64::consts::TAU);
        self.pitch = (self.pitch + dpitch).clamp(-1.55, 1.55);
    }

    /// Eye position interpolated `alpha` of the way into the current tick,
    /// and the view direction.
    #[must_use]
    pub fn camera(&self, alpha: f64) -> (DVec3, DVec3) {
        let feet = interpolate(&self.prev, &self.body, alpha);
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
        // Edits over the network arrive with T2-04.
        if self.is_remote() {
            return false;
        }
        let Some(hit) = self.target() else {
            return false;
        };
        dig(&mut self.editor, &mut self.map, &mut self.loader, &hit).is_ok()
    }

    /// Places `id` against the targeted face.
    pub fn place(&mut self, id: BlockStateId) -> bool {
        if self.is_remote() {
            return false;
        }
        let Some(hit) = self.target() else {
            return false;
        };
        let placer = Placer {
            body: &self.body,
            cfg: &self.physics,
            swim: &self.swim,
        };
        matches!(
            place(
                &mut self.editor,
                &mut self.map,
                &mut self.loader,
                &hit,
                id,
                placer
            ),
            Ok(Some(_))
        )
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
        // On a server the server saves.
        if self.is_remote() {
            return Ok(());
        }
        self.track_saves()?;
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
        if !records.is_empty()
            && let Backend::Local { store, .. } = &mut self.backend
        {
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
        if let Backend::Local { dir, .. } = &self.backend {
            self.info.save(dir)?;
        }
        self.last_save = Instant::now();
        Ok(())
    }

    /// Saves and closes (on a server: leaves it).
    pub fn close(mut self) -> Result<(), SessionError> {
        self.save()?;
        if let Backend::Remote(r) = self.backend {
            r.net.disconnect();
        }
        Ok(())
    }
}
