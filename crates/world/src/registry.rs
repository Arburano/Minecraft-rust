//! Block-state registry and the persisted ID table (`world-storage.md`,
//! "Registries and the ID table"; datapack format: `docs/modding/datapacks.md`).
//!
//! The engine knows no blocks: definitions come from mod data. The registry is
//! built once at startup, then shared immutably (`Arc<Registry>`), so lookups
//! need no locks.

use std::collections::HashMap;
use std::fmt;

use serde::Deserialize;

use crate::BlockStateId;

/// Name of the air block. Air is always state 0.
pub const AIR_NAME: &str = "core:air";
/// Name of the placeholder for states missing from the current mod set.
pub const MISSING_NAME: &str = "core:missing";
/// Maximum light level.
const MAX_LIGHT: u8 = 15;

/// How a block collides with entities.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Collision {
    /// A full cube.
    #[default]
    Full,
    /// No collision (air, flowers).
    None,
    /// The model's own shape (resolved by the physics code).
    Model,
}

/// Engine-relevant properties of a block state (identical for all states of a
/// block in this version).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BlockProps {
    /// Hides neighbor faces; goes into the opaque meshing mask.
    pub opaque: bool,
    /// Goes into the transparent meshing mask (glass, water, leaves).
    pub transparent: bool,
    /// Emitted block light, `0..=15`.
    pub light_emission: u8,
    /// Light attenuation, `0..=15` (`lighting.md`); anything above 0 stops direct sky.
    pub light_opacity: u8,
    /// Collision shape.
    pub collision: Collision,
}

impl BlockProps {
    /// Air: invisible, no collision, no attenuation.
    pub const AIR: Self = Self {
        opaque: false,
        transparent: false,
        light_emission: 0,
        light_opacity: 0,
        collision: Collision::None,
    };
    /// The placeholder for missing states: a visible full block.
    pub const MISSING: Self = Self {
        opaque: true,
        transparent: false,
        light_emission: 0,
        light_opacity: MAX_LIGHT,
        collision: Collision::Full,
    };
}

/// One block definition from a datapack (`<mod>/data/blocks/*.toml`).
#[derive(Clone, Debug, PartialEq)]
pub struct BlockDef {
    /// `namespace:path`.
    pub id: String,
    /// State properties in declaration order; every combination is one state.
    pub states: Vec<(String, Vec<String>)>,
    /// Engine-relevant properties.
    pub props: BlockProps,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBlockDef {
    id: String,
    #[serde(default)]
    opaque: bool,
    #[serde(default)]
    transparent: bool,
    #[serde(default)]
    light_emission: u8,
    light_opacity: Option<u8>,
    #[serde(default)]
    collision: Collision,
    #[serde(default)]
    states: Option<toml::Table>,
    // Fields used by other subsystems (assets, gameplay); accepted and ignored here.
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    textures: Option<toml::Table>,
    #[serde(default)]
    tint: Option<toml::Table>,
    #[serde(default)]
    hardness: Option<f64>,
    #[serde(default)]
    tool: Option<String>,
    #[serde(default)]
    components: Option<Vec<String>>,
    #[serde(default)]
    tick: Option<String>,
}

impl BlockDef {
    /// A stateless block.
    #[must_use]
    pub fn simple(id: &str, props: BlockProps) -> Self {
        Self {
            id: id.to_owned(),
            states: Vec::new(),
            props,
        }
    }

    /// Parses one block file. `light_opacity` defaults to 15 for opaque blocks
    /// and 0 otherwise; values above 15 are errors.
    pub fn from_toml(source: &str) -> Result<Self, RegistryError> {
        let raw: RawBlockDef =
            toml::from_str(source).map_err(|e| RegistryError::Parse(e.to_string()))?;
        let _ = (
            &raw.model,
            &raw.textures,
            &raw.tint,
            &raw.hardness,
            &raw.tool,
            &raw.components,
            &raw.tick,
        );
        check_name(&raw.id)?;
        let light_opacity = raw
            .light_opacity
            .unwrap_or(if raw.opaque { MAX_LIGHT } else { 0 });
        if raw.light_emission > MAX_LIGHT || light_opacity > MAX_LIGHT {
            return Err(RegistryError::LightOutOfRange(raw.id));
        }
        let mut states = Vec::new();
        if let Some(table) = raw.states {
            // toml::Table keeps keys sorted, which is also our canonical order.
            for (name, values) in table {
                let values = values
                    .as_array()
                    .ok_or_else(|| RegistryError::BadState(raw.id.clone(), name.clone()))?
                    .iter()
                    .map(|v| match v {
                        toml::Value::String(s) => Some(s.clone()),
                        toml::Value::Boolean(b) => Some(b.to_string()),
                        toml::Value::Integer(i) => Some(i.to_string()),
                        _ => None,
                    })
                    .collect::<Option<Vec<_>>>()
                    .filter(|v| !v.is_empty())
                    .ok_or_else(|| RegistryError::BadState(raw.id.clone(), name.clone()))?;
                states.push((name, values));
            }
        }
        Ok(Self {
            id: raw.id,
            states,
            props: BlockProps {
                opaque: raw.opaque,
                transparent: raw.transparent,
                light_emission: raw.light_emission,
                light_opacity,
                collision: raw.collision,
            },
        })
    }
}

/// Registry construction errors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegistryError {
    /// A datapack file is not valid TOML or does not match the schema.
    Parse(String),
    /// An ID is not `namespace:path` (lowercase, digits, `_`, `-`, `.`, `/`).
    BadName(String),
    /// Two definitions share an ID.
    Duplicate(String),
    /// A state property has no values or a value of an unsupported type.
    BadState(String, String),
    /// Light emission or opacity above 15.
    LightOutOfRange(String),
    /// More states than fit into a `u16` `BlockStateId`.
    TooManyStates,
    /// A datapack directory or file could not be read.
    Io(String),
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(e) => write!(f, "block definition: {e}"),
            Self::BadName(n) => write!(f, "invalid ID {n:?}: expected namespace:path"),
            Self::Duplicate(n) => write!(f, "block {n:?} is defined twice"),
            Self::BadState(b, s) => write!(f, "block {b:?}: invalid state property {s:?}"),
            Self::LightOutOfRange(b) => write!(f, "block {b:?}: light values must be 0..=15"),
            Self::TooManyStates => write!(f, "more than 65536 block states"),
            Self::Io(e) => write!(f, "reading block definitions: {e}"),
        }
    }
}

impl std::error::Error for RegistryError {}

fn check_name(name: &str) -> Result<(), RegistryError> {
    let ok_part = |p: &str| {
        !p.is_empty()
            && p.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_-./".contains(&b))
    };
    match name.split_once(':') {
        Some((ns, path)) if ok_part(ns) && ok_part(path) && !path.contains(':') => Ok(()),
        _ => Err(RegistryError::BadName(name.to_owned())),
    }
}

/// Reads every `*.toml` in a mod's `data/blocks` directory. Files are taken in
/// file-name order, which is the mod's declaration order. A parse error names
/// the file.
pub fn blocks_from_dir(dir: &std::path::Path) -> Result<Vec<BlockDef>, RegistryError> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| RegistryError::Io(format!("{}: {e}", dir.display())))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .collect();
    files.sort();
    files
        .iter()
        .map(|path| {
            let source = std::fs::read_to_string(path)
                .map_err(|e| RegistryError::Io(format!("{}: {e}", path.display())))?;
            BlockDef::from_toml(&source).map_err(|e| match e {
                RegistryError::Parse(msg) => {
                    RegistryError::Parse(format!("{}: {msg}", path.display()))
                }
                other => other,
            })
        })
        .collect()
}

/// Collects block definitions from mods and assigns numeric IDs.
#[derive(Default)]
pub struct RegistryBuilder {
    mods: Vec<(String, Vec<BlockDef>)>,
}

impl RegistryBuilder {
    /// An empty builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a mod's blocks in declaration order. Mods may be added in any order.
    pub fn add_mod(&mut self, mod_id: &str, blocks: Vec<BlockDef>) -> &mut Self {
        self.mods.push((mod_id.to_owned(), blocks));
        self
    }

    /// Builds the registry: `core:air` = 0, `core:missing` = 1 (both provided by
    /// the engine unless a mod defines them), then mods sorted by id, blocks in
    /// declaration order, states in canonical order (properties sorted by name,
    /// the last property varying fastest).
    pub fn build(mut self) -> Result<Registry, RegistryError> {
        self.mods.sort_by(|a, b| a.0.cmp(&b.0));
        let mut defs: Vec<BlockDef> = Vec::new();
        let mut air = BlockDef::simple(AIR_NAME, BlockProps::AIR);
        let mut missing = BlockDef::simple(MISSING_NAME, BlockProps::MISSING);
        let mut seen: HashMap<String, ()> = HashMap::new();
        for (_, blocks) in self.mods {
            for def in blocks {
                check_name(&def.id)?;
                if seen.insert(def.id.clone(), ()).is_some() {
                    return Err(RegistryError::Duplicate(def.id));
                }
                match def.id.as_str() {
                    AIR_NAME => air = def,
                    MISSING_NAME => missing = def,
                    _ => defs.push(def),
                }
            }
        }
        if !air.states.is_empty() {
            return Err(RegistryError::BadState(
                AIR_NAME.into(),
                "air has no states".into(),
            ));
        }
        let mut registry = Registry {
            entries: Vec::new(),
            props: Vec::new(),
            by_name: HashMap::new(),
        };
        for def in std::iter::once(air)
            .chain(std::iter::once(missing))
            .chain(defs)
        {
            registry.push_block(&def)?;
        }
        Ok(registry)
    }
}

/// The immutable block-state registry of a session.
#[derive(Debug)]
pub struct Registry {
    entries: Vec<String>,
    props: Vec<BlockProps>,
    by_name: HashMap<String, BlockStateId>,
}

impl Registry {
    fn push_block(&mut self, def: &BlockDef) -> Result<(), RegistryError> {
        let total = def
            .states
            .iter()
            .try_fold(1usize, |acc, (_, values)| acc.checked_mul(values.len()))
            .filter(|&t| self.entries.len() + t <= usize::from(u16::MAX) + 1)
            .ok_or(RegistryError::TooManyStates)?;
        for n in 0..total {
            let name = if def.states.is_empty() {
                def.id.clone()
            } else {
                // Mixed-radix decode of `n`: the last property varies fastest.
                let mut rem = n;
                let mut parts = vec![String::new(); def.states.len()];
                for (part, (key, values)) in parts.iter_mut().zip(&def.states).rev() {
                    *part = format!("{key}={}", values[rem % values.len()]);
                    rem /= values.len();
                }
                format!("{}[{}]", def.id, parts.join(","))
            };
            let id = u16::try_from(self.entries.len()).map_err(|_| RegistryError::TooManyStates)?;
            self.by_name.insert(name.clone(), BlockStateId(id));
            self.entries.push(name);
            self.props.push(def.props);
        }
        Ok(())
    }

    /// Number of block states.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Always false: air and the missing placeholder always exist.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether `id` is a valid state of this registry.
    #[must_use]
    pub fn contains(&self, id: BlockStateId) -> bool {
        usize::from(id.0) < self.entries.len()
    }

    /// Properties of a state; unknown IDs get the missing placeholder's.
    #[inline]
    #[must_use]
    pub fn props(&self, id: BlockStateId) -> BlockProps {
        self.props
            .get(usize::from(id.0))
            .copied()
            .unwrap_or(BlockProps::MISSING)
    }

    /// Canonical string ID of a state, e.g. `mod:pipe[facing=north,waterlogged=false]`.
    #[must_use]
    pub fn name(&self, id: BlockStateId) -> Option<&str> {
        self.entries.get(usize::from(id.0)).map(String::as_str)
    }

    /// Numeric ID of a canonical state name.
    #[must_use]
    pub fn state_id(&self, name: &str) -> Option<BlockStateId> {
        self.by_name.get(name).copied()
    }

    /// The placeholder for states missing from the current mod set.
    #[must_use]
    pub fn missing(&self) -> BlockStateId {
        self.state_id(MISSING_NAME).unwrap_or(BlockStateId(1))
    }

    /// The `string → number` table to persist with saved sections.
    #[must_use]
    pub fn id_table(&self) -> IdTable {
        IdTable {
            names: self.entries.clone(),
        }
    }

    /// Maps numeric IDs of a saved table to this registry: `out[saved] = current`.
    /// States the current mod set lacks become [`Self::missing`].
    #[must_use]
    pub fn remap(&self, saved: &IdTable) -> Vec<BlockStateId> {
        let missing = self.missing();
        saved
            .names
            .iter()
            .map(|n| self.state_id(n).unwrap_or(missing))
            .collect()
    }
}

/// A persisted `string state ID → number` table; index = numeric ID.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IdTable {
    /// Canonical state names by numeric ID.
    pub names: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stone() -> BlockDef {
        BlockDef::simple("core:stone", BlockProps::MISSING)
    }

    #[test]
    fn air_is_zero_and_order_is_deterministic() {
        let pipe = BlockDef {
            id: "pipes:pipe".into(),
            states: vec![
                ("facing".into(), vec!["north".into(), "south".into()]),
                ("wet".into(), vec!["false".into(), "true".into()]),
            ],
            props: BlockProps::AIR,
        };
        let mut a = RegistryBuilder::new();
        a.add_mod("pipes", vec![pipe.clone()])
            .add_mod("core", vec![stone()]);
        let mut b = RegistryBuilder::new();
        b.add_mod("core", vec![stone()])
            .add_mod("pipes", vec![pipe]);
        let (a, b) = (a.build().expect("valid"), b.build().expect("valid"));
        assert_eq!(a.id_table(), b.id_table());
        assert_eq!(a.name(BlockStateId::AIR), Some(AIR_NAME));
        assert_eq!(a.name(BlockStateId(1)), Some(MISSING_NAME));
        assert_eq!(a.name(BlockStateId(2)), Some("core:stone"));
        assert_eq!(
            a.name(BlockStateId(3)),
            Some("pipes:pipe[facing=north,wet=false]")
        );
        assert_eq!(
            a.name(BlockStateId(4)),
            Some("pipes:pipe[facing=north,wet=true]")
        );
        assert_eq!(
            a.name(BlockStateId(6)),
            Some("pipes:pipe[facing=south,wet=true]")
        );
        assert_eq!(a.len(), 7);
        assert_eq!(a.props(BlockStateId(999)), BlockProps::MISSING);
    }

    #[test]
    fn remap_uses_missing_for_unknown_states() {
        let mut b = RegistryBuilder::new();
        b.add_mod("core", vec![stone()]);
        let reg = b.build().expect("valid");
        let saved = IdTable {
            names: vec![AIR_NAME.into(), "gone:block".into(), "core:stone".into()],
        };
        assert_eq!(
            reg.remap(&saved),
            vec![BlockStateId::AIR, reg.missing(), BlockStateId(2)]
        );
    }

    #[test]
    fn rejects_bad_input() {
        let mut b = RegistryBuilder::new();
        b.add_mod("core", vec![stone(), stone()]);
        assert_eq!(
            b.build().err(),
            Some(RegistryError::Duplicate("core:stone".into()))
        );
        let mut b = RegistryBuilder::new();
        b.add_mod("x", vec![BlockDef::simple("Bad Name", BlockProps::AIR)]);
        assert!(matches!(b.build(), Err(RegistryError::BadName(_))));
        assert!(matches!(
            BlockDef::from_toml("id = \"a:b\"\nlight_emission = 16"),
            Err(RegistryError::LightOutOfRange(_))
        ));
        assert!(matches!(
            BlockDef::from_toml("id = \"a:b\"\nunknown = 1"),
            Err(RegistryError::Parse(_))
        ));
        let many = BlockDef {
            id: "x:many".into(),
            states: (0..4)
                .map(|i| (format!("p{i}"), (0..17).map(|v| v.to_string()).collect()))
                .collect(),
            props: BlockProps::AIR,
        };
        let mut b = RegistryBuilder::new();
        b.add_mod("x", vec![many]);
        assert_eq!(b.build().err(), Some(RegistryError::TooManyStates));
    }

    #[test]
    fn parses_datapack_block() {
        let def = BlockDef::from_toml(
            r#"
            id = "copper_pipes:copper_pipe"
            model = "copper_pipes:models/pipe"
            textures = { all = "copper_pipes:block/copper_pipe" }
            hardness = 2.5
            tool = "pickaxe"
            opaque = false
            transparent = true
            light_emission = 0
            collision = "model"
            states = { facing = ["north", "south"], waterlogged = [false, true] }
            components = ["copper_pipes:flow"]
            tick = "none"
            "#,
        )
        .expect("valid block");
        assert_eq!(def.props.collision, Collision::Model);
        assert_eq!(def.props.light_opacity, 0);
        assert_eq!(
            def.states[1],
            ("waterlogged".into(), vec!["false".into(), "true".into()])
        );
        let stone = BlockDef::from_toml("id = \"core:stone\"\nopaque = true").expect("valid");
        assert_eq!(stone.props.light_opacity, 15);
    }
}
