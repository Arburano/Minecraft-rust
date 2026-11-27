//! Builds and mod sets: which mods a game is made of, in which order, and where
//! their data lives (ADR-0017, `docs/modding/builds.md`).
//!
//! A *build* (`builds/<id>.toml`) lists the mods a game runs with on top of the
//! engine. The mods are found by id among the folders of a mods root (each
//! folder holds a `mod.toml`). Dependencies named in a manifest are added even
//! if the build does not list them; the order is topological, ties broken by
//! id (`docs/modding/mod-manifest.md`).
//!
//! Data lookup has two forms:
//! - *merged*: every mod's `data/blocks` contributes blocks under its own id;
//! - *provided*: a data file such as `worldgen/overworld.toml` comes from the
//!   last mod in load order that has it, so a later mod overrides an earlier
//!   one (a dimension mod provides its generation, a pack may replace it).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::RegistryBuilder;
use crate::registry::{RegistryError, blocks_from_dir};

/// Errors while reading a build or resolving its mods.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModSetError {
    /// A file could not be read.
    Io(String),
    /// A `build.toml` or `mod.toml` is malformed.
    Parse(String),
    /// A build or a dependency names a mod that no folder provides.
    Missing {
        /// The mod that is missing.
        id: String,
        /// Who asked for it (`build classic`, `mod nether`).
        by: String,
    },
    /// A mod's version does not satisfy a requirement.
    Version {
        /// The mod.
        id: String,
        /// Its version.
        version: String,
        /// The requirement it fails.
        required: String,
        /// Who asked for it.
        by: String,
    },
    /// Two folders declare the same mod id.
    Duplicate(String),
    /// Dependencies form a cycle through these mods.
    Cycle(Vec<String>),
    /// No mod of the set provides a data file.
    NotProvided(String),
    /// A block file of a mod is invalid.
    Blocks(String),
    /// Two mods of the build fill the same gameplay slot (ADR-0013).
    SlotConflict {
        /// The slot (`movement`, `worldgen:overworld`).
        slot: String,
        /// The two providers.
        mods: [String; 2],
    },
    /// No mod of the build fills a slot the game needs.
    SlotMissing {
        /// The slot.
        slot: String,
        /// The build.
        build: String,
    },
}

impl fmt::Display for ModSetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Parse(e) => write!(f, "{e}"),
            Self::Missing { id, by } => write!(f, "{by} needs mod {id:?}, which is not installed"),
            Self::Version {
                id,
                version,
                required,
                by,
            } => write!(f, "{by} needs {id} {required}, installed is {version}"),
            Self::Duplicate(id) => write!(f, "two folders declare mod {id:?}"),
            Self::Cycle(ids) => write!(f, "dependency cycle among mods {}", ids.join(", ")),
            Self::NotProvided(file) => write!(
                f,
                "no mod of this build provides data/{file} (is the mod that owns it in the build?)"
            ),
            Self::Blocks(e) => write!(f, "{e}"),
            Self::SlotConflict { slot, mods } => write!(
                f,
                "mods {} and {} both provide {slot:?}; a build takes one provider per slot",
                mods[0], mods[1]
            ),
            Self::SlotMissing { slot, build } => write!(
                f,
                "build {build} has no mod that provides {slot:?} (add one, e.g. a movement or world module)"
            ),
        }
    }
}

impl std::error::Error for ModSetError {}

impl From<RegistryError> for ModSetError {
    fn from(e: RegistryError) -> Self {
        Self::Blocks(e.to_string())
    }
}

/// A build file: `builds/<id>.toml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildManifest {
    /// The `[build]` table.
    pub build: BuildInfo,
}

/// The `[build]` table of a build file.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildInfo {
    /// Unique id (`[a-z0-9_]`).
    pub id: String,
    /// Name shown to players.
    pub name: String,
    /// One line about the build.
    #[serde(default)]
    pub description: String,
    /// The mods the build runs with; their dependencies are added.
    pub mods: Vec<String>,
}

/// The parts of `mod.toml` the mod set needs.
#[derive(Debug, Clone, Deserialize)]
struct RawManifest {
    #[serde(rename = "mod")]
    info: RawModInfo,
    #[serde(default)]
    dependencies: BTreeMap<String, RawDependency>,
}

#[derive(Debug, Clone, Deserialize)]
struct RawModInfo {
    id: String,
    version: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    provides: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum RawDependency {
    Version(String),
    Table {
        version: String,
        #[serde(default)]
        optional: bool,
    },
}

/// A dependency of a mod.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependency {
    /// The mod depended on.
    pub id: String,
    /// Version requirement (`^0.1`, `0.1.2`).
    pub version: String,
    /// Used only if the build has it anyway.
    pub optional: bool,
}

/// One mod of a set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModInfo {
    /// The mod id, also the namespace of its content.
    pub id: String,
    /// The mod's version.
    pub version: String,
    /// One line about the mod.
    pub description: String,
    /// The mod's folder (holds `mod.toml`).
    pub dir: PathBuf,
    /// Declared dependencies.
    pub dependencies: Vec<Dependency>,
    /// Gameplay slots this mod fills (`movement`, `worldgen:<dimension>`).
    pub provides: Vec<String>,
}

/// The slot of a dimension's world generator.
#[must_use]
pub fn worldgen_slot(dimension: &str) -> String {
    format!("worldgen:{dimension}")
}

/// The slot of the player movement profile.
pub const MOVEMENT_SLOT: &str = "movement";

/// Whether a slot name is known: `movement` or `worldgen:<dimension>`.
fn valid_slot(slot: &str) -> bool {
    slot == MOVEMENT_SLOT
        || slot.strip_prefix("worldgen:").is_some_and(|d| {
            !d.is_empty()
                && d.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        })
}

impl ModInfo {
    /// Reads `<dir>/mod.toml`.
    pub fn read(dir: &Path) -> Result<Self, ModSetError> {
        let path = dir.join("mod.toml");
        let text = std::fs::read_to_string(&path)
            .map_err(|e| ModSetError::Io(format!("{}: {e}", path.display())))?;
        let raw: RawManifest = toml::from_str(&text)
            .map_err(|e| ModSetError::Parse(format!("{}: {e}", path.display())))?;
        let dependencies = raw
            .dependencies
            .into_iter()
            .map(|(id, d)| match d {
                RawDependency::Version(version) => Dependency {
                    id,
                    version,
                    optional: false,
                },
                RawDependency::Table { version, optional } => Dependency {
                    id,
                    version,
                    optional,
                },
            })
            .collect();
        if let Some(bad) = raw.info.provides.iter().find(|s| !valid_slot(s)) {
            return Err(ModSetError::Parse(format!(
                "{}: unknown slot {bad:?} (known: \"movement\", \"worldgen:<dimension>\")",
                path.display()
            )));
        }
        Ok(Self {
            id: raw.info.id,
            version: raw.info.version,
            description: raw.info.description,
            dir: dir.to_path_buf(),
            dependencies,
            provides: raw.info.provides,
        })
    }

    /// The mod's data folder.
    #[must_use]
    pub fn data_dir(&self) -> PathBuf {
        self.dir.join("data")
    }
}

/// The resolved, ordered mods of a build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModSet {
    build: String,
    mods: Vec<ModInfo>,
}

impl ModSet {
    /// Reads a build file and resolves its mods among the folders of `mods_root`.
    pub fn from_build(build_file: &Path, mods_root: &Path) -> Result<Self, ModSetError> {
        let text = std::fs::read_to_string(build_file)
            .map_err(|e| ModSetError::Io(format!("{}: {e}", build_file.display())))?;
        let manifest: BuildManifest = toml::from_str(&text)
            .map_err(|e| ModSetError::Parse(format!("{}: {e}", build_file.display())))?;
        let ids: Vec<&str> = manifest.build.mods.iter().map(String::as_str).collect();
        Self::resolve(&manifest.build.id, &ids, mods_root)
    }

    /// Resolves `ids` and their dependencies among the folders of `mods_root`.
    pub fn resolve(build: &str, ids: &[&str], mods_root: &Path) -> Result<Self, ModSetError> {
        let installed = installed(mods_root)?;
        let mut chosen: BTreeMap<String, ModInfo> = BTreeMap::new();
        let mut queue: Vec<(String, Option<String>, String)> = ids
            .iter()
            .map(|id| ((*id).to_owned(), None, format!("build {build}")))
            .collect();
        while let Some((id, required, by)) = queue.pop() {
            let Some(info) = installed.get(&id) else {
                return Err(ModSetError::Missing { id, by });
            };
            if let Some(req) = &required
                && !version_matches(req, &info.version)
            {
                return Err(ModSetError::Version {
                    id,
                    version: info.version.clone(),
                    required: req.clone(),
                    by,
                });
            }
            if chosen.contains_key(&id) {
                continue;
            }
            for dep in info.dependencies.iter().filter(|d| !d.optional) {
                queue.push((
                    dep.id.clone(),
                    Some(dep.version.clone()),
                    format!("mod {}", info.id),
                ));
            }
            chosen.insert(id, info.clone());
        }
        // Optional dependencies: checked only when present.
        for info in chosen.values() {
            for dep in info.dependencies.iter().filter(|d| d.optional) {
                if let Some(other) = chosen.get(&dep.id)
                    && !version_matches(&dep.version, &other.version)
                {
                    return Err(ModSetError::Version {
                        id: dep.id.clone(),
                        version: other.version.clone(),
                        required: dep.version.clone(),
                        by: format!("mod {}", info.id),
                    });
                }
            }
        }
        let mods = topological(&chosen)?;
        // One provider per slot.
        let mut slots: BTreeMap<&str, &str> = BTreeMap::new();
        for m in &mods {
            for slot in &m.provides {
                if let Some(other) = slots.insert(slot, &m.id) {
                    return Err(ModSetError::SlotConflict {
                        slot: slot.clone(),
                        mods: [other.to_owned(), m.id.clone()],
                    });
                }
            }
        }
        Ok(Self {
            build: build.to_owned(),
            mods,
        })
    }

    /// The build id this set was made from.
    #[must_use]
    pub fn build(&self) -> &str {
        &self.build
    }

    /// The mods in load order (dependencies first, ties by id).
    #[must_use]
    pub fn mods(&self) -> &[ModInfo] {
        &self.mods
    }

    /// Whether the set has a mod.
    #[must_use]
    pub fn contains(&self, id: &str) -> bool {
        self.mods.iter().any(|m| m.id == id)
    }

    /// The `data/blocks` folders that exist, with their mod ids, in load order.
    #[must_use]
    pub fn block_dirs(&self) -> Vec<(String, PathBuf)> {
        self.mods
            .iter()
            .map(|m| (m.id.clone(), m.data_dir().join("blocks")))
            .filter(|(_, d)| d.is_dir())
            .collect()
    }

    /// A registry builder holding every mod's blocks under its own id.
    pub fn registry_builder(&self) -> Result<RegistryBuilder, ModSetError> {
        let mut b = RegistryBuilder::new();
        for (id, dir) in self.block_dirs() {
            b.add_mod(&id, blocks_from_dir(&dir)?);
        }
        Ok(b)
    }

    /// The path of `data/<file>` from the last mod in load order that has it.
    #[must_use]
    pub fn find(&self, file: &str) -> Option<PathBuf> {
        self.mods
            .iter()
            .rev()
            .map(|m| m.data_dir().join(file))
            .find(|p| p.is_file())
    }

    /// The mod that fills a gameplay slot, if any (at most one: checked when
    /// the set is resolved).
    #[must_use]
    pub fn slot(&self, slot: &str) -> Option<&ModInfo> {
        self.mods
            .iter()
            .find(|m| m.provides.iter().any(|s| s == slot))
    }

    /// The mod that fills a slot the game needs.
    pub fn require_slot(&self, slot: &str) -> Result<&ModInfo, ModSetError> {
        self.slot(slot).ok_or_else(|| ModSetError::SlotMissing {
            slot: slot.to_owned(),
            build: self.build.clone(),
        })
    }

    /// Reads `data/<file>` of the mod that fills `slot` (not of any other
    /// mod: the slot's provider owns the file).
    pub fn read_slot(&self, slot: &str, file: &str) -> Result<String, ModSetError> {
        let m = self.require_slot(slot)?;
        let path = m.data_dir().join(file);
        std::fs::read_to_string(&path)
            .map_err(|e| ModSetError::Io(format!("{} (slot {slot}): {e}", path.display())))
    }

    /// The mod that provides `data/<file>` ([`ModSet::find`]).
    #[must_use]
    pub fn provider(&self, file: &str) -> Option<&ModInfo> {
        self.mods
            .iter()
            .rev()
            .find(|m| m.data_dir().join(file).is_file())
    }

    /// Reads `data/<file>` from the mod that provides it ([`ModSet::find`]).
    pub fn read(&self, file: &str) -> Result<String, ModSetError> {
        let path = self
            .find(file)
            .ok_or_else(|| ModSetError::NotProvided(file.to_owned()))?;
        std::fs::read_to_string(&path)
            .map_err(|e| ModSetError::Io(format!("{}: {e}", path.display())))
    }

    /// Like [`ModSet::read`], but `None` when no mod provides the file.
    pub fn read_optional(&self, file: &str) -> Result<Option<String>, ModSetError> {
        match self.find(file) {
            None => Ok(None),
            Some(path) => std::fs::read_to_string(&path)
                .map(Some)
                .map_err(|e| ModSetError::Io(format!("{}: {e}", path.display()))),
        }
    }
}

/// Every mod folder directly under `root` (a folder with `mod.toml`), by id.
fn installed(root: &Path) -> Result<BTreeMap<String, ModInfo>, ModSetError> {
    let entries =
        std::fs::read_dir(root).map_err(|e| ModSetError::Io(format!("{}: {e}", root.display())))?;
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.join("mod.toml").is_file())
        .collect();
    dirs.sort();
    let mut out = BTreeMap::new();
    for dir in dirs {
        let info = ModInfo::read(&dir)?;
        if out.contains_key(&info.id) {
            return Err(ModSetError::Duplicate(info.id));
        }
        out.insert(info.id.clone(), info);
    }
    Ok(out)
}

/// Kahn's algorithm; among ready mods the smallest id goes first.
fn topological(chosen: &BTreeMap<String, ModInfo>) -> Result<Vec<ModInfo>, ModSetError> {
    let mut pending: BTreeMap<&str, BTreeSet<&str>> = chosen
        .values()
        .map(|m| {
            let deps = m
                .dependencies
                .iter()
                .map(|d| d.id.as_str())
                .filter(|d| chosen.contains_key(*d))
                .collect();
            (m.id.as_str(), deps)
        })
        .collect();
    let mut out = Vec::with_capacity(chosen.len());
    while !pending.is_empty() {
        let Some(next) = pending
            .iter()
            .find(|(_, deps)| deps.is_empty())
            .map(|(id, _)| *id)
        else {
            return Err(ModSetError::Cycle(
                pending.keys().map(|s| (*s).to_owned()).collect(),
            ));
        };
        pending.remove(next);
        for deps in pending.values_mut() {
            deps.remove(next);
        }
        if let Some(info) = chosen.get(next) {
            out.push(info.clone());
        }
    }
    Ok(out)
}

fn parse_version(v: &str) -> Option<[u64; 3]> {
    let mut parts = v.trim().split('.');
    let mut out = [0u64; 3];
    for slot in &mut out {
        match parts.next() {
            Some(p) => *slot = p.parse().ok()?,
            None => break,
        }
    }
    parts.next().is_none().then_some(out)
}

/// Cargo-style requirements: `^a.b.c` (the default; compatible within the
/// leftmost non-zero part), `=a.b.c` (exact), `*` (any).
fn version_matches(req: &str, version: &str) -> bool {
    let req = req.trim();
    if req == "*" {
        return true;
    }
    let Some(have) = parse_version(version) else {
        return false;
    };
    if let Some(exact) = req.strip_prefix('=') {
        return parse_version(exact) == Some(have);
    }
    let body = req.strip_prefix('^').unwrap_or(req);
    let given = body.trim().split('.').count();
    let Some(want) = parse_version(body) else {
        return false;
    };
    if have < want {
        return false;
    }
    // The leftmost non-zero part that was written must match.
    let pin = want
        .iter()
        .take(given)
        .position(|&x| x != 0)
        .unwrap_or(given.saturating_sub(1));
    have[..=pin] == want[..=pin]
}

/// The source tree's `mods/` and `builds/` folders, for tests, examples and
/// development runs from `cargo`. Shipped binaries look next to the executable.
#[must_use]
pub fn source_tree() -> (PathBuf, PathBuf) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    (root.join("mods"), root.join("builds"))
}

/// The build `id` of the source tree ([`source_tree`]).
pub fn source_build(id: &str) -> Result<ModSet, ModSetError> {
    let (mods, builds) = source_tree();
    ModSet::from_build(&builds.join(format!("{id}.toml")), &mods)
}

/// The default build's id.
pub const DEFAULT_BUILD: &str = "classic";

/// Finds and resolves a build for a binary (`server`, `client`, `bots`).
///
/// - `build`: a build id (`classic`) or a path to a build file (`*.toml`);
/// - `mods_root`: the folder of mod folders; default: `mods/` next to the
///   executable if it exists, else the source tree's.
///
/// A build id is looked up in `builds/` next to the mods root, then in the
/// source tree.
pub fn locate(build: &str, mods_root: Option<&Path>) -> Result<ModSet, ModSetError> {
    let (tree_mods, tree_builds) = source_tree();
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf));
    let mods = mods_root.map(Path::to_path_buf).unwrap_or_else(|| {
        exe_dir
            .as_ref()
            .map(|d| d.join("mods"))
            .filter(|d| d.is_dir())
            .unwrap_or(tree_mods)
    });
    let file = if build.ends_with(".toml") {
        PathBuf::from(build)
    } else {
        let name = format!("{build}.toml");
        mods.parent()
            .map(|p| p.join("builds").join(&name))
            .filter(|p| p.is_file())
            .unwrap_or_else(|| tree_builds.join(&name))
    };
    ModSet::from_build(&file, &mods)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_mod(root: &Path, folder: &str, toml: &str) {
        let dir = root.join(folder);
        std::fs::create_dir_all(dir.join("data")).expect("mkdir");
        std::fs::write(dir.join("mod.toml"), toml).expect("write");
    }

    fn manifest(id: &str, version: &str, deps: &str) -> String {
        format!("[mod]\nid = \"{id}\"\nversion = \"{version}\"\n\n[dependencies]\n{deps}\n")
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("modset-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    #[test]
    fn versions() {
        assert!(version_matches("^0.1", "0.1.0"));
        assert!(version_matches("^0.1", "0.1.7"));
        assert!(!version_matches("^0.1", "0.2.0"));
        assert!(!version_matches("^0.1.3", "0.1.2"));
        assert!(version_matches("^1.2", "1.9.0"));
        assert!(!version_matches("^1.2", "2.0.0"));
        assert!(!version_matches("^1.2", "1.1.9"));
        assert!(version_matches("=0.3.1", "0.3.1"));
        assert!(!version_matches("=0.3.1", "0.3.2"));
        assert!(version_matches("*", "5.0.0"));
        assert!(version_matches("0.1", "0.1.4"));
        assert!(!version_matches("^0.1", "garbage"));
    }

    #[test]
    fn dependencies_are_added_and_ordered() {
        let root = temp("order");
        write_mod(&root, "a_core", &manifest("core", "0.1.0", ""));
        write_mod(
            &root,
            "b_world",
            &manifest("overworld", "0.1.0", "core = \"^0.1\""),
        );
        write_mod(
            &root,
            "c_mobs",
            &manifest(
                "mobs",
                "0.1.0",
                "core = \"^0.1\"\noverworld = { version = \"^0.1\", optional = true }",
            ),
        );
        write_mod(&root, "unused", &manifest("unused", "0.1.0", ""));
        let set = ModSet::resolve("test", &["mobs", "overworld"], &root).expect("set");
        let ids: Vec<&str> = set.mods().iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["core", "overworld", "mobs"]);
        assert!(!set.contains("unused"));
        // Without overworld the optional dependency is skipped.
        let set = ModSet::resolve("test", &["mobs"], &root).expect("set");
        let ids: Vec<&str> = set.mods().iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["core", "mobs"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn errors_name_the_cause() {
        let root = temp("errors");
        write_mod(&root, "core", &manifest("core", "0.2.0", ""));
        write_mod(&root, "x", &manifest("x", "0.1.0", "core = \"^0.1\""));
        write_mod(&root, "y", &manifest("y", "0.1.0", "z = \"*\""));
        write_mod(&root, "z", &manifest("z", "0.1.0", "y = \"*\""));
        write_mod(&root, "w", &manifest("w", "0.1.0", "nether = \"^0.1\""));
        assert!(matches!(
            ModSet::resolve("t", &["x"], &root),
            Err(ModSetError::Version { .. })
        ));
        assert!(matches!(
            ModSet::resolve("t", &["y"], &root),
            Err(ModSetError::Cycle(_))
        ));
        let missing = ModSet::resolve("t", &["w"], &root).expect_err("missing");
        assert_eq!(
            missing.to_string(),
            "mod w needs mod \"nether\", which is not installed"
        );
        write_mod(&root, "core_copy", &manifest("core", "0.2.0", ""));
        assert!(matches!(
            ModSet::resolve("t", &["core"], &root),
            Err(ModSetError::Duplicate(_))
        ));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn later_mods_provide_data_files() {
        let root = temp("provide");
        write_mod(&root, "core", &manifest("core", "0.1.0", ""));
        write_mod(&root, "pack", &manifest("pack", "0.1.0", "core = \"^0.1\""));
        std::fs::write(root.join("core/data/a.toml"), "core").expect("write");
        std::fs::write(root.join("pack/data/a.toml"), "pack").expect("write");
        std::fs::write(root.join("core/data/b.toml"), "core").expect("write");
        let set = ModSet::resolve("t", &["pack"], &root).expect("set");
        assert_eq!(set.read("a.toml").expect("a"), "pack");
        assert_eq!(set.read("b.toml").expect("b"), "core");
        assert_eq!(set.read_optional("c.toml").expect("c"), None);
        assert!(matches!(
            set.read("c.toml"),
            Err(ModSetError::NotProvided(_))
        ));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn slots_have_one_provider() {
        let root = temp("slots");
        let with = |id: &str, provides: &str| {
            format!(
                "[mod]\nid = \"{id}\"\nversion = \"0.1.0\"\nprovides = [{provides}]\n\n[dependencies]\ncore = \"^0.1\"\n"
            )
        };
        write_mod(&root, "core", &manifest("core", "0.1.0", ""));
        write_mod(&root, "walk", &with("walk", "\"movement\""));
        write_mod(&root, "parkour", &with("parkour", "\"movement\""));
        write_mod(&root, "land", &with("land", "\"worldgen:overworld\""));
        std::fs::create_dir_all(root.join("land/data/worldgen")).expect("mkdir");
        std::fs::write(root.join("land/data/worldgen/overworld.toml"), "land").expect("w");
        let set = ModSet::resolve("t", &["walk", "land"], &root).expect("set");
        assert_eq!(set.slot(MOVEMENT_SLOT).map(|m| m.id.as_str()), Some("walk"));
        assert_eq!(
            set.read_slot(&worldgen_slot("overworld"), "worldgen/overworld.toml")
                .expect("read"),
            "land"
        );
        let err = ModSet::resolve("t", &["walk", "parkour"], &root).expect_err("two");
        assert!(matches!(err, ModSetError::SlotConflict { .. }), "{err}");
        assert!(err.to_string().contains("parkour") && err.to_string().contains("walk"));
        let only_land = ModSet::resolve("t", &["land"], &root).expect("set");
        let err = only_land.require_slot(MOVEMENT_SLOT).expect_err("missing");
        assert_eq!(
            err.to_string(),
            "build t has no mod that provides \"movement\" (add one, e.g. a movement or world module)"
        );
        write_mod(&root, "odd", &with("odd", "\"weather\""));
        assert!(matches!(
            ModSet::resolve("t", &["land"], &root),
            Err(ModSetError::Parse(_))
        ));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn shipped_builds_resolve() {
        let (mods, builds) = source_tree();
        let mut files: Vec<_> = std::fs::read_dir(&builds)
            .expect("builds/")
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "toml"))
            .collect();
        files.sort();
        assert!(!files.is_empty());
        for file in files {
            let set = ModSet::from_build(&file, &mods)
                .unwrap_or_else(|e| panic!("{}: {e}", file.display()));
            let _ = set
                .registry_builder()
                .and_then(|b| b.build().map_err(ModSetError::from))
                .unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        }
        let classic = source_build(DEFAULT_BUILD).expect("classic");
        assert!(classic.contains("core") && classic.contains("overworld"));
        assert!(classic.find("worldgen/overworld.toml").is_some());
        assert!(classic.find("movement/profile.toml").is_some());
        assert!(classic.require_slot(MOVEMENT_SLOT).is_ok());
        assert!(classic.require_slot(&worldgen_slot("overworld")).is_ok());
        // A build with another movement provider carries only that one.
        let agile = source_build("classic_agile").expect("agile");
        assert_eq!(
            agile.require_slot(MOVEMENT_SLOT).map(|m| m.id.as_str()),
            Ok("movement_agile")
        );
        assert!(!agile.contains("movement"));
    }
}
