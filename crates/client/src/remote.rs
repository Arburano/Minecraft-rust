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
    ChunkStream, ClientControl, Hello, InputDatagram, Message, ServerControl, ServerEvent, Spawn,
    Welcome,
};
use quinn::Connection;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
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
    /// The session ended; the text is for the player.
    Ended(String),
}

enum Command {
    Input(InputDatagram),
    Leave,
}

/// A joined server.
pub struct Remote {
    runtime: Option<tokio::runtime::Runtime>,
    events: Receiver<RemoteEvent>,
    commands: UnboundedSender<Command>,
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
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("client-net")
            .enable_all()
            .build()
            .map_err(|e| RemoteError::Connection(e.to_string()))?;
        let login = runtime.block_on(async {
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
        runtime.spawn(receive_columns(
            conn.clone(),
            events_tx.clone(),
            Arc::clone(&registry),
            Arc::new(remap),
            spawn.bounds(),
        ));
        runtime.spawn(receive_events(conn.clone(), events_tx.clone()));
        runtime.spawn(receive_control(ctrl_recv, events_tx.clone()));
        runtime.spawn(send_commands(conn, ctrl_send, commands_rx, events_tx));
        Ok(Self {
            runtime: Some(runtime),
            events,
            commands,
            welcome,
            spawn,
        })
    }

    /// Everything that arrived since the last call.
    pub fn poll(&self) -> impl Iterator<Item = RemoteEvent> + '_ {
        self.events.try_iter()
    }

    /// Sends an input datagram (dropped silently if the connection is gone:
    /// [`RemoteEvent::Ended`] reports that).
    pub fn send_input(&self, d: InputDatagram) {
        let _ = self.commands.send(Command::Input(d));
    }

    /// Leaves the server politely and stops the network threads.
    pub fn leave(mut self) {
        let _ = self.commands.send(Command::Leave);
        if let Some(rt) = self.runtime.take() {
            rt.block_on(async { tokio::time::sleep(Duration::from_millis(100)).await });
            rt.shutdown_timeout(Duration::from_millis(500));
        }
    }
}

impl Drop for Remote {
    fn drop(&mut self) {
        if let Some(rt) = self.runtime.take() {
            rt.shutdown_background();
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

async fn receive_columns(
    conn: Connection,
    events: Sender<RemoteEvent>,
    registry: Arc<Registry>,
    remap: Arc<Option<Vec<BlockStateId>>>,
    bounds: world::WorldBounds,
) {
    while let Ok(mut stream) = conn.accept_uni().await {
        let (events, registry, remap) = (events.clone(), Arc::clone(&registry), Arc::clone(&remap));
        tokio::spawn(async move {
            let Ok(bytes) = stream.read_to_end(ChunkStream::MAX_LEN).await else {
                return;
            };
            // Decompression and decoding are CPU work: off the async workers.
            let column = tokio::task::spawn_blocking(move || {
                let ChunkStream::Column(payload) = ChunkStream::decode(&bytes).ok()?;
                let mut record = payload.decode().ok()?;
                if record.bounds != bounds {
                    return None;
                }
                if let Some(table) = remap.as_ref() {
                    for s in &mut record.sections {
                        s.remap(table, registry.missing());
                    }
                }
                Column::from_sections(record.pos, bounds, record.sections, &registry)
            })
            .await
            .ok()
            .flatten();
            if let Some(c) = column {
                let _ = events.send(RemoteEvent::Column(c));
            }
        });
    }
}

async fn receive_events(conn: Connection, events: Sender<RemoteEvent>) {
    while let Ok((_, mut recv)) = conn.accept_bi().await {
        let events = events.clone();
        tokio::spawn(async move {
            while let Ok(Some(e)) = read_message::<ServerEvent>(&mut recv).await {
                if let ServerEvent::ForgetColumn(c) = e {
                    let _ = events.send(RemoteEvent::Forget(c.into()));
                }
            }
        });
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
    loop {
        tokio::select! {
            c = commands.recv() => match c {
                Some(Command::Input(d)) => {
                    if let Ok(bytes) = d.encode() {
                        let _ = conn.send_datagram(bytes.into());
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
