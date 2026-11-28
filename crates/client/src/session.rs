//! A single-player game session: the world (generated or loaded from the
//! save), the player, edits and saving. No window or GPU here, so the whole
//! play loop runs in tests; the app feeds input and draws what this reports.
//!
//! Timing: [`Session::update`] once per frame (loading, relighting, saving),
//! [`Session::fixed_tick`] at 20 TPS (physics).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use engine_core::jobs::JobPool;
use glam::{DVec3, IVec3};
use light::{SectionLight, WorldLight};
use mesh::PackedBatch;
use sim::physics::{MapBlocks, PhysicsConfig, PlayerBody, PlayerInput, interpolate, step};
use sim::raycast::RayHit;
use world::coords::SectionPos;
use world::map::DirtyKind;
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
}

/// Where the last [`Session::update`] spent its time (T1-25 spike
/// attribution). Filled on every call; reading it costs nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UpdateCost {
    /// The whole call.
    pub total: Duration,
    /// The loader tick, including the saved-column reads below.
    pub loader: Duration,
    /// Saved-column reads on the main thread (part of `loader`).
    pub source: Duration,
    /// Saved-column reads made.
    pub source_calls: u32,
    /// Relighting edited sections.
    pub light: Duration,
    /// Edited-column bookkeeping and writes of unloaded edited columns.
    pub saves: Duration,
    /// Columns that left the map.
    pub unloaded: u32,
    /// The autosave, if it ran in this call (zero otherwise).
    pub autosave: Duration,
}

/// A running single-player world.
pub struct Session {
    registry: Arc<Registry>,
    dir: PathBuf,
    info: WorldInfo,
    store: WorldStore,
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
    cost: UpdateCost,
}

impl Session {
    /// Opens the world in `dir` (created with [`crate::worlds::create`]).
    pub fn open(dir: &Path, config: SessionConfig) -> Result<Self, SessionError> {
        let data_err = |e: &dyn std::fmt::Display| SessionError::Data(e.to_string());
        let mut b = RegistryBuilder::new();
        b.add_mod(
            "core",
            blocks_from_dir(&config.data.join("blocks")).map_err(|e| data_err(&e))?,
        );
        let registry = Arc::new(b.build().map_err(|e| data_err(&e))?);
        let info = WorldInfo::load(dir)?;
        let src = std::fs::read_to_string(config.data.join("worldgen/overworld.toml"))
            .map_err(|e| data_err(&e))?;
        let gen_config = WorldgenConfig::from_toml(&src, &registry).map_err(|e| data_err(&e))?;
        let physics_src = std::fs::read_to_string(config.data.join("player/physics.toml"))
            .map_err(|e| data_err(&e))?;
        let physics = PhysicsConfig::from_toml(&physics_src).map_err(|e| data_err(&e))?;
        let swim = physics.swim_ids(&registry).map_err(|e| data_err(&e))?;
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
            dir: dir.to_owned(),
            yaw: info.player.yaw,
            fly_speed: if info.player.fly_speed > 0.0 {
                info.player.fly_speed
            } else {
                1.0
            },
            pitch: info.player.pitch,
            info,
            store,
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
        let t0 = Instant::now();
        let mut cost = UpdateCost::default();
        let viewer = Viewer {
            pos: self.body.pos.to_array(),
            velocity: (self.body.vel * 20.0).to_array(),
        };
        {
            let (store, registry) = (&mut self.store, &self.registry);
            let (source_time, source_calls) = (&mut cost.source, &mut cost.source_calls);
            let mut source = |c: ColumnPos| {
                let t = Instant::now();
                let col = store
                    .load_column(c, registry)
                    .ok()
                    .flatten()
                    .map(|(col, _)| col);
                *source_time += t.elapsed();
                *source_calls += 1;
                col
            };
            self.loader.tick_with_source(
                &viewer,
                &mut self.map,
                &mut self.light,
                &self.pool,
                Some(&mut source),
            );
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
            if self.last_save.elapsed() >= self.config.autosave {
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
        if !records.is_empty() {
            self.store.save_all(&records)?;
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
        let input = PlayerInput {
            yaw: self.yaw,
            fly_speed: self.fly_speed,
            ..*input
        };
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
        let Some(hit) = self.target() else {
            return false;
        };
        dig(&mut self.editor, &mut self.map, &mut self.loader, &hit).is_ok()
    }

    /// Places `id` against the targeted face.
    pub fn place(&mut self, id: BlockStateId) -> bool {
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
        if !records.is_empty() {
            self.store.save_all(&records)?;
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
        self.info.save(&self.dir)?;
        self.last_save = Instant::now();
        Ok(())
    }

    /// Saves and closes.
    pub fn close(mut self) -> Result<(), SessionError> {
        self.save()
    }
}
