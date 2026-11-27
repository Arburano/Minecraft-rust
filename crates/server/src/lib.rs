//! Dedicated server: world, player join and column streaming (T2-02).
//!
//! Spec:
//! - `docs/architecture/networking-protocol.md` (handshake, channels)
//! - `docs/architecture/server-regions.md` (one region for now; T2-06)
//! - `docs/architecture/auth-and-assets.md` ("Offline mode"), `docs/legal.md` rule 7
//!
//! Threads: a small tokio runtime for QUIC (one task per connection), the
//! world thread ([`world`]: map, players, what each client was sent), a
//! generation job pool and the storage thread ([`storage`]). They talk
//! through channels only.

pub mod conn;
pub mod interest;
pub mod storage;
pub mod world_thread;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Sender, channel};
use std::thread::JoinHandle;
use std::time::Duration;

use auth::AdmissionPolicy;
use bytes::Bytes;
use protocol::{Registry as RegistryMsg, ServerControl, ServerInfo};
use quinn::Endpoint;
use quinn::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use serde::{Deserialize, Serialize};
use world::registry::blocks_from_dir;
use world::{RegistryBuilder, WorldBounds};
use worldgen::{GenContext, Pipeline, WorldgenConfig};

use crate::world_thread::{WorldConfig, WorldMsg};

/// Default game port.
pub const DEFAULT_PORT: u16 = 47_777;

/// Default stream radius in columns.
pub const DEFAULT_VIEW_DISTANCE: u8 = 12;

/// How to run a server.
#[derive(Clone, Debug)]
pub struct ServerOptions {
    /// Listen address.
    pub listen: SocketAddr,
    /// Who may join.
    pub policy: AdmissionPolicy,
    /// World directory (region files, `server.toml`, `players/`).
    pub world_dir: PathBuf,
    /// The `core` mod's data folder (`mods/core/data`).
    pub data: PathBuf,
    /// Seed for a new world (an existing world keeps its own).
    pub seed: Option<u64>,
    /// Stream radius in columns (Chebyshev). Clients light and mesh up to
    /// two columns less.
    pub view_distance: u8,
    /// Generation workers.
    pub threads: usize,
    /// Players at most.
    pub max_players: usize,
    /// Server name shown to clients.
    pub name: String,
    /// Message of the day.
    pub motd: String,
    /// Vertical bounds of the dimension.
    pub bounds: WorldBounds,
}

impl ServerOptions {
    /// Defaults for a world in `world_dir` with mod data in `data`, listening
    /// on localhost.
    #[must_use]
    pub fn new(world_dir: PathBuf, data: PathBuf) -> Self {
        Self {
            listen: SocketAddr::from(([127, 0, 0, 1], DEFAULT_PORT)),
            policy: AdmissionPolicy::default(),
            world_dir,
            data,
            seed: None,
            view_distance: DEFAULT_VIEW_DISTANCE,
            threads: std::thread::available_parallelism()
                .map_or(2, |n| n.get().saturating_sub(2).max(1)),
            max_players: 32,
            name: "Server".into(),
            motd: String::new(),
            bounds: WorldBounds::default(),
        }
    }
}

/// The server could not start.
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    /// Mod data could not be read.
    #[error("mod data: {0}")]
    Data(String),
    /// The world directory could not be read or written.
    #[error("world: {0}")]
    World(String),
    /// Networking failed.
    #[error("network: {0}")]
    Net(String),
}

/// `server.toml` in the world directory.
#[derive(Debug, Serialize, Deserialize)]
struct WorldMeta {
    seed: u64,
}

fn world_seed(dir: &Path, wanted: Option<u64>) -> Result<u64, ServerError> {
    let path = dir.join("server.toml");
    let err = |e: &dyn std::fmt::Display| ServerError::World(format!("{}: {e}", path.display()));
    if let Ok(text) = std::fs::read_to_string(&path) {
        let meta: WorldMeta = toml::from_str(&text).map_err(|e| err(&e))?;
        return Ok(meta.seed);
    }
    let seed = wanted.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64)
    });
    std::fs::create_dir_all(dir).map_err(|e| err(&e))?;
    let text = toml::to_string(&WorldMeta { seed }).map_err(|e| err(&e))?;
    std::fs::write(&path, text).map_err(|e| err(&e))?;
    Ok(seed)
}

/// A self-signed certificate for an offline server (development and private
/// LAN tests; clients accept it only in offline mode).
// OPEN-QUESTION: 16
fn self_signed() -> Result<(CertificateDer<'static>, PrivateKeyDer<'static>), ServerError> {
    let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
        .map_err(|e| ServerError::Net(e.to_string()))?;
    let cert = certified.cert.der().clone();
    let key = PrivateKeyDer::Pkcs8(certified.signing_key.serialize_der().into());
    Ok((cert, key))
}

/// A running server. Dropping it stops the server and saves.
pub struct ServerHandle {
    addr: SocketAddr,
    cert: CertificateDer<'static>,
    endpoint: Endpoint,
    runtime: Option<tokio::runtime::Runtime>,
    world: Sender<WorldMsg>,
    world_thread: Option<JoinHandle<()>>,
}

/// Starts a server: loads mod data, opens the world and starts listening.
pub fn start(opts: ServerOptions) -> Result<ServerHandle, ServerError> {
    let data_err = |e: &dyn std::fmt::Display| ServerError::Data(e.to_string());
    let mut b = RegistryBuilder::new();
    b.add_mod(
        "core",
        blocks_from_dir(&opts.data.join("blocks")).map_err(|e| data_err(&e))?,
    );
    let registry = Arc::new(b.build().map_err(|e| data_err(&e))?);
    let src = std::fs::read_to_string(opts.data.join("worldgen/overworld.toml"))
        .map_err(|e| data_err(&e))?;
    let gen_config = WorldgenConfig::from_toml(&src, &registry).map_err(|e| data_err(&e))?;
    let physics_src =
        std::fs::read_to_string(opts.data.join("player/physics.toml")).map_err(|e| data_err(&e))?;
    let physics = sim::physics::PhysicsConfig::from_toml(&physics_src).map_err(|e| data_err(&e))?;
    let swim = physics.swim_ids(&registry).map_err(|e| data_err(&e))?;

    let seed = world_seed(&opts.world_dir, opts.seed)?;
    let (world_tx, world_rx) = channel();
    let storage = storage::StorageThread::start(
        &opts.world_dir,
        Arc::clone(&registry),
        opts.bounds,
        world_tx.clone(),
    )
    .map_err(|e| ServerError::World(e.to_string()))?;

    let registry_frame = ServerControl::Registry(RegistryMsg {
        id_table: registry.id_table().names,
        entity_kinds: Vec::new(),
        component_schemas: Vec::new(),
    })
    .encode_frame()
    .map_err(|e| ServerError::Data(e.to_string()))?;

    let view = i32::from(opts.view_distance.clamp(3, 32));
    let cfg = WorldConfig {
        registry: Arc::clone(&registry),
        bounds: opts.bounds,
        pipeline: Arc::new(Pipeline::overworld()),
        context: Arc::new(GenContext::new(seed, gen_config, opts.bounds)),
        physics,
        swim,
        view,
        threads: opts.threads,
        max_players: opts.max_players,
        dir: opts.world_dir.clone(),
    };
    let me = world_tx.clone();
    let world_thread = std::thread::Builder::new()
        .name("world".into())
        .spawn(move || world_thread::run(cfg, storage, world_rx, me))
        .map_err(|e| ServerError::World(e.to_string()))?;

    let (cert, key) = self_signed()?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("net-server")
        .enable_all()
        .build()
        .map_err(|e| ServerError::Net(e.to_string()))?;
    let endpoint = {
        let _guard = runtime.enter();
        net::server_endpoint_with_transport(
            opts.listen,
            cert.clone(),
            key,
            net::game_server_transport(),
        )
        .map_err(|e| ServerError::Net(e.to_string()))?
    };
    let addr = endpoint
        .local_addr()
        .map_err(|e| ServerError::Net(e.to_string()))?;
    let shared = Arc::new(conn::Shared {
        world: world_tx.clone(),
        policy: opts.policy,
        info: ServerInfo {
            name: opts.name.clone(),
            motd: opts.motd.clone(),
            tick_rate: (1000 / world_thread::TICK.as_millis()) as u16,
            view_distance: view as u8,
        },
        registry_frame: Bytes::from(registry_frame),
    });
    runtime.spawn(conn::accept_loop(endpoint.clone(), shared));
    Ok(ServerHandle {
        addr,
        cert,
        endpoint,
        runtime: Some(runtime),
        world: world_tx,
        world_thread: Some(world_thread),
    })
}

impl ServerHandle {
    /// The address the server listens on (the real port if 0 was asked).
    #[must_use]
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// The server's (self-signed) certificate, for clients that pin it.
    #[must_use]
    pub fn certificate(&self) -> &CertificateDer<'static> {
        &self.cert
    }

    /// Blocks until the world thread ends (it runs until the process is
    /// stopped).
    pub fn wait(mut self) {
        if let Some(t) = self.world_thread.take() {
            let _ = t.join();
        }
    }

    /// Disconnects everyone, saves and stops.
    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        self.endpoint
            .close(conn::close::SHUTDOWN.into(), b"server stopping");
        let (tx, rx) = channel();
        if self.world.send(WorldMsg::Shutdown(tx)).is_ok() {
            let _ = rx.recv_timeout(Duration::from_secs(15));
        }
        if let Some(t) = self.world_thread.take() {
            let _ = t.join();
        }
        if let Some(rt) = self.runtime.take() {
            rt.shutdown_background();
        }
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.stop();
    }
}
