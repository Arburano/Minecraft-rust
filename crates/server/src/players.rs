//! Saved player state: `<world>/players/<player id>.toml`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// What the server remembers about a player between sessions.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SavedPlayer {
    /// Feet position.
    pub pos: [f64; 3],
    /// View yaw, radians.
    pub yaw: f64,
    /// View pitch, radians.
    pub pitch: f64,
    /// Flying.
    pub flying: bool,
    /// Fly speed multiplier (0 = ×1; older files have none).
    #[serde(default)]
    pub fly_speed: f64,
    /// The dimension the player is in (T4-02; absent: the overworld).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dimension: Option<String>,
}

fn path(world: &Path, id: &str) -> PathBuf {
    world.join("players").join(format!("{id}.toml"))
}

/// Loads a player's state; `None` for a new player or an unreadable file.
#[must_use]
pub fn load(world: &Path, id: &str) -> Option<SavedPlayer> {
    let text = std::fs::read_to_string(path(world, id)).ok()?;
    let p: SavedPlayer = toml::from_str(&text).ok()?;
    p.pos.iter().all(|v| v.is_finite()).then_some(p)
}

/// Saves a player's state (write to a temporary file, then rename).
pub fn save(world: &Path, id: &str, player: &SavedPlayer) -> std::io::Result<()> {
    let p = path(world, id);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = toml::to_string(player).map_err(std::io::Error::other)?;
    let tmp = p.with_extension("toml.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(tmp, p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_and_load() {
        let dir = std::env::temp_dir().join(format!("server-players-{}", std::process::id()));
        let p = SavedPlayer {
            pos: [1.5, 70.0, -3.25],
            yaw: 0.5,
            pitch: -0.1,
            flying: true,
            fly_speed: 2.0,
            dimension: Some("moon".into()),
        };
        save(&dir, "abc", &p).expect("save");
        assert_eq!(load(&dir, "abc"), Some(p.clone()));
        // The overworld is not written, and files from before T4-02 load.
        let old = SavedPlayer {
            dimension: None,
            ..p
        };
        save(&dir, "old", &old).expect("save");
        let text = std::fs::read_to_string(dir.join("players/old.toml")).unwrap_or_default();
        assert!(!text.contains("dimension"), "{text}");
        assert_eq!(load(&dir, "old"), Some(old));
        assert_eq!(load(&dir, "missing"), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
