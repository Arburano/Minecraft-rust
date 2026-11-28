//! Generation parameters from mod data (`<mod>/data/worldgen/*.toml`), resolved
//! against the block registry. The engine knows no blocks: every block the
//! built-in stages place is named here by mod data.
//!
//! The `engine` key picks how the landscape is shaped (`docs/architecture/
//! worldgen.md`, "Engines"): `classic` (one hills noise, biomes by
//! temperature) or `climate` (continents, erosion, ridges, temperature and
//! humidity, height splines, biomes by climate ranges). Everything after the
//! shape (surface layers, caves, ores, trees) is shared.

use std::fmt;

use serde::Deserialize;
use world::{BlockStateId, Registry};

/// Errors loading generation parameters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigError {
    /// Not valid TOML or not the expected schema.
    Parse(String),
    /// A block name is not in the registry.
    UnknownBlock(String),
    /// A value is out of range (named).
    Invalid(String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(e) => write!(f, "worldgen config: {e}"),
            Self::UnknownBlock(b) => write!(f, "worldgen config: unknown block {b:?}"),
            Self::Invalid(what) => write!(f, "worldgen config: invalid {what}"),
        }
    }
}

impl std::error::Error for ConfigError {}

fn classic() -> String {
    "classic".into()
}

fn one() -> f64 {
    1.0
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    #[serde(default = "classic")]
    engine: String,
    #[serde(default)]
    climate: Option<RawClimate>,
    sea_level: i32,
    terrain: RawTerrain,
    caves: RawCaves,
    biomes: Vec<RawBiome>,
    beach: RawBeach,
    #[serde(default)]
    ores: Vec<RawOre>,
    trees: RawTrees,
    #[serde(default)]
    tree_kinds: Vec<RawTreeKind>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawClimate {
    continentalness_scale: f64,
    erosion_scale: f64,
    ridge_scale: f64,
    temperature_scale: f64,
    humidity_scale: f64,
    detail_scale: f64,
    detail: f64,
    continentalness: Vec<[f64; 2]>,
    erosion: Vec<[f64; 2]>,
    ridges: Vec<[f64; 2]>,
    inland: [f64; 2],
    #[serde(default)]
    height_cooling: f64,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawRanges {
    temperature: Option<[f64; 2]>,
    humidity: Option<[f64; 2]>,
    continentalness: Option<[f64; 2]>,
    height: Option<[i32; 2]>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTreeKind {
    id: String,
    log: String,
    leaves: String,
    min_trunk: i32,
    max_trunk: i32,
    #[serde(default)]
    shape: TreeShape,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTerrain {
    base_height: i32,
    amplitude: f64,
    roughness: f64,
    scale: f64,
    stone: String,
    fluid: String,
    #[serde(default)]
    bedrock_depth: i32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCaves {
    air: String,
    threshold: f64,
    scale: f64,
    min_y: i32,
    max_below_surface: i32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBiome {
    id: String,
    #[serde(default = "one")]
    max_temperature: f64,
    #[serde(default)]
    climate: Option<RawRanges>,
    #[serde(default)]
    trees: Vec<String>,
    top: String,
    filler: String,
    filler_depth: i32,
    trees_per_column: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBeach {
    block: String,
    max_above_sea: i32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOre {
    block: String,
    replaces: String,
    veins_per_column: u32,
    vein_size: u32,
    min_y: i32,
    max_y: i32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTrees {
    log: String,
    leaves: String,
    min_trunk: i32,
    max_trunk: i32,
    #[serde(default)]
    shape: TreeShape,
}

/// The crown of a tree kind (radius ≤ 2, so trees across column borders stay
/// exact: `stages::TreePlacer`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TreeShape {
    /// A rounded crown around the trunk top (oak, birch).
    #[default]
    Round,
    /// A narrow cone of alternating rings down the trunk (spruce).
    Cone,
}

/// A piecewise-linear curve through `(x, y)` points sorted by `x`; flat
/// beyond the ends.
#[derive(Clone, Debug, PartialEq)]
pub struct Spline {
    points: Vec<[f64; 2]>,
}

impl Spline {
    fn new(name: &str, mut points: Vec<[f64; 2]>) -> Result<Self, ConfigError> {
        if points.is_empty() || points.iter().flatten().any(|v| !v.is_finite()) {
            return Err(ConfigError::Invalid(format!("climate.{name}: points")));
        }
        points.sort_by(|a, b| a[0].total_cmp(&b[0]));
        Ok(Self { points })
    }

    /// The curve at `x`.
    #[must_use]
    pub fn at(&self, x: f64) -> f64 {
        let p = &self.points;
        let i = p.partition_point(|q| q[0] <= x);
        if i == 0 {
            return p[0][1];
        }
        if i == p.len() {
            return p[p.len() - 1][1];
        }
        let ([x0, y0], [x1, y1]) = (p[i - 1], p[i]);
        let t = if x1 > x0 { (x - x0) / (x1 - x0) } else { 0.0 };
        y0 + (y1 - y0) * t
    }
}

/// Parameters of the `climate` engine.
#[derive(Clone, Debug)]
pub struct Climate {
    /// Horizontal frequency of the continent noise (land vs ocean).
    pub continentalness_scale: f64,
    /// Frequency of the erosion noise (flat vs mountainous).
    pub erosion_scale: f64,
    /// Frequency of the ridge noise (peaks and valleys).
    pub ridge_scale: f64,
    /// Frequency of the temperature noise.
    pub temperature_scale: f64,
    /// Frequency of the humidity noise.
    pub humidity_scale: f64,
    /// Frequency of the small detail noise.
    pub detail_scale: f64,
    /// Amplitude of the detail in blocks.
    pub detail: f64,
    /// Base height by continentalness (−1..1).
    pub continentalness: Spline,
    /// Mountain factor by erosion (−1..1): high where erosion is low.
    pub erosion: Spline,
    /// Peak height in blocks by ridge value (0..1).
    pub ridges: Spline,
    /// Mountains fade in between these continentalness values (none in oceans).
    pub inland: [f64; 2],
    /// Temperature drop per block above the sea level (snowy peaks).
    pub height_cooling: f64,
}

/// Climate ranges a biome occupies (`climate` engine); the first biome in file
/// order whose ranges all hold is chosen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClimateRange {
    /// Temperature 0..1 (after height cooling).
    pub temperature: [f64; 2],
    /// Humidity 0..1.
    pub humidity: [f64; 2],
    /// Continentalness −1..1.
    pub continentalness: [f64; 2],
    /// Surface height in blocks.
    pub height: [i32; 2],
}

impl ClimateRange {
    /// Whether a point lies inside every range (inclusive).
    #[must_use]
    pub fn contains(
        &self,
        temperature: f64,
        humidity: f64,
        continentalness: f64,
        height: i32,
    ) -> bool {
        let inside = |r: [f64; 2], v: f64| r[0] <= v && v <= r[1];
        inside(self.temperature, temperature)
            && inside(self.humidity, humidity)
            && inside(self.continentalness, continentalness)
            && self.height[0] <= height
            && height <= self.height[1]
    }
}

/// How the landscape is shaped.
#[derive(Clone, Debug)]
pub enum Engine {
    /// One hills noise; biomes by temperature (`max_temperature`).
    Classic,
    /// Multi-noise climate shaping; biomes by [`ClimateRange`].
    Climate(Climate),
}

impl Engine {
    /// The engine's data name.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Classic => "classic",
            Self::Climate(_) => "climate",
        }
    }
}

/// Terrain shape and base blocks.
#[derive(Clone, Debug)]
pub struct Terrain {
    /// Mean surface y.
    pub base_height: i32,
    /// Hills amplitude in blocks.
    pub amplitude: f64,
    /// Small-scale detail amplitude in blocks.
    pub roughness: f64,
    /// Horizontal noise frequency.
    pub scale: f64,
    /// Bulk underground block.
    pub stone: BlockStateId,
    /// Fluid filling air below the sea level.
    pub fluid: BlockStateId,
}

/// Cave carving.
#[derive(Clone, Debug)]
pub struct Caves {
    /// Block used to carve.
    pub air: BlockStateId,
    /// Carve where |noise| exceeds this.
    pub threshold: f64,
    /// Noise frequency.
    pub scale: f64,
    /// Lowest carved y.
    pub min_y: i32,
    /// Keep this many blocks under the surface intact.
    pub max_below_surface: i32,
}

/// A biome: surface blocks and decoration density.
#[derive(Clone, Debug)]
pub struct Biome {
    /// Id from the data file.
    pub id: String,
    /// Chosen where the temperature noise (0..1) is at most this.
    pub max_temperature: f64,
    /// Surface block.
    pub top: BlockStateId,
    /// Blocks under the surface.
    pub filler: BlockStateId,
    /// Filler thickness.
    pub filler_depth: i32,
    /// Expected trees per column.
    pub trees_per_column: f64,
    /// Where the biome lies (`climate` engine only).
    pub climate: Option<ClimateRange>,
    /// Tree kinds that grow here (indices into `tree_kinds`), one picked per
    /// tree; empty: the default kind (`[trees]`).
    pub trees: Vec<u8>,
}

/// One ore vein kind.
#[derive(Clone, Debug)]
pub struct Ore {
    /// Ore block.
    pub block: BlockStateId,
    /// Only this block is replaced.
    pub replaces: BlockStateId,
    /// Veins per column.
    pub veins_per_column: u32,
    /// Blocks per vein (upper bound).
    pub vein_size: u32,
    /// Lowest vein origin y.
    pub min_y: i32,
    /// Highest vein origin y (exclusive).
    pub max_y: i32,
}

/// Trees.
#[derive(Clone, Debug)]
pub struct Trees {
    /// Trunk block.
    pub log: BlockStateId,
    /// Crown block.
    pub leaves: BlockStateId,
    /// Shortest trunk.
    pub min_trunk: i32,
    /// Tallest trunk.
    pub max_trunk: i32,
    /// Crown shape.
    pub shape: TreeShape,
}

/// Resolved overworld parameters.
#[derive(Clone, Debug)]
pub struct WorldgenConfig {
    /// How the landscape is shaped.
    pub engine: Engine,
    /// Fluid fills air at `y <= sea_level`.
    pub sea_level: i32,
    /// Terrain.
    pub terrain: Terrain,
    /// Caves.
    pub caves: Caves,
    /// Biomes: sorted by `max_temperature` (`classic`), in file order (`climate`).
    pub biomes: Vec<Biome>,
    /// Beach block.
    pub beach: BlockStateId,
    /// Beaches up to this many blocks above the sea level.
    pub beach_max_above_sea: i32,
    /// Ores.
    pub ores: Vec<Ore>,
    /// The default tree kind (`[trees]`).
    pub trees: Trees,
    /// Named tree kinds (`[[tree_kinds]]`); index 0 is the default `[trees]`.
    pub tree_kinds: Vec<Trees>,
}

impl WorldgenConfig {
    /// Parses a data file and resolves block names through `registry`.
    pub fn from_toml(source: &str, registry: &Registry) -> Result<Self, ConfigError> {
        let raw: RawConfig =
            toml::from_str(source).map_err(|e| ConfigError::Parse(e.to_string()))?;
        let id = |name: &str| {
            registry
                .state_id(name)
                .ok_or_else(|| ConfigError::UnknownBlock(name.to_owned()))
        };
        if raw.biomes.is_empty() {
            return Err(ConfigError::Invalid("biomes: at least one".into()));
        }
        if raw.biomes.len() > usize::from(u8::MAX) {
            return Err(ConfigError::Invalid("biomes: at most 255".into()));
        }
        let trunks = |what: &str, min: i32, max: i32| {
            if min < 1 || max < min || max > 32 {
                Err(ConfigError::Invalid(format!("{what}: trunk heights")))
            } else {
                Ok(())
            }
        };
        trunks("trees", raw.trees.min_trunk, raw.trees.max_trunk)?;
        let _ = raw.terrain.bedrock_depth;
        let trees = Trees {
            log: id(&raw.trees.log)?,
            leaves: id(&raw.trees.leaves)?,
            min_trunk: raw.trees.min_trunk,
            max_trunk: raw.trees.max_trunk,
            shape: raw.trees.shape,
        };
        let mut tree_kinds = vec![trees.clone()];
        let mut kind_names = vec![String::from("default")];
        for k in &raw.tree_kinds {
            trunks(&format!("tree_kinds.{}", k.id), k.min_trunk, k.max_trunk)?;
            if kind_names.contains(&k.id) {
                return Err(ConfigError::Invalid(format!(
                    "tree_kinds: duplicate {}",
                    k.id
                )));
            }
            kind_names.push(k.id.clone());
            tree_kinds.push(Trees {
                log: id(&k.log)?,
                leaves: id(&k.leaves)?,
                min_trunk: k.min_trunk,
                max_trunk: k.max_trunk,
                shape: k.shape,
            });
        }
        if tree_kinds.len() > usize::from(u8::MAX) {
            return Err(ConfigError::Invalid("tree_kinds: at most 254".into()));
        }
        let engine = match (raw.engine.as_str(), raw.climate) {
            ("classic", None) => Engine::Classic,
            ("classic", Some(_)) => {
                return Err(ConfigError::Invalid(
                    "[climate] is only for engine = \"climate\"".into(),
                ));
            }
            ("climate", Some(c)) => Engine::Climate(Climate {
                continentalness_scale: c.continentalness_scale,
                erosion_scale: c.erosion_scale,
                ridge_scale: c.ridge_scale,
                temperature_scale: c.temperature_scale,
                humidity_scale: c.humidity_scale,
                detail_scale: c.detail_scale,
                detail: c.detail,
                continentalness: Spline::new("continentalness", c.continentalness)?,
                erosion: Spline::new("erosion", c.erosion)?,
                ridges: Spline::new("ridges", c.ridges)?,
                inland: c.inland,
                height_cooling: c.height_cooling,
            }),
            ("climate", None) => {
                return Err(ConfigError::Invalid(
                    "engine = \"climate\" needs a [climate] table".into(),
                ));
            }
            (other, _) => {
                return Err(ConfigError::Invalid(format!(
                    "engine {other:?} (known: classic, climate)"
                )));
            }
        };
        let climate_engine = matches!(engine, Engine::Climate(_));
        let mut biomes = raw
            .biomes
            .iter()
            .map(|b| {
                let climate = match (&b.climate, climate_engine) {
                    (Some(r), true) => Some(ClimateRange {
                        temperature: r.temperature.unwrap_or([f64::MIN, f64::MAX]),
                        humidity: r.humidity.unwrap_or([f64::MIN, f64::MAX]),
                        continentalness: r.continentalness.unwrap_or([f64::MIN, f64::MAX]),
                        height: r.height.unwrap_or([i32::MIN, i32::MAX]),
                    }),
                    (None, true) => None, // matches everything: a catch-all biome
                    (Some(_), false) => {
                        return Err(ConfigError::Invalid(format!(
                            "biomes.{}: climate ranges need engine = \"climate\"",
                            b.id
                        )));
                    }
                    (None, false) => None,
                };
                let trees = b
                    .trees
                    .iter()
                    .map(|t| {
                        kind_names
                            .iter()
                            .position(|k| k == t)
                            .map(|i| i as u8)
                            .ok_or_else(|| {
                                ConfigError::Invalid(format!("biomes.{}: tree kind {t:?}", b.id))
                            })
                    })
                    .collect::<Result<Vec<_>, ConfigError>>()?;
                Ok(Biome {
                    id: b.id.clone(),
                    max_temperature: b.max_temperature,
                    top: id(&b.top)?,
                    filler: id(&b.filler)?,
                    filler_depth: b.filler_depth.max(0),
                    trees_per_column: b.trees_per_column.max(0.0),
                    climate,
                    trees,
                })
            })
            .collect::<Result<Vec<_>, ConfigError>>()?;
        if !climate_engine {
            biomes.sort_by(|a, b| a.max_temperature.total_cmp(&b.max_temperature));
        }
        Ok(Self {
            engine,
            sea_level: raw.sea_level,
            terrain: Terrain {
                base_height: raw.terrain.base_height,
                amplitude: raw.terrain.amplitude,
                roughness: raw.terrain.roughness,
                scale: raw.terrain.scale,
                stone: id(&raw.terrain.stone)?,
                fluid: id(&raw.terrain.fluid)?,
            },
            caves: Caves {
                air: id(&raw.caves.air)?,
                threshold: raw.caves.threshold,
                scale: raw.caves.scale,
                min_y: raw.caves.min_y,
                max_below_surface: raw.caves.max_below_surface,
            },
            biomes,
            beach: id(&raw.beach.block)?,
            beach_max_above_sea: raw.beach.max_above_sea,
            ores: raw
                .ores
                .iter()
                .map(|o| {
                    Ok(Ore {
                        block: id(&o.block)?,
                        replaces: id(&o.replaces)?,
                        veins_per_column: o.veins_per_column,
                        vein_size: o.vein_size.max(1),
                        min_y: o.min_y,
                        max_y: o.max_y.max(o.min_y + 1),
                    })
                })
                .collect::<Result<Vec<_>, ConfigError>>()?,
            trees,
            tree_kinds,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spline_interpolates_and_clamps() {
        let s = Spline::new("t", vec![[1.0, 10.0], [-1.0, 0.0], [0.0, 4.0]]).expect("spline");
        assert_eq!(s.at(-5.0), 0.0);
        assert_eq!(s.at(-1.0), 0.0);
        assert_eq!(s.at(-0.5), 2.0);
        assert_eq!(s.at(0.0), 4.0);
        assert_eq!(s.at(0.5), 7.0);
        assert_eq!(s.at(9.0), 10.0);
        assert!(Spline::new("t", vec![]).is_err());
        assert!(Spline::new("t", vec![[0.0, f64::NAN]]).is_err());
    }

    #[test]
    fn climate_ranges_are_inclusive() {
        let r = ClimateRange {
            temperature: [0.2, 0.4],
            humidity: [f64::MIN, f64::MAX],
            continentalness: [-1.0, 0.0],
            height: [60, 100],
        };
        assert!(r.contains(0.2, 0.9, -1.0, 60));
        assert!(r.contains(0.4, 0.0, 0.0, 100));
        assert!(!r.contains(0.41, 0.5, -0.5, 80));
        assert!(!r.contains(0.3, 0.5, 0.1, 80));
        assert!(!r.contains(0.3, 0.5, -0.5, 101));
    }
}
