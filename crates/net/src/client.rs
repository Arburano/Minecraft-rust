//! The game client's connection (T2-02): handshake, then column streams and
//! control messages delivered to the (synchronous) game loop.
//!
//! The connection runs on its own small tokio runtime. The game loop polls
//! [`NetClient::try_event`] once per frame and sends input datagrams with
//! [`NetClient::send_input`]; neither blocks. Columns are decoded and
//! unpacked on the network threads by a caller-supplied function, so the main
//! thread only inserts ready columns.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use bytes::Bytes;
use protocol::{
    ClientControl, ColumnData, HandshakeError, Hello, InputDatagram, Registry, ServerControl,
    ServerHello, Spawn, Welcome,
};
use quinn::{Connection, Endpoint, RecvStream, SendStream};

use crate::frame::{FrameError, frame_body, read_frame, write_frame};

/// Largest server handshake frame (`ServerHello`).
const MAX_HELLO_BYTES: usize = 16 * 1024;

/// How the client checks the server's certificate.
#[derive(Clone, Debug)]
pub enum Trust {
    /// Trust exactly this certificate (tests, pinned servers).
    Certificate(quinn::rustls::pki_types::CertificateDer<'static>),
    /// Offline mode: accept the server's self-signed certificate
    /// ([`crate::endpoint::client_endpoint_offline`]).
    OfflineAnyCertificate,
}

/// Where and how to join.
#[derive(Clone, Debug)]
pub struct JoinParams {
    /// Server address.
    pub addr: SocketAddr,
    /// TLS server name (any name works for an offline server).
    pub server_name: String,
    /// The first message.
    pub hello: Hello,
    /// Certificate check.
    pub trust: Trust,
    /// Limit for the connection and the handshake.
    pub timeout: Duration,
}

/// What the server told the client during the handshake.
#[derive(Clone, Debug)]
pub struct Joined {
    /// Server description and the player's ID.
    pub welcome: Welcome,
    /// ID tables.
    pub registry: Registry,
}

/// Joining failed.
#[derive(Debug, thiserror::Error)]
pub enum JoinError {
    /// The local endpoint could not be created.
    #[error("network: {0}")]
    Endpoint(#[from] crate::endpoint::Error),
    /// The tokio runtime could not start.
    #[error("network runtime: {0}")]
    Runtime(std::io::Error),
    /// No connection.
    #[error("cannot connect: {0}")]
    Connect(String),
    /// Version mismatch, rejection or a bad handshake.
    #[error(transparent)]
    Handshake(#[from] HandshakeError),
    /// The server broke the protocol.
    #[error("protocol error: {0}")]
    Protocol(String),
    /// The server did not answer in time.
    #[error("the server did not answer in time")]
    Timeout,
}

impl From<FrameError> for JoinError {
    fn from(e: FrameError) -> Self {
        Self::Protocol(e.to_string())
    }
}

/// Something that happened on the connection.
#[derive(Debug)]
pub enum NetEvent<T> {
    /// The player is placed.
    Spawn(Spawn),
    /// A column arrived and was unpacked.
    Column(T),
    /// A column record could not be unpacked (the server is broken).
    BadColumn(String),
    /// The server dropped the client.
    Kicked(String),
    /// The connection ended.
    Closed(String),
}

/// A connection after the handshake, before columns flow.
pub struct PendingClient {
    runtime: tokio::runtime::Runtime,
    endpoint: Endpoint,
    conn: Connection,
    send: SendStream,
    recv: RecvStream,
}

impl PendingClient {
    /// Connects and runs the handshake up to `Registry`. Blocks at most
    /// `params.timeout`.
    pub fn connect(params: &JoinParams) -> Result<(Self, Joined), JoinError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("net-client")
            .enable_all()
            .build()
            .map_err(JoinError::Runtime)?;
        let result = runtime.block_on(async {
            let bind = if params.addr.is_ipv6() {
                SocketAddr::from(([0u16; 8], 0))
            } else {
                SocketAddr::from(([0, 0, 0, 0], 0))
            };
            let endpoint = match &params.trust {
                Trust::Certificate(c) => {
                    let e = crate::endpoint::client_endpoint(c.clone())?;
                    e.rebind(std::net::UdpSocket::bind(bind).map_err(crate::Error::Io)?)
                        .map_err(crate::Error::Io)?;
                    e
                }
                Trust::OfflineAnyCertificate => crate::endpoint::client_endpoint_offline(bind)?,
            };
            let handshake = async {
                let conn = endpoint
                    .connect(params.addr, &params.server_name)
                    .map_err(|e| JoinError::Connect(e.to_string()))?
                    .await
                    .map_err(|e| JoinError::Connect(e.to_string()))?;
                let (mut send, mut recv) = conn
                    .open_bi()
                    .await
                    .map_err(|e| JoinError::Connect(e.to_string()))?;
                let hello = frame_body(&params.hello.encode())
                    .map_err(|e| JoinError::Protocol(e.to_string()))?;
                write_frame(&mut send, &hello).await?;
                let answer = read_frame(&mut recv, MAX_HELLO_BYTES).await?;
                let welcome = ServerHello::decode(&answer)?;
                let body = read_frame(&mut recv, protocol::MAX_FRAME_BYTES).await?;
                let registry = match ServerControl::decode(&body)
                    .map_err(|e| JoinError::Protocol(e.to_string()))?
                {
                    ServerControl::Registry(r) => r,
                    ServerControl::Kick(text) => {
                        return Err(JoinError::Protocol(format!("kicked: {text}")));
                    }
                    other => {
                        return Err(JoinError::Protocol(format!(
                            "expected the ID tables, got {other:?}"
                        )));
                    }
                };
                Ok((conn, send, recv, Joined { welcome, registry }))
            };
            match tokio::time::timeout(params.timeout, handshake).await {
                Ok(Ok((conn, send, recv, joined))) => Ok((endpoint, conn, send, recv, joined)),
                Ok(Err(e)) => Err(e),
                Err(_) => Err(JoinError::Timeout),
            }
        });
        let (endpoint, conn, send, recv, joined) = result?;
        Ok((
            Self {
                runtime,
                endpoint,
                conn,
                send,
                recv,
            },
            joined,
        ))
    }

    /// Tells the server the client is ready and starts receiving. `unpack`
    /// turns a column record into what the game needs (remapped blocks); it
    /// runs on the network threads and returns `Err` for a corrupt record.
    pub fn start<T, F>(self, unpack: F) -> NetClient<T>
    where
        T: Send + 'static,
        F: Fn(ColumnData) -> Result<T, String> + Send + Sync + 'static,
    {
        let Self {
            runtime,
            endpoint,
            conn,
            mut send,
            mut recv,
        } = self;
        let (tx, rx) = mpsc::channel();
        let unpack = Arc::new(unpack);
        {
            let (conn, tx) = (conn.clone(), tx.clone());
            runtime.spawn(async move {
                let ready = match ClientControl::Ready.encode_frame() {
                    Ok(f) => f,
                    Err(e) => {
                        let _ = tx.send(NetEvent::Closed(e.to_string()));
                        return;
                    }
                };
                if let Err(e) = write_frame(&mut send, &ready).await {
                    let _ = tx.send(NetEvent::Closed(e.to_string()));
                    return;
                }
                // Keep `send` open: finishing it would tell the server we left.
                loop {
                    let body = match read_frame(&mut recv, protocol::MAX_FRAME_BYTES).await {
                        Ok(b) => b,
                        Err(_) => break, // the connection watcher reports why
                    };
                    match ServerControl::decode(&body) {
                        Ok(ServerControl::Spawn(s)) => {
                            let _ = tx.send(NetEvent::Spawn(s));
                        }
                        Ok(ServerControl::Kick(text)) => {
                            let _ = tx.send(NetEvent::Kicked(text));
                            conn.close(0u32.into(), b"kicked");
                            break;
                        }
                        Ok(_) => {}
                        Err(e) => {
                            let _ = tx.send(NetEvent::Closed(format!("bad control message: {e}")));
                            conn.close(1u32.into(), b"bad control message");
                            break;
                        }
                    }
                }
                drop(send);
            });
        }
        {
            let (conn, tx) = (conn.clone(), tx.clone());
            runtime.spawn(async move {
                while let Ok(mut stream) = conn.accept_uni().await {
                    let (tx, unpack) = (tx.clone(), Arc::clone(&unpack));
                    tokio::spawn(async move {
                        let event = match read_frame(&mut stream, protocol::MAX_FRAME_BYTES).await {
                            Ok(body) => match ColumnData::decode(&body) {
                                Ok(column) => match unpack(column) {
                                    Ok(t) => NetEvent::Column(t),
                                    Err(e) => NetEvent::BadColumn(e),
                                },
                                Err(e) => NetEvent::BadColumn(e.to_string()),
                            },
                            // A stream cut short by the connection ending.
                            Err(_) => return,
                        };
                        let _ = tx.send(event);
                    });
                }
            });
        }
        {
            let (conn, tx) = (conn.clone(), tx);
            runtime.spawn(async move {
                let reason = conn.closed().await;
                let _ = tx.send(NetEvent::Closed(reason.to_string()));
            });
        }
        NetClient {
            events: rx,
            conn,
            endpoint,
            runtime: Some(runtime),
        }
    }
}

/// A joined connection. Dropping it closes the connection.
pub struct NetClient<T> {
    events: mpsc::Receiver<NetEvent<T>>,
    conn: Connection,
    endpoint: Endpoint,
    runtime: Option<tokio::runtime::Runtime>,
}

impl<T> NetClient<T> {
    /// The next event, if any. Never blocks.
    pub fn try_event(&self) -> Option<NetEvent<T>> {
        self.events.try_recv().ok()
    }

    /// Sends an input datagram. Returns `false` if it could not be sent
    /// (too large, or the connection is gone); input is resent in the next
    /// datagrams anyway.
    pub fn send_input(&self, input: &InputDatagram) -> bool {
        match input.encode() {
            Ok(bytes) => self.conn.send_datagram(Bytes::from(bytes)).is_ok(),
            Err(_) => false,
        }
    }

    /// Current round-trip estimate.
    #[must_use]
    pub fn rtt(&self) -> Duration {
        self.conn.rtt()
    }

    /// Leaves the server.
    pub fn disconnect(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.conn.close(0u32.into(), b"client left");
        if let Some(rt) = self.runtime.take() {
            let endpoint = self.endpoint.clone();
            // Give the close frame a moment to leave, then stop the threads
            // (`block_on` is not allowed inside another runtime).
            if tokio::runtime::Handle::try_current().is_err() {
                rt.block_on(async {
                    let _ = tokio::time::timeout(Duration::from_millis(200), endpoint.wait_idle())
                        .await;
                });
            }
            rt.shutdown_background();
        }
    }
}

impl<T> Drop for NetClient<T> {
    fn drop(&mut self) {
        self.shutdown();
    }
}
