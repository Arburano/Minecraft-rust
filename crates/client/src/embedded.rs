//! The server embedded in the client for single-player (T2-08), compiled
//! only with the `singleplayer` feature (T2-11, on by default).
//!
//! The client itself never needs the server crate: everything both ends
//! must agree on lives in `netcode`. A client built without this feature
//! plays only on servers (`Session::connect`); [`start`] then reports that
//! single-player is not part of the build and [`EmbeddedServer`] cannot
//! exist.

use std::net::SocketAddr;
use std::path::Path;

use crate::session::{SessionConfig, SessionError};
use crate::worlds::WorldInfo;

/// Whether this build can run single-player worlds.
pub const AVAILABLE: bool = cfg!(feature = "singleplayer");

#[cfg(feature = "singleplayer")]
mod imp {
    use super::*;
    use auth::AdmissionPolicy;
    use server::{DEFAULT_SIM_DISTANCE, Server, ServerConfig};

    /// A running embedded server.
    pub struct EmbeddedServer(Server);

    impl EmbeddedServer {
        /// Stops it (it saves the world first).
        pub fn shutdown(self) {
            self.0.shutdown();
        }
    }

    /// Starts the server for the world in `dir` on a free loopback port.
    pub fn start(
        dir: &Path,
        info: &WorldInfo,
        config: &SessionConfig,
    ) -> Result<(EmbeddedServer, SocketAddr), SessionError> {
        migrate_world(dir, info, &config.player_name)?;
        let view = config.radius + 2;
        let mut sc = ServerConfig::new(
            "127.0.0.1:0"
                .parse()
                .map_err(|e| SessionError::Data(format!("{e}")))?,
            AdmissionPolicy {
                allow_offline: true,
            },
            dir.to_owned(),
            config.mods.clone(),
        );
        sc.seed = Some(info.seed);
        sc.view_distance = view;
        sc.sim_distance = view.min(DEFAULT_SIM_DISTANCE);
        sc.threads = config.threads.max(1);
        sc.region_threads = 2;
        sc.bounds = config.bounds;
        sc.lan_announce = false;
        sc.server_name = info.name.clone();
        let server = Server::start(sc).map_err(|e| SessionError::Data(e.to_string()))?;
        let addr = server.local_addr();
        Ok((EmbeddedServer(server), addr))
    }

    /// Worlds saved before T2-08 keep the player in `world.toml`; the
    /// embedded server reads `players/<player id>.toml`. Copy it over once,
    /// so a single-player world opens where it was left (the seed is passed
    /// to the server, which writes its own `server.toml`).
    fn migrate_world(dir: &Path, info: &WorldInfo, name: &str) -> Result<(), SessionError> {
        let id = auth::Identity::offline(name)
            .map_err(|e| SessionError::Data(e.to_string()))?
            .id()
            .to_string();
        if server::players::load(dir, &id).is_some() {
            return Ok(());
        }
        if let Some(pos) = info.player.pos {
            server::players::save(
                dir,
                &id,
                &server::players::SavedPlayer {
                    pos,
                    yaw: info.player.yaw,
                    pitch: info.player.pitch,
                    flying: info.player.flying,
                    fly_speed: info.player.fly_speed,
                    dimension: None,
                },
            )
            .map_err(|e| SessionError::Data(format!("player file: {e}")))?;
        }
        Ok(())
    }
}

#[cfg(not(feature = "singleplayer"))]
mod imp {
    use super::*;

    /// No embedded server in this build: the type has no values.
    pub enum EmbeddedServer {}

    impl EmbeddedServer {
        /// Unreachable: no value exists.
        pub fn shutdown(self) {
            match self {}
        }
    }

    /// Single-player is not part of this build.
    pub fn start(
        _dir: &Path,
        _info: &WorldInfo,
        _config: &SessionConfig,
    ) -> Result<(EmbeddedServer, SocketAddr), SessionError> {
        Err(SessionError::Data(
            "single-player is not part of this build (built without the `singleplayer` feature); join a server instead"
                .into(),
        ))
    }
}

pub use imp::{EmbeddedServer, start};
