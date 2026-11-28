//! Dedicated server: world, player join and column streaming (T2-02).
//!
//! Spec:
//! - `docs/architecture/networking-protocol.md` (handshake, channels)
//! - `docs/architecture/server-regions.md` (one region for now; T2-06)
//! - `docs/architecture/auth-and-assets.md` ("Offline mode"), `docs/legal.md` rule 7
//!
//! Threads: a small tokio runtime for QUIC (accept loop, one task per
//! connection, one task per column stream), the world thread (20 TPS, owns
//! the world map), the generation job pool and the storage thread.

pub mod capabilities;
// Shared with the client (T2-11); re-exported so `server::input` and
// `server::replication` keep working.
pub use netcode::{input, replication};
pub mod columns;
pub mod conn;
pub mod entities;
pub mod game;
pub mod metrics;
pub mod partition;
pub mod players;
pub mod region;
pub mod storage;
pub mod zones;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use auth::AdmissionPolicy;
use quinn::rustls::pki_types::PrivateKeyDer;
use serde::{Deserialize, Serialize};
use sim::physics::PhysicsConfig;
use world::storage::WorldStore;
use world::{ModSet, WorldBounds};
use worldgen::{GenContext, WorldgenConfig};

use crate::conn::ConnContext;
use crate::game::{World, WorldSetup};
use crate::storage::Storage;

/// Default game port.
pub const DEFAULT_PORT: u16 = 47_777;
/// Default streamed column radius: render distance 12 plus the 2 columns the
/// client needs around it to light and mesh the border (`loading.rs`).
pub const DEFAULT_VIEW_DISTANCE: i32 = 14;
/// Default simulated column radius (`server-regions.md`: 6–8 chunks).
pub const DEFAULT_SIM_DISTANCE: i32 = 8;

/// Everything the server needs to start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerConfig {
    /// UDP address for QUIC.
    pub listen: SocketAddr,
    /// Who may join.
    pub policy: AdmissionPolicy,
    /// The world folder (created if missing).
    pub world_dir: PathBuf,
    /// The mods of the build the server runs (ADR-0017): blocks from every
    /// mod, generation, player physics and entity limits from the mods that
    /// provide them.
    pub mods: ModSet,
    /// Seed for a new world; an existing world keeps its own.
    pub seed: Option<u64>,
    /// Streamed column radius (≥ 10: render distance ≥ 8, PRD §14).
    pub view_distance: i32,
    /// Generation worker threads.
    pub threads: usize,
    /// Region tick threads (`server-regions.md`: ≈ 80% of the cores left
    /// after network, storage and generation).
    pub region_threads: usize,
    /// Simulated column radius around each player (6–8, `server-regions.md`);
    /// the rest of the view radius is only streamed.
    pub sim_distance: i32,
    /// Test entities spawned around the world spawn at start (T2-07).
    pub test_walkers: usize,
    /// Vertical extent of the world.
    pub bounds: WorldBounds,
    /// Announce the server on the LAN (only offline servers on a private
    /// non-loopback address ever do; legal.md rule 7).
    pub lan_announce: bool,
    /// Display name.
    pub server_name: String,
    /// Message of the day.
    pub motd: String,
    /// Factories of the optional subsystems (ADR-0018); only those of the
    /// build's capabilities start.
    pub subsystems: capabilities::SubsystemRegistry,
}

impl ServerConfig {
    /// Defaults for a world in `world_dir`.
    #[must_use]
    pub fn new(
        listen: SocketAddr,
        policy: AdmissionPolicy,
        world_dir: PathBuf,
        mods: ModSet,
    ) -> Self {
        Self {
            listen,
            policy,
            world_dir,
            mods,
            seed: None,
            view_distance: DEFAULT_VIEW_DISTANCE,
            threads: std::thread::available_parallelism()
                .map_or(2, |n| n.get().saturating_sub(2).max(1)),
            region_threads: std::thread::available_parallelism()
                .map_or(2, |n| (n.get() * 4 / 5).max(1)),
            sim_distance: DEFAULT_SIM_DISTANCE,
            test_walkers: 0,
            bounds: WorldBounds::default(),
            lan_announce: true,
            server_name: "Voxel Engine server".into(), // OPEN-QUESTION: 15
            motd: String::new(),
            subsystems: capabilities::SubsystemRegistry::builtin(),
        }
    }
}

/// Errors while starting the server.
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    /// Mod data could not be read.
    #[error("mod data: {0}")]
    Data(String),
    /// The world folder could not be used.
    #[error("world: {0}")]
    World(String),
    /// Networking could not start.
    #[error("network: {0}")]
    Net(String),
}

/// `<world>/server.toml`: what the server remembers about its world.
#[derive(Serialize, Deserialize)]
struct WorldMeta {
    seed: u64,
    /// The overworld generator that made the world: `<mod>:<engine>`, e.g.
    /// `overworld:classic` (absent in worlds made before engines existed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    generator: Option<String>,
    /// Gameplay slot → provider (`mod@version`) the world was last opened
    /// with (ADR-0013, T1-18).
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    slots: std::collections::BTreeMap<String, String>,
    /// Engine capabilities whose data this world may hold (ADR-0018): every
    /// capability of every build that opened it. A build without one of them
    /// refuses the world.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    capabilities: Vec<String>,
}

/// The overworld generator of a build: the mod that fills the overworld's
/// generation slot and its engine.
fn generator_id(mods: &ModSet, engine: &worldgen::Engine) -> String {
    let provider = mods
        .slot(&world::modset::worldgen_slot("overworld"))
        .map_or("unknown", |m| m.id.as_str());
    format!("{provider}:{}", engine.name())
}

/// The slot choices of a build, as stored in `server.toml`.
fn slot_choices(mods: &ModSet) -> std::collections::BTreeMap<String, String> {
    mods.mods()
        .iter()
        .flat_map(|m| {
            m.provides
                .iter()
                .map(move |s| (s.clone(), format!("{}@{}", m.id, m.version)))
        })
        .collect()
}

/// Reads or creates `server.toml`: the seed, the generator, which must
/// match the build's (new terrain from another generator would not fit the
/// saved terrain at the borders), and the slot choices. A changed movement
/// provider is allowed (nothing in the world depends on it) and reported in
/// `notices`.
fn world_seed(
    config: &ServerConfig,
    generator: &str,
    notices: &mut Vec<String>,
) -> Result<u64, ServerError> {
    let slots = slot_choices(&config.mods);
    let path = config.world_dir.join("server.toml");
    let write = |meta: &WorldMeta| {
        let text = toml::to_string(meta).map_err(|e| ServerError::World(e.to_string()))?;
        std::fs::write(&path, text).map_err(|e| ServerError::World(e.to_string()))
    };
    if let Ok(text) = std::fs::read_to_string(&path) {
        let mut meta: WorldMeta = toml::from_str(&text)
            .map_err(|e| ServerError::World(format!("{}: {e}", path.display())))?;
        match &meta.generator {
            Some(g) if g != generator => {
                return Err(ServerError::World(format!(
                    "this world was generated by {g}, but build {} generates with {generator}; \
                     open it with a build that has the same overworld module",
                    config.mods.build()
                )));
            }
            Some(_) => {}
            None => meta.generator = Some(generator.to_owned()),
        }
        let capabilities =
            capabilities::check_world(&meta.capabilities, &config.mods, &config.world_dir)
                .map_err(ServerError::World)?
                .names();
        for (slot, now) in &slots {
            if let Some(before) = meta.slots.get(slot)
                && before != now
            {
                notices.push(format!(
                    "slot {slot}: this world was last opened with {before}, now {now}"
                ));
            }
        }
        if meta.slots != slots
            || meta.generator.as_deref() != Some(generator)
            || meta.capabilities != capabilities
        {
            meta.slots = slots;
            meta.capabilities = capabilities;
            write(&meta)?;
        }
        return Ok(meta.seed);
    }
    let seed = config.seed.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64)
    });
    write(&WorldMeta {
        seed,
        generator: Some(generator.to_owned()),
        slots,
        capabilities: config.mods.capabilities().names(),
    })?;
    Ok(seed)
}

/// A private LAN address (RFC 1918, IPv6 unique local) — not loopback: the
/// address an offline server may announce on.
fn lan_private(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => v4.is_private(),
        std::net::IpAddr::V6(v6) => (v6.segments()[0] & 0xfe00) == 0xfc00,
    }
}

/// Broadcasts the LAN announcement every few seconds until `stop`.
fn start_announcer(port: u16, name: String, stop: Arc<AtomicBool>) {
    let packet = protocol::lan::LanAnnouncement {
        protocol: protocol::PROTOCOL_VERSION,
        port,
        name,
    }
    .encode();
    let _ = std::thread::Builder::new()
        .name("lan-announce".into())
        .spawn(move || {
            let Ok(socket) = std::net::UdpSocket::bind("0.0.0.0:0") else {
                return;
            };
            if socket.set_broadcast(true).is_err() {
                return;
            }
            let target =
                std::net::SocketAddr::from(([255, 255, 255, 255], protocol::lan::LAN_PORT));
            while !stop.load(Ordering::Relaxed) {
                let _ = socket.send_to(&packet, target);
                std::thread::sleep(Duration::from_secs(protocol::lan::LAN_INTERVAL_SECS));
            }
        });
}

/// A running server. Dropping it without [`Server::shutdown`] stops it too,
/// but without waiting for the save.
pub struct Server {
    addr: SocketAddr,
    log: Arc<metrics::TickLog>,
    stop: Arc<AtomicBool>,
    world: Option<JoinHandle<()>>,
    endpoint: quinn::Endpoint,
    runtime: Option<tokio::runtime::Runtime>,
    notices: Vec<String>,
    capabilities: world::Capabilities,
    zones: Option<zones::ZoneControl>,
}

impl Server {
    /// Loads mod data, opens the world and starts listening.
    pub fn start(config: ServerConfig) -> Result<Self, ServerError> {
        let data = |e: &dyn std::fmt::Display| ServerError::Data(e.to_string());
        let mods = &config.mods;
        // Capabilities first: a binary without one fails before any work.
        mods.check_compiled(capabilities::compiled(), "server")
            .map_err(|e| data(&e))?;
        let b = mods.registry_builder().map_err(|e| data(&e))?;
        let registry = Arc::new(b.build().map_err(|e| data(&e))?);
        // Slots (ADR-0013): each file comes from the one mod that fills the
        // slot. The only dimension so far is the overworld.
        let src = mods
            .read_slot(
                &world::modset::worldgen_slot("overworld"),
                "worldgen/overworld.toml",
            )
            .map_err(|e| data(&e))?;
        let gen_config = WorldgenConfig::from_toml(&src, &registry).map_err(|e| data(&e))?;
        let physics_src = mods
            .read_slot(world::modset::MOVEMENT_SLOT, "movement/profile.toml")
            .map_err(|e| data(&e))?;
        // The build's dimensions (T4-02): checked at start; the overworld's
        // physics apply to players (other dimensions run with part 2).
        let dimensions = world::dimension::from_mods(mods).map_err(|e| data(&e))?;
        let gravity_scale = dimensions
            .get(world::dimension::OVERWORLD)
            .map_or(1.0, |d| d.physics.player_gravity_scale);
        let physics = PhysicsConfig::from_toml(&physics_src)
            .map_err(|e| data(&e))?
            .with_gravity_scale(gravity_scale);

        std::fs::create_dir_all(&config.world_dir)
            .map_err(|e| ServerError::World(e.to_string()))?;
        let mut notices = Vec::new();
        let seed = world_seed(
            &config,
            &generator_id(mods, &gen_config.engine),
            &mut notices,
        )?;
        let subsystems = config
            .subsystems
            .start(&capabilities::SubsystemContext {
                mods,
                world_dir: &config.world_dir,
            })
            .map_err(ServerError::Data)?;
        let active = subsystems.capabilities();
        let zone_control = subsystems.zone_control();
        let bounds = config.bounds;
        let store = WorldStore::open(&config.world_dir, &registry, bounds)
            .map_err(|e| ServerError::World(e.to_string()))?;
        let storage = Storage::start(store, Arc::clone(&registry))
            .map_err(|e| ServerError::World(e.to_string()))?;
        let mut world = World::new(
            WorldSetup {
                registry: Arc::clone(&registry),
                context: Arc::new(GenContext::new(seed, gen_config, bounds)),
                physics,
                dir: config.world_dir.clone(),
                view_distance: config.view_distance.max(2),
                sim_distance: config.sim_distance,
                threads: config.threads,
                region_threads: config.region_threads,
                entity_limits: crate::entities::EntityLimits::from_mods(&config.mods)
                    .map_err(ServerError::Data)?,
                subsystems,
            },
            storage,
        )
        .map_err(ServerError::World)?;
        let log = Arc::new(metrics::TickLog::default());
        world.set_log(Arc::clone(&log));
        if config.test_walkers > 0 {
            world.spawn_walkers(glam::DVec3::new(8.5, 0.0, 8.5), config.test_walkers, 30.0);
        }

        // Offline mode serves LAN and loopback only (legal.md rule 7), and its
        // clients accept any certificate, so a fresh self-signed one is enough.
        // Real certificates come with online play (stage 3).
        let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
            .map_err(|e| ServerError::Net(e.to_string()))?;
        let cert = certified.cert.der().clone();
        let key = PrivateKeyDer::Pkcs8(certified.signing_key.serialize_der().into());

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("net")
            .enable_all()
            .build()
            .map_err(|e| ServerError::Net(e.to_string()))?;
        let endpoint = {
            let _guard = runtime.enter();
            net::server_endpoint(config.listen, cert, key)
                .map_err(|e| ServerError::Net(e.to_string()))?
        };
        let addr = endpoint
            .local_addr()
            .map_err(|e| ServerError::Net(e.to_string()))?;

        let (world_tx, world_rx) = crossbeam_channel::unbounded();
        let ctx = Arc::new(ConnContext {
            world: world_tx,
            policy: config.policy,
            server_name: config.server_name.clone(),
            motd: config.motd.clone(),
            block_states: Arc::new(registry.id_table().names),
            capabilities: active.names(),
        });
        let accept = endpoint.clone();
        runtime.spawn(async move {
            while let Some(incoming) = accept.accept().await {
                tokio::spawn(conn::serve(incoming, Arc::clone(&ctx)));
            }
        });
        let stop = Arc::new(AtomicBool::new(false));
        if config.lan_announce && config.policy.allow_offline && lan_private(addr.ip()) {
            start_announcer(addr.port(), config.server_name.clone(), Arc::clone(&stop));
        }
        let world_stop = Arc::clone(&stop);
        let world = std::thread::Builder::new()
            .name("world".into())
            .spawn(move || world.run(world_rx, world_stop))
            .map_err(|e| ServerError::World(e.to_string()))?;
        Ok(Self {
            addr,
            log,
            stop,
            world: Some(world),
            endpoint,
            runtime: Some(runtime),
            notices,
            capabilities: active,
            zones: zone_control,
        })
    }

    /// Things worth telling the operator about the world at start (a slot
    /// provider changed since the last run).
    #[must_use]
    pub fn notices(&self) -> &[String] {
        &self.notices
    }

    /// The capabilities whose subsystems run (the build's, ADR-0018).
    #[must_use]
    pub fn capabilities(&self) -> world::Capabilities {
        self.capabilities
    }

    /// The zone API (T4-08), when the build runs the `zones` capability.
    #[must_use]
    pub fn zones(&self) -> Option<&zones::ZoneControl> {
        self.zones.as_ref()
    }

    /// Tick records of the world thread (metrics).
    #[must_use]
    pub fn tick_log(&self) -> Arc<metrics::TickLog> {
        Arc::clone(&self.log)
    }

    /// The address the server listens on (useful with port 0).
    #[must_use]
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Kicks everyone, saves players and edited columns, and stops.
    pub fn shutdown(mut self) {
        self.stop_now();
    }

    fn stop_now(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(w) = self.world.take() {
            let _ = w.join();
        }
        if let Some(rt) = self.runtime.take() {
            // Let the kicks reach the clients, then close.
            rt.block_on(async {
                tokio::time::sleep(Duration::from_millis(200)).await;
                self.endpoint.close(0u32.into(), b"server stopped");
            });
            rt.shutdown_timeout(Duration::from_secs(1));
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop_now();
    }
}

#[cfg(test)]
mod seed_tests {
    #[test]
    fn big_seeds_survive_toml() {
        let text = toml::to_string(&super::WorldMeta {
            seed: u64::MAX,
            generator: None,
            slots: Default::default(),
            capabilities: vec!["zones".into()],
        })
        .expect("encode");
        let back: super::WorldMeta = toml::from_str(&text).expect("decode");
        assert_eq!(back.seed, u64::MAX, "{text}");
    }
}
