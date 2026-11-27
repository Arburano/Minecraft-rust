//! The storage thread: owns the region files (`world::WorldStore`), so disk
//! reads and writes never stall the world tick. The world thread asks for
//! columns and gets answers back on a channel; saves are fire-and-forget.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Sender, channel};
use std::thread::JoinHandle;

use serde::{Deserialize, Serialize};
use world::storage::{ColumnRecord, StorageError, WorldStore};
use world::{Column, ColumnPos, Registry, WorldBounds};

use crate::world_thread::WorldMsg;

/// A player's saved state (`players/<id>.toml`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PlayerSave {
    /// Feet position.
    pub pos: [f64; 3],
    /// View yaw in radians.
    pub yaw: f64,
    /// View pitch in radians.
    pub pitch: f64,
    /// Flying.
    pub flying: bool,
}

pub(crate) enum StorageMsg {
    /// Load a column; the answer is a [`Loaded`].
    Load(ColumnPos),
    /// Write columns (edited ones).
    Save(Vec<ColumnRecord>),
    /// Write a player's state.
    SavePlayer([u8; 16], PlayerSave),
    /// Answer once everything before it is written.
    Flush(Sender<()>),
}

/// The answer to [`StorageMsg::Load`]: the saved column, `None` if it was
/// never saved, or the error text (the world then generates it).
pub(crate) struct Loaded {
    pub pos: ColumnPos,
    pub column: Result<Option<Column>, String>,
}

pub(crate) struct StorageThread {
    pub tx: Sender<StorageMsg>,
    pub handle: Option<JoinHandle<()>>,
}

fn player_path(dir: &Path, id: [u8; 16]) -> PathBuf {
    let hex: String = id.iter().map(|b| format!("{b:02x}")).collect();
    dir.join("players").join(format!("{hex}.toml"))
}

/// Reads a player's saved state (small file, read once at join).
pub(crate) fn load_player(dir: &Path, id: [u8; 16]) -> Option<PlayerSave> {
    let text = std::fs::read_to_string(player_path(dir, id)).ok()?;
    let save: PlayerSave = toml::from_str(&text).ok()?;
    save.pos.iter().all(|v| v.is_finite()).then_some(save)
}

fn save_player(dir: &Path, id: [u8; 16], save: &PlayerSave) -> Result<(), String> {
    let path = player_path(dir, id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let text = toml::to_string(save).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

impl StorageThread {
    /// Opens the store (on the caller's thread, so errors surface at start)
    /// and starts the thread. Answers to loads go to the world thread's own
    /// channel, so they wake it at once.
    pub fn start(
        dir: &Path,
        registry: Arc<Registry>,
        bounds: WorldBounds,
        world: Sender<WorldMsg>,
    ) -> Result<Self, StorageError> {
        let mut store = WorldStore::open(dir, &registry, bounds)?;
        let (tx, rx) = channel::<StorageMsg>();
        let dir = dir.to_owned();
        let handle = std::thread::Builder::new()
            .name("storage".into())
            .spawn(move || {
                // Saves wait in memory only while a batch is being written.
                let mut players: HashMap<[u8; 16], PlayerSave> = HashMap::new();
                while let Ok(msg) = rx.recv() {
                    match msg {
                        StorageMsg::Load(pos) => {
                            let column = store
                                .load_column(pos, &registry)
                                .map(|c| c.map(|(col, _)| col))
                                .map_err(|e| e.to_string());
                            if world
                                .send(WorldMsg::Loaded(Loaded { pos, column }))
                                .is_err()
                            {
                                break;
                            }
                        }
                        StorageMsg::Save(records) => {
                            if let Err(e) = store.save_all(&records) {
                                eprintln!("storage: saving {} columns failed: {e}", records.len());
                            }
                        }
                        StorageMsg::SavePlayer(id, save) => {
                            players.insert(id, save);
                            // Coalesce bursts (many players leaving at once).
                            for (id, save) in players.drain() {
                                if let Err(e) = save_player(&dir, id, &save) {
                                    eprintln!("storage: saving a player failed: {e}");
                                }
                            }
                        }
                        StorageMsg::Flush(done) => {
                            let _ = done.send(());
                        }
                    }
                }
            })
            .map_err(StorageError::Io)?;
        Ok(Self {
            tx,
            handle: Some(handle),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_save_roundtrip() {
        let dir = std::env::temp_dir().join(format!("server-player-{}", std::process::id()));
        let id = [7u8; 16];
        let save = PlayerSave {
            pos: [1.5, 70.0, -3.25],
            yaw: 1.0,
            pitch: -0.5,
            flying: true,
        };
        assert_eq!(load_player(&dir, id), None);
        save_player(&dir, id, &save).expect("writes");
        assert_eq!(load_player(&dir, id), Some(save));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
