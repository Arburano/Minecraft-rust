//! The connection to a dedicated server (T2-02, `networking-protocol.md`).
//!
//! The handshake runs blocking (with a timeout) when the player joins; then a
//! small tokio runtime receives columns, unloads and kicks in the background
//! and hands them to the main thread through a channel. Columns are
//! decompressed, decoded and mapped to this client's block IDs on the network
//! threads, so the main thread only inserts finished columns.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use net::messages::{StreamError, read_header, read_message, write_header, write_message};
use protocol::{
    ChunkStream, ClientControl, ClientEvent, EntityDatagram, Hello, InputDatagram, Message,
    ServerControl, ServerEvent, Spawn, Welcome,
};
use quinn::Connection;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use world::coords::BlockPos;
use world::registry::IdTable;
use world::{BlockStateId, Column, ColumnPos, Registry};

/// Time allowed to connect and log in.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// Errors while joining a server.
#[derive(Debug, thiserror::Error)]
pub enum RemoteError {
    /// The server could not be reached or the connection broke.
    #[error("connection: {0}")]
    Connection(String),
    /// The server refused the player (full, bad name, login required…).
    #[error("{0}")]
    Refused(String),
    /// The server speaks another protocol version, or not this protocol.
    #[error(transparent)]
    Protocol(#[from] protocol::ProtocolError),
    /// The server did not answer in time.
    #[error("the server did not answer within {0:?}")]
    Timeout(Duration),
    /// The server runs a capability this client cannot (ADR-0018).
    #[error("{0}")]
    Capability(String),
}

/// Checks the capabilities a server runs against the ones this client was
/// compiled with ([`crate::capabilities`]).
pub fn check_server_capabilities(
    names: &[String],
    compiled: world::Capabilities,
) -> Result<(), RemoteError> {
    for name in names {
        match world::Capability::from_name(name) {
            None => {
                return Err(RemoteError::Capability(format!(
                    "the server runs capability {name:?}, which this version of the game does \
                     not know; update the game"
                )));
            }
            Some(c) if !compiled.contains(c) => {
                return Err(RemoteError::Capability(format!(
                    "the server runs capability {c}; this client was built without it \
                     (cargo feature {:?})",
                    c.cargo_feature().unwrap_or(c.name())
                )));
            }
            Some(_) => {}
        }
    }
    Ok(())
}

impl From<StreamError> for RemoteError {
    fn from(e: StreamError) -> Self {
        match e {
            StreamError::Protocol(p) => Self::Protocol(p),
            other => Self::Connection(other.to_string()),
        }
    }
}

/// Something that arrived from the server.
pub enum RemoteEvent {
    /// A column, already mapped to this client's block IDs.
    Column(Column),
    /// The server will not keep this column up to date any more.
    Forget(ColumnPos),
    /// A column outside the focus, still compressed ([`Remote::unpack`]).
    Packed(ColumnPos, protocol::ColumnPayload),
    /// This tick's server datagram (reconciliation and entity deltas).
    Datagram(EntityDatagram),
    /// Authoritative block changes, already in this client's block IDs.
    Blocks(Vec<(BlockPos, BlockStateId)>),
    /// The server's answer to one of our edits.
    EditAck {
        /// Our sequence number.
        seq: u32,
        /// Whether it was applied.
        accepted: bool,
    },
    /// The session ended; the text is for the player.
    Ended(String),
}

enum Command {
    Input(InputDatagram),
    Event(ClientEvent),
    Leave,
}

/// Only columns within `radius` of `center` are decoded; the rest are
/// received (and counted) but dropped. Headless bots use it so a bot does
/// not spend CPU and memory on columns it never looks at.
#[derive(Debug)]
pub struct Focus {
    center: std::sync::atomic::AtomicU64,
    radius: i32,
}

impl Focus {
    /// A focus of `radius` columns around the origin.
    #[must_use]
    pub fn new(radius: i32) -> Self {
        Self {
            center: std::sync::atomic::AtomicU64::new(0),
            radius,
        }
    }

    /// Moves the center.
    pub fn set(&self, c: ColumnPos) {
        let packed = (u64::from(c.x as u32) << 32) | u64::from(c.z as u32);
        self.center
            .store(packed, std::sync::atomic::Ordering::Relaxed);
    }

    /// Whether `c` is decoded.
    #[must_use]
    pub fn wants(&self, c: ColumnPos) -> bool {
        let v = self.center.load(std::sync::atomic::Ordering::Relaxed);
        let (x, z) = ((v >> 32) as u32 as i32, v as u32 as i32);
        (c.x - x).abs() <= self.radius && (c.z - z).abs() <= self.radius
    }
}

/// How to connect.
#[derive(Clone, Default)]
pub struct ConnectOptions {
    /// Run on this runtime instead of a private one (many bots in one process).
    pub runtime: Option<tokio::runtime::Handle>,
    /// Decode only the columns this focus wants.
    pub focus: Option<Arc<Focus>>,
}

/// A joined server.
pub struct Remote {
    runtime: Option<tokio::runtime::Runtime>,
    handle: tokio::runtime::Handle,
    events: Receiver<RemoteEvent>,
    commands: UnboundedSender<Command>,
    conn: Connection,
    /// This client's state ID → the server's (`None`: the server lacks it).
    to_server: Vec<Option<u16>>,
    registry: Arc<Registry>,
    remap: Arc<Option<Vec<BlockStateId>>>,
    /// The server's greeting.
    pub welcome: Welcome,
    /// Where the player starts.
    pub spawn: Spawn,
}

impl Remote {
    /// Connects, logs in as `name` (offline mode) and waits for the spawn.
    pub fn connect(
        addr: SocketAddr,
        name: &str,
        registry: Arc<Registry>,
    ) -> Result<Self, RemoteError> {
        Self::connect_with(addr, name, registry, ConnectOptions::default())
    }

    /// [`Self::connect`] with options.
    pub fn connect_with(
        addr: SocketAddr,
        name: &str,
        registry: Arc<Registry>,
        options: ConnectOptions,
    ) -> Result<Self, RemoteError> {
        let runtime = match &options.runtime {
            Some(_) => None,
            None => Some(
                tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .thread_name("client-net")
                    .enable_all()
                    .build()
                    .map_err(|e| RemoteError::Connection(e.to_string()))?,
            ),
        };
        let handle = match (&options.runtime, &runtime) {
            (Some(h), _) => h.clone(),
            (None, Some(rt)) => rt.handle().clone(),
            (None, None) => return Err(RemoteError::Connection("no runtime".into())),
        };
        let login = handle.block_on(async {
            tokio::time::timeout(CONNECT_TIMEOUT, login(addr, name, &registry))
                .await
                .map_err(|_| RemoteError::Timeout(CONNECT_TIMEOUT))?
        })?;
        let Login {
            conn,
            ctrl_send,
            ctrl_recv,
            welcome,
            spawn,
            remap,
        } = login;
        let (events_tx, events) = channel();
        let (commands, commands_rx) = unbounded_channel();
        let table: Vec<BlockStateId> = remap.clone().unwrap_or_else(|| {
            (0..registry.len())
                .map(|i| BlockStateId(i as u16))
                .collect()
        });
        let mut to_server = vec![None; registry.len()];
        for (server, client) in table.iter().enumerate() {
            if let Some(slot) = to_server.get_mut(usize::from(client.0))
                && slot.is_none()
                && *client != registry.missing()
            {
                *slot = u16::try_from(server).ok();
            }
        }
        let from_server = Arc::new(table);
        let remap = Arc::new(remap);
        let runtime_ref = &handle;
        runtime_ref.spawn(receive_columns(
            conn.clone(),
            events_tx.clone(),
            Arc::clone(&registry),
            Arc::clone(&remap),
            spawn.bounds(),
            options.focus.clone(),
        ));
        runtime_ref.spawn(receive_events(
            conn.clone(),
            events_tx.clone(),
            from_server,
            registry.missing(),
        ));
        runtime_ref.spawn(receive_datagrams(conn.clone(), events_tx.clone()));
        runtime_ref.spawn(receive_control(ctrl_recv, events_tx.clone()));
        runtime_ref.spawn(send_commands(
            conn.clone(),
            ctrl_send,
            commands_rx,
            events_tx,
        ));
        Ok(Self {
            runtime,
            handle,
            events,
            commands,
            conn,
            to_server,
            registry: Arc::clone(&registry),
            remap,
            welcome,
            spawn,
        })
    }

    /// Everything that arrived since the last call.
    pub fn poll(&self) -> impl Iterator<Item = RemoteEvent> + '_ {
        self.events.try_iter()
    }

    /// UDP bytes received and sent on this connection so far (QUIC, UDP
    /// payload: everything except the UDP/IP headers).
    #[must_use]
    pub fn bytes(&self) -> (u64, u64) {
        let s = self.conn.stats();
        (s.udp_rx.bytes, s.udp_tx.bytes)
    }

    /// Decodes a column kept compressed ([`RemoteEvent::Packed`]).
    #[must_use]
    pub fn unpack(&self, payload: &protocol::ColumnPayload) -> Option<Column> {
        decode_payload(payload, &self.registry, &self.remap, self.spawn.bounds())
    }

    /// Sends a reliable event (an edit, chat).
    pub fn send_event(&self, e: ClientEvent) {
        let _ = self.commands.send(Command::Event(e));
    }

    /// The server's number of one of this client's block states.
    #[must_use]
    pub fn server_state(&self, id: BlockStateId) -> Option<u16> {
        self.to_server.get(usize::from(id.0)).copied().flatten()
    }

    /// Sends an input datagram (dropped silently if the connection is gone:
    /// [`RemoteEvent::Ended`] reports that).
    pub fn send_input(&self, d: InputDatagram) {
        let _ = self.commands.send(Command::Input(d));
    }

    /// Leaves the server politely and stops the network threads (a shared
    /// runtime keeps running).
    pub fn leave(mut self) {
        let _ = self.commands.send(Command::Leave);
        match self.runtime.take() {
            Some(rt) => {
                rt.block_on(async { tokio::time::sleep(Duration::from_millis(100)).await });
                rt.shutdown_timeout(Duration::from_millis(500));
            }
            None => {
                let conn = self.conn.clone();
                self.handle.spawn(async move {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    conn.close(0u32.into(), b"left");
                });
            }
        }
    }
}

impl Drop for Remote {
    fn drop(&mut self) {
        if let Some(rt) = self.runtime.take() {
            rt.shutdown_background();
        } else {
            self.conn.close(0u32.into(), b"dropped");
        }
    }
}

struct Login {
    conn: Connection,
    ctrl_send: quinn::SendStream,
    ctrl_recv: quinn::RecvStream,
    welcome: Welcome,
    spawn: Spawn,
    /// Server state number → this client's state; `None` if identical.
    remap: Option<Vec<BlockStateId>>,
}

async fn login(addr: SocketAddr, name: &str, registry: &Registry) -> Result<Login, RemoteError> {
    let conn_err = |e: &dyn std::fmt::Display| RemoteError::Connection(e.to_string());
    let bind = match addr.ip() {
        IpAddr::V4(_) => SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
        IpAddr::V6(_) => SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0),
    };
    let endpoint = net::client_endpoint_offline(bind).map_err(|e| conn_err(&e))?;
    let conn = endpoint
        .connect(addr, "localhost")
        .map_err(|e| conn_err(&e))?
        .await
        .map_err(|e| conn_err(&e))?;
    let (mut send, mut recv) = conn.open_bi().await.map_err(|e| conn_err(&e))?;
    write_header(&mut send).await?;
    write_message(
        &mut send,
        &ClientControl::Hello(Hello {
            client_version: env!("CARGO_PKG_VERSION").into(),
            player_name: name.into(),
            session_jwt: None,
        }),
    )
    .await?;
    read_header(&mut recv).await?.check()?;

    let mut next = async || -> Result<ServerControl, RemoteError> {
        match read_message::<ServerControl>(&mut recv).await? {
            Some(ServerControl::Kick { message, .. }) => Err(RemoteError::Refused(message)),
            Some(m) => Ok(m),
            None => Err(RemoteError::Connection(
                "the server closed the connection".into(),
            )),
        }
    };
    let ServerControl::Welcome(welcome) = next().await? else {
        return Err(RemoteError::Connection("expected Welcome".into()));
    };
    check_server_capabilities(&welcome.capabilities, crate::capabilities())?;
    let ServerControl::Registry(sync) = next().await? else {
        return Err(RemoteError::Connection("expected the block table".into()));
    };
    let ServerControl::Spawn(spawn) = next().await? else {
        return Err(RemoteError::Connection("expected Spawn".into()));
    };
    let table = registry.remap(&IdTable {
        names: sync.block_states,
    });
    let identity = table.len() == registry.len()
        && table.iter().enumerate().all(|(i, v)| usize::from(v.0) == i);
    Ok(Login {
        conn,
        ctrl_send: send,
        ctrl_recv: recv,
        welcome,
        spawn,
        remap: (!identity).then_some(table),
    })
}

/// A column from its payload, in this client's block IDs.
fn decode_payload(
    payload: &protocol::ColumnPayload,
    registry: &Registry,
    remap: &Option<Vec<BlockStateId>>,
    bounds: world::WorldBounds,
) -> Option<Column> {
    let mut record = payload.decode().ok()?;
    if record.bounds != bounds {
        return None;
    }
    if let Some(table) = remap.as_ref() {
        for s in &mut record.sections {
            s.remap(table, registry.missing());
        }
    }
    Column::from_sections(record.pos, bounds, record.sections, registry)
}

async fn receive_columns(
    conn: Connection,
    events: Sender<RemoteEvent>,
    registry: Arc<Registry>,
    remap: Arc<Option<Vec<BlockStateId>>>,
    bounds: world::WorldBounds,
    focus: Option<Arc<Focus>>,
) {
    while let Ok(mut stream) = conn.accept_uni().await {
        let focus = focus.clone();
        let (events, registry, remap) = (events.clone(), Arc::clone(&registry), Arc::clone(&remap));
        tokio::spawn(async move {
            let Ok(bytes) = stream.read_to_end(ChunkStream::MAX_LEN).await else {
                return;
            };
            // Decompression and decoding are CPU work: off the async workers.
            let event = tokio::task::spawn_blocking(move || {
                let ChunkStream::Column(payload) = ChunkStream::decode(&bytes).ok()?;
                if focus.is_some_and(|f| !f.wants(payload.pos.into())) {
                    // Not needed now: kept compressed, decoded when needed.
                    return Some(RemoteEvent::Packed(payload.pos.into(), payload));
                }
                decode_payload(&payload, &registry, &remap, bounds).map(RemoteEvent::Column)
            })
            .await
            .ok()
            .flatten();
            if let Some(e) = event {
                let _ = events.send(e);
            }
        });
    }
}

async fn receive_events(
    conn: Connection,
    events: Sender<RemoteEvent>,
    table: Arc<Vec<BlockStateId>>,
    missing: BlockStateId,
) {
    while let Ok((_, mut recv)) = conn.accept_bi().await {
        let (events, table) = (events.clone(), Arc::clone(&table));
        tokio::spawn(async move {
            while let Ok(Some(e)) = read_message::<ServerEvent>(&mut recv).await {
                let out = match e {
                    ServerEvent::ForgetColumn(c) => RemoteEvent::Forget(c.into()),
                    ServerEvent::BlockChanges(changes) => RemoteEvent::Blocks(
                        changes
                            .iter()
                            .map(|c| {
                                let id =
                                    table.get(usize::from(c.state)).copied().unwrap_or(missing);
                                (c.pos.into(), id)
                            })
                            .collect(),
                    ),
                    ServerEvent::BlockAck { seq, accepted } => {
                        RemoteEvent::EditAck { seq, accepted }
                    }
                    ServerEvent::Chat { .. } => continue,
                };
                if events.send(out).is_err() {
                    return;
                }
            }
        });
    }
}

async fn receive_datagrams(conn: Connection, events: Sender<RemoteEvent>) {
    while let Ok(bytes) = conn.read_datagram().await {
        // A malformed datagram is dropped; the next tick brings a new one.
        if let Ok(d) = EntityDatagram::decode(&bytes)
            && events.send(RemoteEvent::Datagram(d)).is_err()
        {
            return;
        }
    }
}

async fn receive_control(mut recv: quinn::RecvStream, events: Sender<RemoteEvent>) {
    loop {
        match read_message::<ServerControl>(&mut recv).await {
            Ok(Some(ServerControl::Kick { message, .. })) => {
                let _ = events.send(RemoteEvent::Ended(message));
                return;
            }
            Ok(Some(_)) => {}
            Ok(None) => return,
            Err(e) => {
                let _ = events.send(RemoteEvent::Ended(format!("connection lost: {e}")));
                return;
            }
        }
    }
}

async fn send_commands(
    conn: Connection,
    mut ctrl: quinn::SendStream,
    mut commands: tokio::sync::mpsc::UnboundedReceiver<Command>,
    events: Sender<RemoteEvent>,
) {
    let mut stream: Option<quinn::SendStream> = None;
    loop {
        tokio::select! {
            c = commands.recv() => match c {
                Some(Command::Input(d)) => {
                    if let Ok(bytes) = d.encode() {
                        let _ = conn.send_datagram(bytes.into());
                    }
                }
                Some(Command::Event(e)) => {
                    if stream.is_none() {
                        stream = conn.open_bi().await.ok().map(|(s, _)| s);
                    }
                    let ok = match stream.as_mut() {
                        Some(s) => write_message(s, &e).await.is_ok(),
                        None => false,
                    };
                    if !ok {
                        let _ = events.send(RemoteEvent::Ended("connection lost".into()));
                        return;
                    }
                }
                Some(Command::Leave) | None => {
                    let _ = write_message(&mut ctrl, &ClientControl::Leave).await;
                    let _ = ctrl.finish();
                    conn.close(0u32.into(), b"left");
                    return;
                }
            },
            reason = conn.closed() => {
                let _ = events.send(RemoteEvent::Ended(format!("disconnected: {reason}")));
                return;
            }
        }
    }
}
