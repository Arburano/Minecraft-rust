//! Dimension definitions (T4-02, `docs/architecture/dimensions.md`): what a
//! world module declares about each world it adds — generator file, height,
//! sky, physics. Loaded from the build's mods; the dimension's generation
//! file comes from the module that fills its `worldgen:<id>` slot.

use std::collections::BTreeMap;
use std::fmt;

use serde::Deserialize;

use crate::WorldBounds;
use crate::modset::{ModSet, worldgen_slot};

/// The dimension every build starts players in.
pub const OVERWORLD: &str = "overworld";

/// How the sky is drawn (the renderer's choice; the engine only passes it on).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sky {
    /// Sun, moon and a day-night cycle.
    #[default]
    DayNight,
    /// No sky (caves, the nether).
    None,
    /// A fixed light (the end, space).
    Fixed,
}

/// Physics of a dimension.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct DimensionPhysics {
    /// m/s², for grids and other rigid bodies.
    pub gravity: [f64; 3],
    /// Multiplies the movement profile's gravity (0.38 for a Mars-like planet).
    pub player_gravity_scale: f64,
    /// Air pressure at the sea level; 0 is vacuum.
    pub pressure: f64,
    /// Pressure by altitude: `[y, pressure]` points (empty: constant).
    pub pressure_by_altitude: Vec<[f64; 2]>,
    /// Flat drag on all motion.
    pub drag: f64,
    /// For buoyancy of grids.
    pub fluid_density: f64,
}

impl Default for DimensionPhysics {
    fn default() -> Self {
        Self {
            gravity: [0.0, -9.8, 0.0],
            player_gravity_scale: 1.0,
            pressure: 1.0,
            pressure_by_altitude: Vec::new(),
            drag: 0.09,
            fluid_density: 1.0,
        }
    }
}

impl DimensionPhysics {
    /// Air pressure at height `y` (piecewise linear between the points, flat
    /// beyond them; the sea-level value without points).
    #[must_use]
    pub fn pressure_at(&self, y: f64) -> f64 {
        let p = &self.pressure_by_altitude;
        let Some(first) = p.first() else {
            return self.pressure;
        };
        let i = p.partition_point(|q| q[0] <= y);
        if i == 0 {
            return first[1];
        }
        if i == p.len() {
            return p[p.len() - 1][1];
        }
        let ([x0, y0], [x1, y1]) = (p[i - 1], p[i]);
        y0 + (y1 - y0) * (y - x0) / (x1 - x0).max(f64::MIN_POSITIVE)
    }
}

/// `data/dimensions/<id>.toml`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DimensionDef {
    /// Id (`[a-z0-9_]`), also the storage folder name.
    pub id: String,
    /// The generation file under the module's `data/`, e.g.
    /// `worldgen/overworld.toml`.
    pub generator: String,
    /// Lowest and highest block y (exclusive top), multiples of 16.
    #[serde(default = "default_height")]
    pub height: [i32; 2],
    /// How the sky is drawn.
    #[serde(default)]
    pub sky: Sky,
    /// Physics.
    #[serde(default)]
    pub physics: DimensionPhysics,
}

fn default_height() -> [i32; 2] {
    [-64, 320]
}

/// A dimension definition is invalid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DimensionError(pub String);

impl fmt::Display for DimensionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for DimensionError {}

impl DimensionDef {
    /// Parses and checks a definition.
    pub fn from_toml(source: &str) -> Result<Self, DimensionError> {
        let d: Self = toml::from_str(source).map_err(|e| DimensionError(e.to_string()))?;
        d.check()?;
        Ok(d)
    }

    fn check(&self) -> Result<(), DimensionError> {
        let bad = |m: String| Err(DimensionError(format!("dimension {}: {m}", self.id)));
        if self.id.is_empty()
            || !self
                .id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            return bad("id must be [a-z0-9_]".into());
        }
        let [lo, hi] = self.height;
        if lo % 16 != 0 || hi % 16 != 0 || hi <= lo || hi - lo > 4096 {
            return bad("height must be multiples of 16, low < high, at most 4096 blocks".into());
        }
        let p = &self.physics;
        if !(0.0..=100.0).contains(&p.player_gravity_scale) || p.pressure < 0.0 || p.drag < 0.0 {
            return bad("physics values out of range".into());
        }
        if p.pressure_by_altitude
            .windows(2)
            .any(|w| w[1][0] <= w[0][0])
        {
            return bad("pressure_by_altitude heights must increase".into());
        }
        if self.generator.contains("..") || !self.generator.ends_with(".toml") {
            return bad("generator must be a .toml file inside the module".into());
        }
        Ok(())
    }

    /// The world bounds of this dimension.
    #[must_use]
    pub fn bounds(&self) -> WorldBounds {
        WorldBounds {
            min_section_y: self.height[0] / 16,
            section_count: ((self.height[1] - self.height[0]) / 16) as u32,
        }
    }

    /// The generation slot of this dimension (`worldgen:<id>`).
    #[must_use]
    pub fn slot(&self) -> String {
        worldgen_slot(&self.id)
    }
}

/// The dimensions of a build: every `data/dimensions/*.toml` of its mods,
/// keyed by id. Each must have a provider for its generation slot; a later
/// mod may not redefine an id (one owner per dimension).
pub fn from_mods(mods: &ModSet) -> Result<BTreeMap<String, DimensionDef>, DimensionError> {
    let mut out: BTreeMap<String, (DimensionDef, String)> = BTreeMap::new();
    for m in mods.mods() {
        let dir = m.data_dir().join("dimensions");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut files: Vec<_> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "toml"))
            .collect();
        files.sort();
        for f in files {
            let text = std::fs::read_to_string(&f)
                .map_err(|e| DimensionError(format!("{}: {e}", f.display())))?;
            let d = DimensionDef::from_toml(&text)
                .map_err(|e| DimensionError(format!("{}: {e}", f.display())))?;
            if let Some((_, other)) = out.get(&d.id) {
                return Err(DimensionError(format!(
                    "dimension {} is defined by both {other} and {}",
                    d.id, m.id
                )));
            }
            out.insert(d.id.clone(), (d, m.id.clone()));
        }
    }
    for (id, (d, owner)) in &out {
        let Some(provider) = mods.slot(&d.slot()) else {
            return Err(DimensionError(format!(
                "dimension {id} (from {owner}) has no generator: no mod of the build provides {:?}",
                d.slot()
            )));
        };
        if !provider.data_dir().join(&d.generator).is_file() {
            return Err(DimensionError(format!(
                "dimension {id}: {} provides {:?} but has no data/{}",
                provider.id,
                d.slot(),
                d.generator
            )));
        }
    }
    Ok(out.into_iter().map(|(k, (d, _))| (k, d)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_documented_format_parses() {
        let d = DimensionDef::from_toml(
            r#"
id = "overworld"
generator = "worldgen/overworld.toml"
height = [-64, 320]
sky = "day_night"
[physics]
gravity = [0.0, -9.8, 0.0]
player_gravity_scale = 1.0
pressure = 1.0
pressure_by_altitude = [[62, 1.0], [200, 0.6], [320, 0.3]]
drag = 0.09
fluid_density = 1.0
"#,
        )
        .expect("parses");
        assert_eq!(d.bounds(), WorldBounds::default());
        assert_eq!(d.slot(), "worldgen:overworld");
        assert_eq!(d.physics.pressure_at(0.0), 1.0);
        assert!((d.physics.pressure_at(131.0) - 0.8).abs() < 1e-9);
        assert_eq!(d.physics.pressure_at(1000.0), 0.3);
    }

    #[test]
    fn bad_definitions_are_refused() {
        let base = "id = \"mars\"\ngenerator = \"worldgen/mars.toml\"\n";
        assert!(DimensionDef::from_toml(base).is_ok());
        let bad = |extra: &str| DimensionDef::from_toml(&format!("{base}{extra}")).is_err();
        assert!(bad("height = [0, 100]\n"));
        assert!(bad("height = [64, 0]\n"));
        assert!(bad("sky = \"rainbow\"\n"));
        assert!(bad("[physics]\npressure = -1.0\n"));
        assert!(bad(
            "[physics]\npressure_by_altitude = [[10, 1.0], [5, 0.5]]\n"
        ));
        assert!(
            DimensionDef::from_toml("id = \"Mars\"\ngenerator = \"worldgen/m.toml\"\n").is_err()
        );
        assert!(DimensionDef::from_toml("id = \"m\"\ngenerator = \"../x.toml\"\n").is_err());
        // Defaults: Earth-like physics.
        let d = DimensionDef::from_toml(base).expect("ok");
        assert_eq!(d.physics, DimensionPhysics::default());
        assert_eq!(d.sky, Sky::DayNight);
    }

    #[test]
    fn shipped_builds_define_their_dimensions() {
        for build in ["classic", "classic_plus", "classic_agile"] {
            let mods = crate::modset::source_build(build).expect("build");
            let dims = from_mods(&mods).unwrap_or_else(|e| panic!("{build}: {e}"));
            assert_eq!(dims.keys().collect::<Vec<_>>(), [OVERWORLD], "{build}");
            assert_eq!(dims[OVERWORLD].bounds(), WorldBounds::default());
        }
    }
}
