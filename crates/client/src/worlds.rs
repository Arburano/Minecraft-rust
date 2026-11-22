//! Single-player worlds on disk: `<data>/worlds/<folder>/` holds `world.toml`
//! (name, seed, generator, player) next to the T1-02 store (`world.redb`,
//! `region/`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Saved player state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PlayerSave {
    /// Feet position; `None` until the player has spawned once.
    pub pos: Option<[f64; 3]>,
    /// View yaw (radians).
    pub yaw: f64,
    /// View pitch (radians).
    pub pitch: f64,
    /// Flying when saved.
    #[serde(default)]
    pub flying: bool,
    /// Fly speed multiplier (mouse wheel); 0 = ×1.
    #[serde(default)]
    pub fly_speed: f64,
}

/// `world.toml`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorldInfo {
    /// Display name.
    pub name: String,
    /// Generation seed.
    pub seed: u64,
    /// Generation mode (`normal` now; `extended` with T1-17).
    pub generator: String,
    /// Creation time, seconds since the Unix epoch.
    pub created: u64,
    /// Last time played, seconds since the Unix epoch.
    pub last_played: u64,
    /// The player.
    #[serde(default)]
    pub player: PlayerSave,
}

/// Errors of world management.
#[derive(Debug, thiserror::Error)]
pub enum WorldsError {
    /// File system error.
    #[error("world files: {0}")]
    Io(#[from] std::io::Error),
    /// `world.toml` is damaged.
    #[error("world.toml: {0}")]
    Parse(String),
    /// The name is empty after trimming.
    #[error("the world needs a name")]
    EmptyName,
}

/// Seconds since the Unix epoch.
#[must_use]
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl WorldInfo {
    /// Reads `<dir>/world.toml`.
    pub fn load(dir: &Path) -> Result<Self, WorldsError> {
        let text = std::fs::read_to_string(dir.join("world.toml"))?;
        toml::from_str(&text).map_err(|e| WorldsError::Parse(e.to_string()))
    }

    /// Writes `<dir>/world.toml` atomically.
    pub fn save(&self, dir: &Path) -> Result<(), WorldsError> {
        let text = toml::to_string_pretty(self).map_err(|e| WorldsError::Parse(e.to_string()))?;
        let tmp = dir.join("world.toml.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(tmp, dir.join("world.toml"))?;
        Ok(())
    }
}

/// Worlds under `root`, most recently played first. Folders without a
/// readable `world.toml` are skipped.
#[must_use]
pub fn list(root: &Path) -> Vec<(PathBuf, WorldInfo)> {
    let mut out: Vec<(PathBuf, WorldInfo)> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            WorldInfo::load(&p).ok().map(|i| (p, i))
        })
        .collect();
    out.sort_by(|a, b| {
        b.1.last_played
            .cmp(&a.1.last_played)
            .then(a.1.name.cmp(&b.1.name))
    });
    out
}

/// Creates a world folder with a name-derived, unique folder name.
pub fn create(root: &Path, name: &str, seed: u64) -> Result<PathBuf, WorldsError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(WorldsError::EmptyName);
    }
    let base: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(40)
        .collect();
    std::fs::create_dir_all(root)?;
    let mut dir = root.join(&base);
    let mut n = 2;
    while dir.exists() {
        dir = root.join(format!("{base}_{n}"));
        n += 1;
    }
    std::fs::create_dir_all(&dir)?;
    let t = now();
    WorldInfo {
        name: name.to_owned(),
        seed,
        generator: "normal".into(),
        created: t,
        last_played: t,
        player: PlayerSave::default(),
    }
    .save(&dir)?;
    Ok(dir)
}

/// Deletes a world folder (only folders holding a `world.toml`).
pub fn delete(dir: &Path) -> Result<(), WorldsError> {
    WorldInfo::load(dir)?;
    std::fs::remove_dir_all(dir)?;
    Ok(())
}

/// A seed from text: a number is used as is, other text is hashed (FNV-1a),
/// empty text gives a time-based seed.
#[must_use]
pub fn seed_from_text(text: &str) -> u64 {
    let t = text.trim();
    if t.is_empty() {
        return now().wrapping_mul(0x9e37_79b9_7f4a_7c15);
    }
    if let Ok(n) = t.parse::<u64>() {
        return n;
    }
    t.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_list_delete() {
        let root = std::env::temp_dir().join(format!("client-worlds-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let a = create(&root, "My world!", 7).expect("create");
        let b = create(&root, "My world!", 8).expect("create again");
        assert_ne!(a, b, "unique folders");
        assert!(matches!(
            create(&root, "  ", 1),
            Err(WorldsError::EmptyName)
        ));
        let mut info = WorldInfo::load(&b).expect("load");
        info.last_played += 100;
        info.player.pos = Some([1.5, 70.0, -3.25]);
        info.save(&b).expect("save");
        let l = list(&root);
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].0, b, "most recent first");
        assert_eq!(l[0].1.player.pos, Some([1.5, 70.0, -3.25]));
        delete(&a).expect("delete");
        assert_eq!(list(&root).len(), 1);
        assert_eq!(seed_from_text("42"), 42);
        assert_eq!(seed_from_text("hello"), seed_from_text("hello"));
        let _ = std::fs::remove_dir_all(root);
    }
}
