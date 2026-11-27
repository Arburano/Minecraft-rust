//! Client settings, persisted as `settings.toml` in the data directory.

use std::path::{Path, PathBuf};

use render::selection::GpuPreference;
use render::terrain::profiles::QualityProfile;
use serde::{Deserialize, Serialize};

/// Everything the settings screen changes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Name for offline play (development and private tests only).
    pub offline_name: Option<String>,
    /// Quality profile (render distance 8 / 12 / 16 chunks).
    pub quality: QualityProfile,
    /// GPU and backend override (T1-05).
    pub gpu: GpuPreference,
    /// Mouse look speed, radians per pixel × 1000.
    pub mouse_sensitivity: f32,
    /// Vertical field of view in degrees.
    pub fov_degrees: f32,
    /// Autosave interval in seconds.
    pub autosave_seconds: u32,
    /// Worker threads for world jobs (0 = all cores but one).
    pub worker_threads: usize,
    /// Textures from the player's own installation (T1-14).
    pub own_assets: OwnAssets,
    /// Enabled resource packs (ids = folder or zip names), top first (ADR-0014).
    pub resource_packs: Vec<String>,
    /// The shader pack in use (`None` = the built-in look).
    pub shader_pack: Option<String>,
    /// Shader pack option values: pack id → option id → value.
    pub shader_options: std::collections::BTreeMap<String, std::collections::BTreeMap<String, f32>>,
    /// The multiplayer server list (T2-08).
    pub servers: Vec<SavedServer>,
}

/// A server in the multiplayer list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedServer {
    /// Display name.
    pub name: String,
    /// `host[:port]`.
    pub address: String,
}

/// Where the player's own licensed installation is (read locally only).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OwnAssets {
    /// Use its textures (fallback textures fill any gap).
    pub enabled: bool,
    /// The installation folder; `None` = look in the usual places.
    pub folder: Option<PathBuf>,
    /// Version to read; `None` = the newest release.
    pub version: Option<String>,
}

impl Default for OwnAssets {
    fn default() -> Self {
        Self {
            enabled: true,
            folder: None,
            version: None,
        }
    }
}

impl OwnAssets {
    /// The folder to read: the configured one, else the first usual place that
    /// has installed versions.
    #[must_use]
    pub fn resolve_folder(&self) -> Option<PathBuf> {
        self.folder.clone().or_else(|| {
            assets::local_install::default_dirs()
                .into_iter()
                .find(|d| assets::local_install::has_versions(d))
        })
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            offline_name: None,
            quality: QualityProfile::Default,
            gpu: GpuPreference::default(),
            mouse_sensitivity: 2.0,
            fov_degrees: 70.0,
            autosave_seconds: 30,
            worker_threads: 0,
            own_assets: OwnAssets::default(),
            resource_packs: Vec::new(),
            shader_pack: None,
            shader_options: std::collections::BTreeMap::new(),
            servers: Vec::new(),
        }
    }
}

impl Settings {
    /// Reads `path`. A missing file gives the defaults; a damaged one gives
    /// the defaults and the parse error (the caller shows it, the old file is
    /// overwritten on the next save).
    #[must_use]
    pub fn load(path: &Path) -> (Self, Option<String>) {
        match std::fs::read_to_string(path) {
            Err(_) => (Self::default(), None),
            Ok(text) => match toml::from_str(&text) {
                Ok(s) => (s, None),
                Err(e) => (Self::default(), Some(format!("{}: {e}", path.display()))),
            },
        }
    }

    /// Writes `path` (through a temporary file, so a crash never leaves half a file).
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let text = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(tmp, path)
    }

    /// Worker threads to start.
    #[must_use]
    pub fn threads(&self) -> usize {
        if self.worker_threads > 0 {
            self.worker_threads
        } else {
            std::thread::available_parallelism().map_or(3, |n| n.get().saturating_sub(1).max(1))
        }
    }
}

/// The default per-user data directory (settings, worlds, caches).
#[must_use]
pub fn default_data_dir() -> PathBuf {
    // OPEN-QUESTION: 15 — the folder carries the working title until the game has a name.
    const APP: &str = "voxel-engine";
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    if cfg!(windows) {
        env("APPDATA").map_or_else(|| PathBuf::from(APP), |p| p.join(APP))
    } else if cfg!(target_os = "macos") {
        env("HOME").map_or_else(
            || PathBuf::from(APP),
            |p| p.join("Library/Application Support").join(APP),
        )
    } else {
        env("XDG_DATA_HOME")
            .or_else(|| env("HOME").map(|h| h.join(".local/share")))
            .map_or_else(|| PathBuf::from(APP), |p| p.join(APP))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_damaged_file() {
        let dir = std::env::temp_dir().join(format!("client-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("settings.toml");
        let mut s = Settings {
            offline_name: Some("dev_01".into()),
            quality: QualityProfile::High,
            ..Settings::default()
        };
        s.gpu.adapter_name = Some("vega".into());
        s.save(&path).expect("save");
        assert_eq!(Settings::load(&path), (s, None));
        std::fs::write(&path, "quality = 5").expect("write");
        let (d, err) = Settings::load(&path);
        assert_eq!(d, Settings::default());
        assert!(err.is_some());
        assert_eq!(
            Settings::load(&dir.join("missing.toml")),
            (Settings::default(), None)
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
