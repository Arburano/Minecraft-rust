//! Generation parameters from mod data (`<mod>/data/worldgen/*.toml`), resolved
//! against the block registry. The engine knows no blocks: every block the
//! built-in stages place is named here by mod data.

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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    sea_level: i32,
    terrain: RawTerrain,
    caves: RawCaves,
    biomes: Vec<RawBiome>,
    beach: RawBeach,
    #[serde(default)]
    ores: Vec<RawOre>,
    trees: RawTrees,
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
    max_temperature: f64,
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
}

/// Resolved overworld parameters.
#[derive(Clone, Debug)]
pub struct WorldgenConfig {
    /// Fluid fills air at `y <= sea_level`.
    pub sea_level: i32,
    /// Terrain.
    pub terrain: Terrain,
    /// Caves.
    pub caves: Caves,
    /// Biomes sorted by `max_temperature`.
    pub biomes: Vec<Biome>,
    /// Beach block.
    pub beach: BlockStateId,
    /// Beaches up to this many blocks above the sea level.
    pub beach_max_above_sea: i32,
    /// Ores.
    pub ores: Vec<Ore>,
    /// Trees.
    pub trees: Trees,
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
        if raw.trees.min_trunk < 1 || raw.trees.max_trunk < raw.trees.min_trunk {
            return Err(ConfigError::Invalid("trees: trunk heights".into()));
        }
        let _ = raw.terrain.bedrock_depth;
        let mut biomes = raw
            .biomes
            .iter()
            .map(|b| {
                Ok(Biome {
                    id: b.id.clone(),
                    max_temperature: b.max_temperature,
                    top: id(&b.top)?,
                    filler: id(&b.filler)?,
                    filler_depth: b.filler_depth.max(0),
                    trees_per_column: b.trees_per_column.max(0.0),
                })
            })
            .collect::<Result<Vec<_>, ConfigError>>()?;
        biomes.sort_by(|a, b| a.max_temperature.total_cmp(&b.max_temperature));
        Ok(Self {
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
            trees: Trees {
                log: id(&raw.trees.log)?,
                leaves: id(&raw.trees.leaves)?,
                min_trunk: raw.trees.min_trunk,
                max_trunk: raw.trees.max_trunk,
            },
        })
    }
}
