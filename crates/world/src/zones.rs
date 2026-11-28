//! Game zones: named world areas with rules (PRD §14 No. 11, capability
//! `zones`, ADR-0018, `docs/modding/zones.md`).
//!
//! A zone is a static map layout, not a threading region: a list of boxes
//! with rules for building, entry and PvP, a spawn table and a difficulty.
//! Zones come from mod data (`data/zones/*.toml`) and from the world
//! (`<world>/zones/zones.toml`, written by the server's zone API).
//!
//! [`ZoneIndex`] answers "which rules hold at this block" for the server's
//! hot paths: block edits, player movement, spawning and combat. It is built
//! once (on the world thread, between ticks) and only read afterwards, so
//! regions share it without locks.
//!
//! Overlapping zones: the zone with the higher `priority` decides; equal
//! priorities are ordered by id. Each rule is resolved on its own: a zone
//! that does not set a rule leaves it to the zones below it, and where no
//! zone sets it the default is "allowed".

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::hash::{BuildHasherDefault, Hasher};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::ModSet;
use crate::coords::BlockPos;

/// Edge of one index tile in blocks (a power of two).
pub const TILE_SHIFT: u32 = 6;
/// An area covering more tiles than this is checked on every lookup instead
/// of being entered into each tile (a zone spanning the whole map).
pub const MAX_TILES_PER_AREA: u64 = 1 << 16;
/// Most zones one index holds.
pub const MAX_ZONES: usize = 4096;
/// Most areas one zone holds.
pub const MAX_AREAS: usize = 256;

/// An error in a zone definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneError(pub String);

impl fmt::Display for ZoneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ZoneError {}

/// A box of blocks; every range is inclusive at both ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Area {
    /// East-west range.
    pub x: [i32; 2],
    /// North-south range.
    pub z: [i32; 2],
    /// Height range; the whole height when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<[i32; 2]>,
}

impl Area {
    /// Whether the box holds the block.
    #[inline]
    #[must_use]
    pub fn contains(&self, p: BlockPos) -> bool {
        let [y0, y1] = self.y.unwrap_or([i32::MIN, i32::MAX]);
        p.x >= self.x[0]
            && p.x <= self.x[1]
            && p.z >= self.z[0]
            && p.z <= self.z[1]
            && p.y >= y0
            && p.y <= y1
    }

    fn validate(&self) -> Result<(), String> {
        let bad = |r: [i32; 2]| r[0] > r[1];
        if bad(self.x) || bad(self.z) || self.y.is_some_and(bad) {
            return Err(format!(
                "area {self:?} has a range whose start is above its end"
            ));
        }
        Ok(())
    }

    fn tiles(&self) -> ([i32; 2], [i32; 2], u64) {
        let tx = [self.x[0] >> TILE_SHIFT, self.x[1] >> TILE_SHIFT];
        let tz = [self.z[0] >> TILE_SHIFT, self.z[1] >> TILE_SHIFT];
        let n = (i64::from(tx[1]) - i64::from(tx[0]) + 1) as u64
            * (i64::from(tz[1]) - i64::from(tz[0]) + 1) as u64;
        (tx, tz, n)
    }
}

/// The rules a zone sets; `None` leaves a rule to the zones below.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ZoneRules {
    /// Whether players may break and place blocks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<bool>,
    /// Whether players may walk in (players already inside may move and
    /// leave).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<bool>,
    /// Whether players may hurt each other.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pvp: Option<bool>,
}

/// One entry of a spawn table.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpawnEntry {
    /// Entity kind (`mod:name`).
    pub entity: String,
    /// Relative weight (> 0).
    pub weight: u32,
    /// Group size range, inclusive.
    #[serde(default = "one_one")]
    pub group: [u32; 2],
    /// Block light range where it may spawn, inclusive; any light when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub light: Option<[u8; 2]>,
}

fn one_one() -> [u32; 2] {
    [1, 1]
}

/// A zone as written in `data/zones/*.toml` and in the world's zone file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ZoneDef {
    /// `namespace:name`; in mod data the namespace is the mod id.
    pub id: String,
    /// Display name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// The higher priority decides where zones overlap.
    #[serde(default)]
    pub priority: i32,
    /// Difficulty for the build's content (spawning, loot); 0 when absent.
    #[serde(default)]
    pub difficulty: u8,
    /// The boxes the zone covers.
    pub areas: Vec<Area>,
    /// Players (by name) for whom a denied `build` or `entry` does not hold.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<String>,
    /// Rules.
    #[serde(default)]
    pub rules: ZoneRules,
    /// Spawn table; absent leaves spawning to the zones below, an empty list
    /// means nothing spawns here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawns: Option<Vec<SpawnEntry>>,
    /// Lets a later mod replace a zone of a mod it depends on.
    #[serde(default, rename = "override", skip_serializing_if = "is_false")]
    pub replaces: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn valid_id(id: &str) -> bool {
    let ok = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    };
    id.split_once(':')
        .is_some_and(|(ns, name)| ok(ns) && ok(name))
}

impl ZoneDef {
    /// Parses one zone (the format of `data/zones/*.toml`).
    pub fn from_toml(text: &str) -> Result<Self, ZoneError> {
        let def: Self = toml::from_str(text).map_err(|e| ZoneError(e.to_string()))?;
        def.validate()?;
        Ok(def)
    }

    /// Checks ids, ranges and the spawn table.
    pub fn validate(&self) -> Result<(), ZoneError> {
        let err = |m: String| Err(ZoneError(format!("zone {}: {m}", self.id)));
        if !valid_id(&self.id) {
            return err("the id must be namespace:name (a-z, 0-9, _)".into());
        }
        if self.areas.is_empty() || self.areas.len() > MAX_AREAS {
            return err(format!("needs 1..={MAX_AREAS} areas"));
        }
        for a in &self.areas {
            if let Err(m) = a.validate() {
                return err(m);
            }
        }
        for s in self.spawns.iter().flatten() {
            if !valid_id(&s.entity) {
                return err(format!("spawn entity {:?} must be mod:name", s.entity));
            }
            if s.weight == 0 || s.group[0] == 0 || s.group[0] > s.group[1] {
                return err(format!(
                    "spawn {}: weight must be > 0 and group a range 1 ≤ min ≤ max",
                    s.entity
                ));
            }
            if s.light.is_some_and(|l| l[0] > l[1] || l[1] > 15) {
                return err(format!(
                    "spawn {}: light must be a range in 0..=15",
                    s.entity
                ));
            }
        }
        Ok(())
    }

    fn is_member(&self, player: &str) -> bool {
        self.members.iter().any(|m| m == player)
    }
}

/// A zone's spawn table, ready for the spawn system.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpawnTable {
    entries: Vec<SpawnEntry>,
}

impl SpawnTable {
    /// The entries.
    #[must_use]
    pub fn entries(&self) -> &[SpawnEntry] {
        &self.entries
    }

    /// Picks an entry by weight among those allowed at `light`; `roll` is
    /// any random number. `None` when nothing may spawn at this light.
    #[must_use]
    pub fn pick(&self, roll: u64, light: u8) -> Option<&SpawnEntry> {
        let fits = |e: &&SpawnEntry| e.light.is_none_or(|[lo, hi]| light >= lo && light <= hi);
        let total: u64 = self
            .entries
            .iter()
            .filter(fits)
            .map(|e| u64::from(e.weight))
            .sum();
        if total == 0 {
            return None;
        }
        let mut r = roll % total;
        for e in self.entries.iter().filter(fits) {
            let w = u64::from(e.weight);
            if r < w {
                return Some(e);
            }
            r -= w;
        }
        None
    }
}

/// A zone in the index.
#[derive(Debug)]
struct Compiled {
    def: ZoneDef,
    spawns: Option<SpawnTable>,
}

/// Which zone decided each rule at a position.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Resolved {
    /// The highest-priority zone at the position.
    pub top: Option<u16>,
    /// The zone that set `build`, with its value.
    pub build: Option<(u16, bool)>,
    /// The zone that set `entry`, with its value.
    pub entry: Option<(u16, bool)>,
    /// The zone that set `pvp`, with its value.
    pub pvp: Option<(u16, bool)>,
    /// The zone whose spawn table applies.
    pub spawns: Option<u16>,
}

impl Resolved {
    fn done(&self) -> bool {
        self.top.is_some()
            && self.build.is_some()
            && self.entry.is_some()
            && self.pvp.is_some()
            && self.spawns.is_some()
    }
}

/// A fast hash for tile keys (the default SipHash is several times slower
/// for two `i32`s; keys are not attacker-chosen).
#[derive(Default)]
struct TileHasher(u64);

impl Hasher for TileHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0.rotate_left(8) ^ u64::from(b)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        }
    }
    fn write_u64(&mut self, n: u64) {
        self.0 = (self.0 ^ n).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        self.0 ^= self.0 >> 29;
    }
}

type Tiles = HashMap<u64, Box<[u32]>, BuildHasherDefault<TileHasher>>;

#[inline]
fn tile_key(tx: i32, tz: i32) -> u64 {
    (u64::from(tx as u32) << 32) | u64::from(tz as u32)
}

/// Zones by position. Built once, then read by every region.
#[derive(Debug, Default)]
pub struct ZoneIndex {
    zones: Vec<Compiled>,
    /// Every area as (zone, area index), in decision order.
    areas: Vec<(u16, Area)>,
    /// Tile → areas overlapping it, in decision order.
    tiles: Tiles,
    /// Areas too wide for tiles, in decision order.
    wide: Box<[u32]>,
}

impl ZoneIndex {
    /// Builds the index. Fails on an invalid zone, a duplicate id or too many
    /// zones.
    pub fn build(defs: Vec<ZoneDef>) -> Result<Self, ZoneError> {
        if defs.len() > MAX_ZONES {
            return Err(ZoneError(format!(
                "{} zones; at most {MAX_ZONES} are supported",
                defs.len()
            )));
        }
        let mut by_id = BTreeMap::new();
        for d in &defs {
            d.validate()?;
            if by_id.insert(d.id.clone(), ()).is_some() {
                return Err(ZoneError(format!("zone {} is defined twice", d.id)));
            }
        }
        let mut defs = defs;
        // Decision order: higher priority first, then by id.
        defs.sort_by(|a, b| b.priority.cmp(&a.priority).then_with(|| a.id.cmp(&b.id)));
        let zones: Vec<Compiled> = defs
            .into_iter()
            .map(|def| Compiled {
                spawns: def.spawns.clone().map(|entries| SpawnTable { entries }),
                def,
            })
            .collect();
        let mut areas = Vec::new();
        for (z, c) in zones.iter().enumerate() {
            for a in &c.def.areas {
                areas.push((z as u16, *a));
            }
        }
        let mut tiles: HashMap<u64, Vec<u32>, BuildHasherDefault<TileHasher>> = HashMap::default();
        let mut wide = Vec::new();
        for (i, (_, a)) in areas.iter().enumerate() {
            let (tx, tz, n) = a.tiles();
            if n > MAX_TILES_PER_AREA {
                wide.push(i as u32);
                continue;
            }
            for x in tx[0]..=tx[1] {
                for z in tz[0]..=tz[1] {
                    tiles.entry(tile_key(x, z)).or_default().push(i as u32);
                }
            }
        }
        Ok(Self {
            zones,
            areas,
            tiles: tiles
                .into_iter()
                .map(|(k, v)| (k, v.into_boxed_slice()))
                .collect(),
            wide: wide.into_boxed_slice(),
        })
    }

    /// Whether no zone is defined.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.zones.is_empty()
    }

    /// How many zones the index holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.zones.len()
    }

    /// The zone with this index (from [`Resolved`]).
    #[must_use]
    pub fn zone(&self, index: u16) -> Option<&ZoneDef> {
        self.zones.get(usize::from(index)).map(|c| &c.def)
    }

    /// Which zones decide each rule at `p`: one hash lookup and a scan of
    /// the areas of that 64×64 tile (plus the few very wide areas).
    #[must_use]
    pub fn resolve(&self, p: BlockPos) -> Resolved {
        let mut r = Resolved::default();
        if self.zones.is_empty() {
            return r;
        }
        let tile = self
            .tiles
            .get(&tile_key(p.x >> TILE_SHIFT, p.z >> TILE_SHIFT))
            .map_or(&[][..], |t| &t[..]);
        if self.wide.is_empty() {
            self.scan(tile, p, &mut r);
        } else {
            // Merge the two lists in decision order (both are sorted).
            let (mut i, mut j) = (0, 0);
            while (i < tile.len() || j < self.wide.len()) && !r.done() {
                let next = match (tile.get(i), self.wide.get(j)) {
                    (Some(&a), Some(&b)) if a <= b => {
                        i += 1;
                        a
                    }
                    (Some(&a), None) => {
                        i += 1;
                        a
                    }
                    (_, Some(&b)) => {
                        j += 1;
                        b
                    }
                    (None, None) => break,
                };
                self.visit(next, p, &mut r);
            }
        }
        r
    }

    /// The same answer as [`ZoneIndex::resolve`] by scanning every area,
    /// without tiles: the reference the index is checked against.
    #[doc(hidden)]
    #[must_use]
    pub fn resolve_reference(&self, p: BlockPos) -> Resolved {
        let mut r = Resolved::default();
        for i in 0..self.areas.len() {
            self.visit(i as u32, p, &mut r);
        }
        r
    }

    #[inline]
    fn scan(&self, list: &[u32], p: BlockPos, r: &mut Resolved) {
        for &i in list {
            self.visit(i, p, r);
            if r.done() {
                break;
            }
        }
    }

    #[inline]
    fn visit(&self, i: u32, p: BlockPos, r: &mut Resolved) {
        let (z, a) = &self.areas[i as usize];
        if !a.contains(p) {
            return;
        }
        let c = &self.zones[usize::from(*z)];
        r.top.get_or_insert(*z);
        let rules = c.def.rules;
        if r.build.is_none() {
            r.build = rules.build.map(|v| (*z, v));
        }
        if r.entry.is_none() {
            r.entry = rules.entry.map(|v| (*z, v));
        }
        if r.pvp.is_none() {
            r.pvp = rules.pvp.map(|v| (*z, v));
        }
        if r.spawns.is_none() && c.spawns.is_some() {
            r.spawns = Some(*z);
        }
    }

    fn allowed(&self, rule: Option<(u16, bool)>, player: &str) -> bool {
        match rule {
            None | Some((_, true)) => true,
            Some((z, false)) => self.zones[usize::from(z)].def.is_member(player),
        }
    }

    /// Whether `player` (by name) may break or place the block at `p`.
    #[must_use]
    pub fn may_build(&self, p: BlockPos, player: &str) -> bool {
        self.allowed(self.resolve(p).build, player)
    }

    /// Whether `player` (by name) may be at `p`.
    #[must_use]
    pub fn may_enter(&self, p: BlockPos, player: &str) -> bool {
        self.allowed(self.resolve(p).entry, player)
    }

    /// Whether a player at `attacker` may hurt a player at `victim`: PvP must
    /// be allowed at both positions.
    #[must_use]
    pub fn pvp_allowed(&self, attacker: BlockPos, victim: BlockPos) -> bool {
        let ok = |p| self.resolve(p).pvp.is_none_or(|(_, v)| v);
        ok(attacker) && ok(victim)
    }

    /// The spawn table at `p`; `None` where no zone sets one (the build's
    /// default spawning applies).
    #[must_use]
    pub fn spawn_table(&self, p: BlockPos) -> Option<&SpawnTable> {
        let z = self.resolve(p).spawns?;
        self.zones[usize::from(z)].spawns.as_ref()
    }

    /// The highest-priority zone at `p`.
    #[must_use]
    pub fn zone_at(&self, p: BlockPos) -> Option<&ZoneDef> {
        self.resolve(p).top.and_then(|z| self.zone(z))
    }
}

/// The world's own zones (`<world>/zones/zones.toml`).
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorldZones {
    #[serde(default)]
    zone: Vec<ZoneDef>,
}

/// The world's zone file.
#[must_use]
pub fn world_file(world_dir: &Path) -> std::path::PathBuf {
    world_dir.join("zones").join("zones.toml")
}

/// Reads the world's own zones; none when the file does not exist.
pub fn read_world(world_dir: &Path) -> Result<Vec<ZoneDef>, ZoneError> {
    let path = world_file(world_dir);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(ZoneError(format!("{}: {e}", path.display()))),
    };
    let file: WorldZones =
        toml::from_str(&text).map_err(|e| ZoneError(format!("{}: {e}", path.display())))?;
    for z in &file.zone {
        z.validate()
            .map_err(|e| ZoneError(format!("{}: {e}", path.display())))?;
    }
    Ok(file.zone)
}

/// Writes the world's own zones (atomically: a temporary file, then a rename).
pub fn write_world(world_dir: &Path, zones: &[ZoneDef]) -> Result<(), ZoneError> {
    let path = world_file(world_dir);
    let io = |e: std::io::Error| ZoneError(format!("{}: {e}", path.display()));
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(io)?;
    }
    let text = toml::to_string(&WorldZones {
        zone: zones.to_vec(),
    })
    .map_err(|e| ZoneError(e.to_string()))?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text).map_err(io)?;
    std::fs::rename(&tmp, &path).map_err(io)
}

/// The zones of a build's mods (`data/zones/*.toml`, in load order).
///
/// A zone's namespace must be its mod's id. A later mod may replace a zone
/// of a mod it depends on with `override = true`; any other repeated id is
/// an error naming both mods.
pub fn from_mods(mods: &ModSet) -> Result<Vec<ZoneDef>, ZoneError> {
    let mut out: Vec<(String, ZoneDef)> = Vec::new();
    for m in mods.mods() {
        let dir = m.data_dir().join("zones");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut files: Vec<_> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "toml"))
            .collect();
        files.sort();
        for path in files {
            let at = |e: String| ZoneError(format!("{}: {e}", path.display()));
            let text = std::fs::read_to_string(&path).map_err(|e| at(e.to_string()))?;
            let def = ZoneDef::from_toml(&text).map_err(|e| at(e.0))?;
            let ns = def.id.split_once(':').map_or("", |(ns, _)| ns);
            if let Some(i) = out.iter().position(|(_, d)| d.id == def.id) {
                let owner = out[i].0.clone();
                let depends = m.dependencies.iter().any(|d| d.id == owner);
                if !(def.replaces && depends) {
                    return Err(at(format!(
                        "zone {} is already defined by mod {owner}; to replace it, depend on \
                         {owner} and set override = true",
                        def.id
                    )));
                }
                out[i] = (m.id.clone(), def);
            } else if ns != m.id {
                return Err(at(format!(
                    "zone {} must be in the namespace of mod {} ({}:<name>)",
                    def.id, m.id, m.id
                )));
            } else {
                out.push((m.id.clone(), def));
            }
        }
    }
    Ok(out.into_iter().map(|(_, d)| d).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bp(x: i32, y: i32, z: i32) -> BlockPos {
        BlockPos { x, y, z }
    }

    fn zone(id: &str, priority: i32, x: [i32; 2], z: [i32; 2], rules: ZoneRules) -> ZoneDef {
        ZoneDef {
            id: id.into(),
            name: String::new(),
            priority,
            difficulty: 0,
            areas: vec![Area { x, z, y: None }],
            members: Vec::new(),
            rules,
            spawns: None,
            replaces: false,
        }
    }

    const DENY_BUILD: ZoneRules = ZoneRules {
        build: Some(false),
        entry: None,
        pvp: None,
    };

    #[test]
    fn parses_the_documented_format() {
        let z = ZoneDef::from_toml(
            r#"
id = "rpg:town"
name = "Town"
priority = 10
difficulty = 1
areas = [{ x = [-64, 63], z = [-64, 63] }, { x = [100, 120], z = [0, 5], y = [60, 80] }]
members = ["alice"]
rules = { build = false, pvp = false }
spawns = [
  { entity = "mobs:villager", weight = 80, group = [1, 3] },
  { entity = "mobs:cat", weight = 20, light = [8, 15] },
]
"#,
        )
        .expect("parse");
        assert_eq!(z.areas.len(), 2);
        assert_eq!(z.rules.build, Some(false));
        assert_eq!(z.rules.entry, None);
        assert_eq!(z.spawns.as_ref().map(Vec::len), Some(2));
        assert_eq!(z.spawns.as_ref().map(|s| s[1].group), Some([1, 1]));
    }

    #[test]
    fn bad_definitions_are_clear_errors() {
        for (text, needle) in [
            (
                "id = \"town\"\nareas = [{ x = [0, 1], z = [0, 1] }]",
                "namespace:name",
            ),
            ("id = \"a:b\"\nareas = []", "areas"),
            (
                "id = \"a:b\"\nareas = [{ x = [5, 1], z = [0, 1] }]",
                "start is above",
            ),
            (
                "id = \"a:b\"\nareas = [{ x = [0, 1], z = [0, 1] }]\nspawns = [{ entity = \"m:x\", weight = 0 }]",
                "weight",
            ),
            (
                "id = \"a:b\"\nareas = [{ x = [0, 1], z = [0, 1] }]\nspawns = [{ entity = \"m:x\", weight = 1, light = [3, 16] }]",
                "light",
            ),
            (
                "id = \"a:b\"\nareas = [{ x = [0, 1], z = [0, 1] }]\npvp = true",
                "unknown field",
            ),
        ] {
            let e = ZoneDef::from_toml(text).expect_err(text).0;
            assert!(e.contains(needle), "{text}: {e}");
        }
        let dup = vec![
            zone("a:x", 0, [0, 1], [0, 1], DENY_BUILD),
            zone("a:x", 0, [5, 6], [0, 1], DENY_BUILD),
        ];
        assert!(ZoneIndex::build(dup).expect_err("dup").0.contains("twice"));
    }

    #[test]
    fn lookup_by_position_and_inclusive_bounds() {
        let idx =
            ZoneIndex::build(vec![zone("a:x", 0, [-10, 10], [0, 70], DENY_BUILD)]).expect("index");
        assert!(!idx.may_build(bp(-10, 64, 0), "bob"));
        assert!(!idx.may_build(bp(10, -300, 70), "bob"));
        assert!(idx.may_build(bp(11, 64, 0), "bob"));
        assert!(idx.may_build(bp(0, 64, -1), "bob"));
        assert!(idx.may_build(bp(0, 64, 71), "bob"));
        assert_eq!(idx.zone_at(bp(0, 0, 0)).map(|z| z.id.as_str()), Some("a:x"));
        assert!(idx.zone_at(bp(500, 0, 0)).is_none());
        // Negative coordinates across tile borders.
        let idx = ZoneIndex::build(vec![zone("a:y", 0, [-200, -65], [-1, 0], DENY_BUILD)])
            .expect("index");
        assert!(!idx.may_build(bp(-65, 0, -1), "bob"));
        assert!(!idx.may_build(bp(-200, 0, 0), "bob"));
        assert!(idx.may_build(bp(-64, 0, 0), "bob"));
        assert!(idx.may_build(bp(-201, 0, 0), "bob"));
    }

    #[test]
    fn height_limits_apply() {
        let mut z = zone("a:x", 0, [0, 15], [0, 15], DENY_BUILD);
        z.areas[0].y = Some([60, 70]);
        let idx = ZoneIndex::build(vec![z]).expect("index");
        assert!(!idx.may_build(bp(5, 60, 5), "bob"));
        assert!(!idx.may_build(bp(5, 70, 5), "bob"));
        assert!(idx.may_build(bp(5, 71, 5), "bob"));
        assert!(idx.may_build(bp(5, 59, 5), "bob"));
    }

    #[test]
    fn priority_decides_each_rule_separately() {
        // A no-PvP town inside a no-build wilderness reserve.
        let reserve = zone("a:reserve", 0, [-100, 100], [-100, 100], DENY_BUILD);
        let town = zone(
            "a:town",
            5,
            [-10, 10],
            [-10, 10],
            ZoneRules {
                build: None,
                entry: None,
                pvp: Some(false),
            },
        );
        let plot = zone(
            "a:plot",
            9,
            [0, 3],
            [0, 3],
            ZoneRules {
                build: Some(true),
                entry: None,
                pvp: None,
            },
        );
        let idx = ZoneIndex::build(vec![reserve, plot, town]).expect("index");
        let c = bp(1, 64, 1); // in all three
        assert!(idx.may_build(c, "bob"), "the plot allows building");
        assert!(!idx.pvp_allowed(c, c), "the town's PvP rule still holds");
        assert_eq!(idx.zone_at(c).map(|z| z.id.as_str()), Some("a:plot"));
        let t = bp(-5, 64, -5); // town + reserve
        assert!(
            !idx.may_build(t, "bob"),
            "the town leaves building to the reserve"
        );
        let w = bp(50, 64, 50); // reserve only
        assert!(!idx.may_build(w, "bob") && idx.pvp_allowed(w, w));
        // PvP needs both ends.
        assert!(!idx.pvp_allowed(w, t) && !idx.pvp_allowed(t, w));
        // Same priority: ordered by id, so the result does not depend on the
        // definition order.
        let a = zone("a:a", 1, [0, 1], [0, 1], DENY_BUILD);
        let mut b = zone("a:b", 1, [0, 1], [0, 1], DENY_BUILD);
        b.rules.build = Some(true);
        let one = ZoneIndex::build(vec![a.clone(), b.clone()]).expect("index");
        let two = ZoneIndex::build(vec![b, a]).expect("index");
        assert!(!one.may_build(bp(0, 0, 0), "x") && !two.may_build(bp(0, 0, 0), "x"));
    }

    #[test]
    fn members_are_exempt_from_build_and_entry() {
        let mut z = zone(
            "a:base",
            0,
            [0, 15],
            [0, 15],
            ZoneRules {
                build: Some(false),
                entry: Some(false),
                pvp: Some(false),
            },
        );
        z.members = vec!["alice".into()];
        let idx = ZoneIndex::build(vec![z]).expect("index");
        let p = bp(3, 64, 3);
        assert!(idx.may_build(p, "alice") && idx.may_enter(p, "alice"));
        assert!(!idx.may_build(p, "bob") && !idx.may_enter(p, "bob"));
        assert!(!idx.pvp_allowed(p, p), "members do not bypass PvP");
    }

    #[test]
    fn wide_areas_merge_in_priority_order() {
        let world = zone(
            "a:world",
            0,
            [-5_000_000, 5_000_000],
            [-5_000_000, 5_000_000],
            ZoneRules {
                build: None,
                entry: None,
                pvp: Some(false),
            },
        );
        let arena = zone(
            "a:arena",
            3,
            [0, 31],
            [0, 31],
            ZoneRules {
                build: Some(false),
                entry: None,
                pvp: Some(true),
            },
        );
        let mut sky = zone("a:sky", 9, [-4_000_000, 4_000_000], [-1, 1], DENY_BUILD);
        sky.areas[0].y = Some([300, 400]);
        let idx = ZoneIndex::build(vec![world, arena, sky]).expect("index");
        assert_eq!(idx.wide.len(), 2);
        assert!(
            idx.pvp_allowed(bp(5, 64, 5), bp(6, 64, 6)),
            "the arena wins"
        );
        assert!(
            !idx.pvp_allowed(bp(40, 64, 5), bp(5, 64, 5)),
            "outside: no PvP"
        );
        assert!(!idx.pvp_allowed(bp(-3_000_000, 0, 3_000_000), bp(0, 0, 0)));
        assert!(!idx.may_build(bp(0, 350, 0), "x"), "the sky zone");
        assert!(!idx.may_build(bp(0, 64, 0), "x"), "the arena");
        assert!(idx.may_build(bp(100, 64, 100), "x"));
    }

    #[test]
    fn spawn_tables_come_only_from_the_zone_under_the_position() {
        let mut town = zone("a:town", 5, [0, 63], [0, 63], ZoneRules::default());
        town.spawns = Some(vec![]);
        let mut forest = zone(
            "a:forest",
            0,
            [-500, 500],
            [-500, 500],
            ZoneRules::default(),
        );
        forest.spawns = Some(vec![
            SpawnEntry {
                entity: "mobs:wolf".into(),
                weight: 3,
                group: [2, 4],
                light: None,
            },
            SpawnEntry {
                entity: "mobs:ghost".into(),
                weight: 1,
                group: [1, 1],
                light: Some([0, 7]),
            },
        ]);
        let mut quiet = zone("a:quiet", 9, [200, 210], [200, 210], ZoneRules::default());
        quiet.difficulty = 2; // sets no table: the forest's applies
        let idx = ZoneIndex::build(vec![town, forest, quiet]).expect("index");
        let t = idx.spawn_table(bp(10, 64, 10)).expect("town table");
        assert!(
            t.entries().is_empty() && t.pick(7, 0).is_none(),
            "nothing in town"
        );
        let f = idx.spawn_table(bp(-100, 64, 10)).expect("forest table");
        let mut counts: BTreeMap<&str, u32> = BTreeMap::new();
        for roll in 0..4000 {
            *counts
                .entry(&f.pick(roll, 0).expect("pick").entity)
                .or_default() += 1;
        }
        assert_eq!(counts["mobs:wolf"], 3000);
        assert_eq!(counts["mobs:ghost"], 1000);
        // In daylight the ghost is filtered out.
        assert!((0..100).all(|r| f.pick(r, 15).expect("pick").entity == "mobs:wolf"));
        assert_eq!(
            idx.spawn_table(bp(205, 64, 205)).map(|t| t.entries().len()),
            Some(2)
        );
        assert!(
            idx.spawn_table(bp(900, 64, 0)).is_none(),
            "no zone: default spawning"
        );
    }

    #[test]
    fn world_file_round_trips() {
        let dir = std::env::temp_dir().join(format!("world-zones-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(read_world(&dir).expect("missing is empty").is_empty());
        let mut z = zone("srv:claim_1", 2, [0, 9], [0, 9], DENY_BUILD);
        z.members = vec!["alice".into()];
        z.spawns = Some(Vec::new());
        write_world(&dir, &[z.clone()]).expect("write");
        assert_eq!(read_world(&dir).expect("read"), vec![z]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn write_mod(root: &Path, id: &str, deps: &str, zones: &[(&str, &str)]) {
        let dir = root.join(id);
        std::fs::create_dir_all(dir.join("data/zones")).expect("mkdir");
        std::fs::write(
            dir.join("mod.toml"),
            format!("[mod]\nid = \"{id}\"\nversion = \"0.1.0\"\n\n[dependencies]\n{deps}"),
        )
        .expect("write");
        for (file, text) in zones {
            std::fs::write(dir.join("data/zones").join(file), text).expect("write");
        }
    }

    #[test]
    fn mods_provide_zones_and_override_only_what_they_depend_on() {
        let root = std::env::temp_dir().join(format!("world-zone-mods-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let area = "areas = [{ x = [0, 9], z = [0, 9] }]";
        write_mod(
            &root,
            "base",
            "",
            &[("town.toml", &format!("id = \"base:town\"\n{area}"))],
        );
        write_mod(
            &root,
            "pack",
            "base = \"^0.1\"\n",
            &[(
                "town.toml",
                &format!("id = \"base:town\"\noverride = true\npriority = 4\n{area}"),
            )],
        );
        write_mod(
            &root,
            "rogue",
            "",
            &[(
                "town.toml",
                &format!("id = \"base:town\"\noverride = true\n{area}"),
            )],
        );
        write_mod(
            &root,
            "thief",
            "",
            &[("x.toml", &format!("id = \"base:other\"\n{area}"))],
        );
        let set = ModSet::resolve("t", &["base", "pack"], &root).expect("set");
        let zones = from_mods(&set).expect("zones");
        assert_eq!(zones.len(), 1);
        assert_eq!(zones[0].priority, 4, "the dependent mod replaced the zone");
        let set = ModSet::resolve("t", &["base", "rogue"], &root).expect("set");
        let e = from_mods(&set).expect_err("no dependency").0;
        assert!(e.contains("already defined by mod base"), "{e}");
        let set = ModSet::resolve("t", &["thief"], &root).expect("set");
        let e = from_mods(&set).expect_err("namespace").0;
        assert!(e.contains("namespace of mod thief"), "{e}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn index_matches_the_reference_scan() {
        let mut seed = 0x1234_5678_u64;
        let mut next = |n: i32| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as i32
        };
        let flag = |v: i32| match v {
            0 => None,
            1 => Some(false),
            _ => Some(true),
        };
        let mut defs = Vec::new();
        for i in 0..300 {
            let (x, z) = (next(2000) - 1000, next(2000) - 1000);
            let (w, d) = if i % 50 == 0 {
                (5_000_000, 3)
            } else {
                (next(200), next(200))
            };
            let mut def = zone(
                &format!("t:z{i}"),
                next(5),
                [x, x.saturating_add(w)],
                [z, z + d],
                ZoneRules {
                    build: flag(next(3)),
                    entry: flag(next(3)),
                    pvp: flag(next(3)),
                },
            );
            if next(4) == 0 {
                def.areas[0].y = Some([next(100), 100 + next(100)]);
            }
            if next(3) == 0 {
                def.spawns = Some(Vec::new());
            }
            defs.push(def);
        }
        let idx = ZoneIndex::build(defs).expect("index");
        assert!(!idx.wide.is_empty());
        for _ in 0..20_000 {
            let p = bp(next(2400) - 1200, next(220) - 10, next(2400) - 1200);
            assert_eq!(idx.resolve(p), idx.resolve_reference(p), "{p:?}");
        }
    }

    #[test]
    fn empty_index_allows_everything() {
        let idx = ZoneIndex::build(Vec::new()).expect("index");
        assert!(idx.is_empty());
        let p = bp(0, 0, 0);
        assert!(idx.may_build(p, "x") && idx.may_enter(p, "x") && idx.pvp_allowed(p, p));
        assert!(idx.spawn_table(p).is_none());
    }
}
