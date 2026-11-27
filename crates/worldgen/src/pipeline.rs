//! The generation pipeline (`docs/architecture/worldgen.md`, "Pipeline").
//!
//! A column is a pure function of `(seed, column position)`: every stage reads
//! only the seed, the configuration and the column being built. Features that
//! cross column borders (trees) are placed by **every** column they touch,
//! each computing the same feature from its origin's coordinates and writing
//! only its own cells. So generation order and thread count never change the
//! result, without a cross-column dependency graph (tested).

use std::sync::Arc;

use engine_core::jobs::{CancelToken, JobPool, Priority};
use noise::{NoiseFn, Perlin};
use world::{BlockStateId, Column, ColumnPos, Registry, SECTION_VOLUME, Section, WorldBounds};

use crate::config::{Climate, Engine, WorldgenConfig};
#[cfg(feature = "builtin-overworld")]
use crate::stages;

/// Shared, read-only generation context.
pub struct GenContext {
    /// World seed.
    pub seed: u64,
    /// Resolved parameters.
    pub config: WorldgenConfig,
    /// Vertical bounds.
    pub bounds: WorldBounds,
    pub(crate) hills: Perlin,
    pub(crate) detail: Perlin,
    pub(crate) temperature: Perlin,
    #[cfg_attr(not(feature = "builtin-overworld"), allow(dead_code))]
    pub(crate) caves: Perlin,
    pub(crate) continents: Perlin,
    pub(crate) erosion: Perlin,
    pub(crate) ridges: Perlin,
    pub(crate) humidity: Perlin,
}

/// The climate at a block column (`climate` engine; the `classic` engine fills
/// only `temperature`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClimateSample {
    /// Land (> 0) vs ocean (< 0), −1..1.
    pub continentalness: f64,
    /// Flat (high) vs mountainous (low), −1..1.
    pub erosion: f64,
    /// 0 in valleys, 1 on ridges.
    pub ridges: f64,
    /// 0..1, after height cooling.
    pub temperature: f64,
    /// 0..1.
    pub humidity: f64,
}

/// Fractal (octave) Perlin noise, roughly −1..1.
fn fbm(noise: &Perlin, x: f64, z: f64, octaves: u32) -> f64 {
    let (mut sum, mut amp, mut freq, mut norm) = (0.0, 1.0, 1.0, 0.0);
    for _ in 0..octaves {
        sum += amp * noise.get([x * freq, z * freq]);
        norm += amp;
        amp *= 0.5;
        freq *= 2.0;
    }
    sum / norm
}

/// Surface data of one block column, computed without generating anything else.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Surface {
    /// y of the topmost terrain block.
    pub height: i32,
    /// Index into `config.biomes`.
    pub biome: u8,
}

/// splitmix64: the seed mixer for every random decision.
#[must_use]
pub fn mix(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

impl GenContext {
    /// A context for `seed`.
    #[must_use]
    pub fn new(seed: u64, config: WorldgenConfig, bounds: WorldBounds) -> Self {
        let p = |domain: u64| Perlin::new(mix(seed ^ domain) as u32);
        Self {
            seed,
            config,
            bounds,
            hills: p(1),
            detail: p(2),
            temperature: p(3),
            caves: p(4),
            continents: p(5),
            erosion: p(6),
            ridges: p(7),
            humidity: p(8),
        }
    }

    /// The climate engine's shaping at world `(x, z)`: height and climate.
    #[must_use]
    pub fn climate(&self, wx: i32, wz: i32) -> (i32, ClimateSample) {
        match &self.config.engine {
            Engine::Climate(c) => self.climate_with(c, wx, wz),
            Engine::Classic => {
                let s = self.surface(wx, wz);
                let t = &self.config.terrain;
                let (x, z) = (f64::from(wx) * t.scale, f64::from(wz) * t.scale);
                let temperature =
                    (self.temperature.get([x * 0.35, z * 0.35]) * 0.5 + 0.5).clamp(0.0, 1.0);
                (
                    s.height,
                    ClimateSample {
                        continentalness: 0.0,
                        erosion: 0.0,
                        ridges: 0.0,
                        temperature,
                        humidity: 0.5,
                    },
                )
            }
        }
    }

    fn climate_with(&self, c: &Climate, wx: i32, wz: i32) -> (i32, ClimateSample) {
        let (x, z) = (f64::from(wx), f64::from(wz));
        let cs = c.continentalness_scale;
        let continentalness = (fbm(&self.continents, x * cs, z * cs, 4) * 1.6).clamp(-1.0, 1.0);
        let es = c.erosion_scale;
        let erosion = (fbm(&self.erosion, x * es, z * es, 3) * 1.6).clamp(-1.0, 1.0);
        let rs = c.ridge_scale;
        let ridges = 1.0 - (fbm(&self.ridges, x * rs, z * rs, 3) * 1.6).abs().min(1.0);
        let ds = c.detail_scale;
        let detail = fbm(&self.detail, x * ds, z * ds, 2);
        let [a, b] = c.inland;
        let land = if b > a {
            let t = ((continentalness - a) / (b - a)).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        } else {
            1.0
        };
        let height = c.continentalness.at(continentalness)
            + c.ridges.at(ridges) * c.erosion.at(erosion) * land
            + c.detail * detail;
        let height =
            (height.round() as i32).clamp(self.bounds.min_y() + 1, self.bounds.max_y() - 16);
        let ts = c.temperature_scale;
        let above_sea = f64::from((height - self.config.sea_level).max(0));
        let temperature = ((fbm(&self.temperature, x * ts, z * ts, 2) * 0.8 + 0.5)
            - above_sea * c.height_cooling)
            .clamp(0.0, 1.0);
        let hs = c.humidity_scale;
        let humidity = (fbm(&self.humidity, x * hs, z * hs, 2) * 0.8 + 0.5).clamp(0.0, 1.0);
        (
            height,
            ClimateSample {
                continentalness,
                erosion,
                ridges,
                temperature,
                humidity,
            },
        )
    }

    /// Surface height and biome at world `(x, z)`: the cheap 2D pass, also
    /// used for far LOD and by features in neighboring columns.
    #[must_use]
    pub fn surface(&self, wx: i32, wz: i32) -> Surface {
        if let Engine::Climate(c) = &self.config.engine {
            let (height, k) = self.climate_with(c, wx, wz);
            let last = self.config.biomes.len() - 1;
            let biome = self
                .config
                .biomes
                .iter()
                .position(|b| {
                    b.climate.is_none_or(|r| {
                        r.contains(k.temperature, k.humidity, k.continentalness, height)
                    })
                })
                .unwrap_or(last) as u8;
            return Surface { height, biome };
        }
        let t = &self.config.terrain;
        let (x, z) = (f64::from(wx), f64::from(wz));
        let s = t.scale;
        let hills =
            self.hills.get([x * s, z * s]) + 0.5 * self.hills.get([x * s * 2.0, z * s * 2.0]);
        let detail = self.detail.get([x * s * 6.0, z * s * 6.0]);
        let height =
            (f64::from(t.base_height) + t.amplitude * hills + t.roughness * detail).round() as i32;
        let height = height.clamp(self.bounds.min_y() + 1, self.bounds.max_y() - 16);
        let temp = (self.temperature.get([x * s * 0.35, z * s * 0.35]) * 0.5 + 0.5).clamp(0.0, 1.0);
        let biome = self
            .config
            .biomes
            .iter()
            .position(|b| temp <= b.max_temperature)
            .unwrap_or(self.config.biomes.len() - 1) as u8;
        Surface { height, biome }
    }

    /// A deterministic random stream for a decision keyed by `key`.
    #[must_use]
    pub fn rng(&self, key: &[i64]) -> Rng {
        let mut h = mix(self.seed);
        for &k in key {
            h = mix(h ^ k as u64);
        }
        Rng(h | 1)
    }
}

/// xorshift64 stream.
pub struct Rng(u64);

impl Rng {
    /// Next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// Uniform in `lo..hi` (`hi > lo`).
    pub fn range(&mut self, lo: i32, hi: i32) -> i32 {
        lo + (self.next_u64() % (hi - lo).max(1) as u64) as i32
    }

    /// Uniform in `0.0..1.0`.
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// A column under construction: flat block array (section-major, YZX) and
/// per-block-column surface data.
pub struct ProtoColumn {
    /// Position.
    pub pos: ColumnPos,
    bounds: WorldBounds,
    blocks: Vec<BlockStateId>,
    /// Surface per `(z * 16 + x)`.
    pub surface: [Surface; 256],
}

impl ProtoColumn {
    /// An all-air column.
    #[must_use]
    pub fn new(pos: ColumnPos, bounds: WorldBounds) -> Self {
        Self {
            pos,
            bounds,
            blocks: vec![BlockStateId::AIR; bounds.section_count as usize * SECTION_VOLUME],
            surface: [Surface {
                height: 0,
                biome: 0,
            }; 256],
        }
    }

    /// Vertical bounds.
    #[must_use]
    pub fn bounds(&self) -> WorldBounds {
        self.bounds
    }

    #[inline]
    fn index(&self, x: usize, y: i32, z: usize) -> Option<usize> {
        let ry = y - self.bounds.min_y();
        if x < 16 && z < 16 && (0..self.bounds.max_y() - self.bounds.min_y()).contains(&ry) {
            Some(((ry as usize) * 16 + z) * 16 + x)
        } else {
            None
        }
    }

    /// Block at local `x, z` and world `y` (`AIR` outside).
    #[inline]
    #[must_use]
    pub fn get(&self, x: usize, y: i32, z: usize) -> BlockStateId {
        self.index(x, y, z)
            .map_or(BlockStateId::AIR, |i| self.blocks[i])
    }

    /// Sets a block; ignored outside the column.
    #[inline]
    pub fn set(&mut self, x: usize, y: i32, z: usize, id: BlockStateId) {
        if let Some(i) = self.index(x, y, z) {
            self.blocks[i] = id;
        }
    }

    /// Packs into palette sections and computes the heightmap.
    #[must_use]
    pub fn into_column(self, registry: &Registry) -> Option<Column> {
        let sections = self
            .blocks
            .chunks_exact(SECTION_VOLUME)
            .map(|chunk| {
                let mut arr = [BlockStateId::AIR; SECTION_VOLUME];
                arr.copy_from_slice(chunk);
                Section::from_blocks(&arr)
            })
            .collect();
        Column::from_sections(self.pos, self.bounds, sections, registry)
    }

    /// FNV-1a hash of every block in YZX order (the determinism check).
    #[must_use]
    pub fn hash(&self) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for b in &self.blocks {
            for byte in b.0.to_le_bytes() {
                h = (h ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
            }
        }
        h
    }
}

/// One generation stage. Built-in stages are configured by mod data; mods add
/// or replace stages by id (stage 3 exposes this through the mod API).
pub trait Stage: Send + Sync {
    /// Stable id, e.g. `core:terrain`.
    fn id(&self) -> &str;
    /// Runs the stage on one column. Must be a pure function of the context
    /// and the column.
    fn run(&self, ctx: &GenContext, column: &mut ProtoColumn);
}

/// An ordered list of stages.
pub struct Pipeline {
    stages: Vec<Box<dyn Stage>>,
}

impl Pipeline {
    /// A pipeline of the given stages, in order.
    #[must_use]
    pub fn new(stages: Vec<Box<dyn Stage>>) -> Self {
        Self { stages }
    }

    /// The built-in overworld pipeline: surface (heightmap + biomes) → terrain →
    /// caves → ores → trees.
    #[cfg(feature = "builtin-overworld")]
    #[must_use]
    pub fn overworld() -> Self {
        Self {
            stages: vec![
                Box::new(stages::SurfacePass),
                Box::new(stages::TerrainFill),
                Box::new(stages::CaveCarver),
                Box::new(stages::OreVeins),
                Box::new(stages::TreePlacer),
            ],
        }
    }

    /// Stage ids in order.
    #[must_use]
    pub fn ids(&self) -> Vec<&str> {
        self.stages.iter().map(|s| s.id()).collect()
    }

    /// Replaces the stage with `id`; returns false if there is none.
    pub fn replace(&mut self, id: &str, stage: Box<dyn Stage>) -> bool {
        match self.stages.iter().position(|s| s.id() == id) {
            Some(i) => {
                self.stages[i] = stage;
                true
            }
            None => false,
        }
    }

    /// Inserts a stage after `id` (at the end if `id` is absent).
    pub fn insert_after(&mut self, id: &str, stage: Box<dyn Stage>) {
        let at = self
            .stages
            .iter()
            .position(|s| s.id() == id)
            .map_or(self.stages.len(), |i| i + 1);
        self.stages.insert(at, stage);
    }

    /// Generates one column.
    #[must_use]
    pub fn generate(&self, ctx: &GenContext, pos: ColumnPos) -> ProtoColumn {
        let mut column = ProtoColumn::new(pos, ctx.bounds);
        for stage in &self.stages {
            stage.run(ctx, &mut column);
        }
        column
    }

    /// Generates columns as `Visible` jobs on `pool`; returns them sorted by
    /// position (results arrive in any order).
    #[must_use]
    pub fn generate_parallel(
        self: &Arc<Self>,
        ctx: &Arc<GenContext>,
        pool: &JobPool,
        positions: &[ColumnPos],
    ) -> Vec<ProtoColumn> {
        for &pos in positions {
            let (pipeline, ctx) = (Arc::clone(self), Arc::clone(ctx));
            pool.submit(Priority::Visible, CancelToken::new(), move |_| {
                pipeline.generate(&ctx, pos)
            });
        }
        let mut out = Vec::with_capacity(positions.len());
        while out.len() < positions.len() {
            let before = out.len();
            out.extend(
                pool.drain_completed()
                    .filter_map(|j| j.downcast::<ProtoColumn>()),
            );
            if out.len() == before {
                std::thread::yield_now();
            }
        }
        out.sort_by_key(|c| (c.pos.x, c.pos.z));
        out
    }
}
