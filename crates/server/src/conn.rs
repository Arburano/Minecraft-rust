//! One task per client connection: handshake and admission, then input
//! datagrams to the world thread and column streams to the client.
//!
//! Nothing a client sends can panic the server or make it allocate without
//! bound: every frame has a size limit checked before allocating
//! (`net::frame::read_frame`), every message is decoded by the fuzzed
//! `protocol` decoders, input datagrams are rate-limited, and the client may
//! not open streams of its own beyond the control stream
//! (`net::game_server_transport`).

use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use auth::{AdmissionPolicy, Identity};
use bytes::Bytes;
use net::frame::{frame_body, read_frame, write_frame};
use protocol::{
    ClaimedIdentity, ClientControl, HandshakeError, Hello, InputDatagram, Rejection, ServerControl,
    ServerHello, ServerInfo, Welcome,
};
use quinn::{Connection, Endpoint, Incoming, SendStream};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use tokio::sync::oneshot;
use tokio::task::JoinSet;

use crate::interest::SendQueue;
use crate::world_thread::{ToClient, WorldMsg};

/// Column streams in flight per client (networking-protocol.md).
pub const MAX_COLUMN_STREAMS: usize = 16;

/// The client must send `Hello` within this time.
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
/// The world must place the player within this time.
const JOIN_TIMEOUT: Duration = Duration::from_secs(30);
/// The client must be `Ready` within this time (it may download mods first).
const READY_TIMEOUT: Duration = Duration::from_secs(60);
/// Largest `Hello` frame.
const MAX_HELLO_BYTES: usize = 16 * 1024;
/// Largest client control frame after the handshake.
const MAX_CONTROL_BYTES: usize = 1024;

/// Input datagrams: sustained rate and burst (the client sends 20 per
/// second); more is dropped.
const INPUT_RATE: f64 = 60.0;
const INPUT_BURST: f64 = 30.0;
/// A client that keeps flooding after this many dropped datagrams is closed.
const MAX_DROPPED_INPUTS: u64 = 2_000;

/// QUIC close codes.
pub mod close {
    /// Normal end.
    pub const NORMAL: u32 = 0;
    /// The client broke the protocol (garbage, oversized or unexpected data).
    pub const PROTOCOL: u32 = 1;
    /// Another protocol version.
    pub const VERSION: u32 = 2;
    /// Refused at admission.
    pub const REJECTED: u32 = 3;
    /// Too slow during the handshake.
    pub const TIMEOUT: u32 = 4;
    /// The server is shutting down.
    pub const SHUTDOWN: u32 = 5;
    /// The client flooded the server.
    pub const FLOOD: u32 = 6;
}

/// What every connection task shares (read-only).
pub(crate) struct Shared {
    pub world: Sender<WorldMsg>,
    pub policy: AdmissionPolicy,
    pub info: ServerInfo,
    /// The framed `ServerControl::Registry`, built once.
    pub registry_frame: Bytes,
}

/// Accepts connections until the endpoint closes.
pub(crate) async fn accept_loop(endpoint: Endpoint, shared: Arc<Shared>) {
    while let Some(incoming) = endpoint.accept().await {
        let shared = Arc::clone(&shared);
        tokio::spawn(async move {
            let remote = incoming.remote_address();
            if let Err(e) = handle(incoming, &shared).await {
                eprintln!("server: connection from {remote}: {e}");
            }
        });
    }
}

/// Sends `Leave` when the connection task ends, however it ends.
struct LeaveGuard {
    world: Sender<WorldMsg>,
    client: u32,
}

impl Drop for LeaveGuard {
    fn drop(&mut self) {
        let _ = self.world.send(WorldMsg::Leave {
            client: self.client,
        });
    }
}

async fn send_hello(send: &mut SendStream, body: &[u8]) -> Result<(), String> {
    let frame = frame_body(body).map_err(|e| e.to_string())?;
    write_frame(send, &frame).await.map_err(|e| e.to_string())
}

/// Sends a final handshake answer and closes once the client has it.
async fn refuse(conn: &Connection, mut send: SendStream, body: &[u8], code: u32, why: &str) {
    let _ = send_hello(&mut send, body).await;
    let _ = send.finish();
    let _ = tokio::time::timeout(Duration::from_secs(2), send.stopped()).await;
    conn.close(code.into(), why.as_bytes());
}

fn admit(hello: &Hello, policy: AdmissionPolicy) -> Result<Identity, Rejection> {
    if hello.session_jwt.is_some() {
        // OPEN-QUESTION: 4
        return Err(Rejection::SessionNotSupported);
    }
    let ClaimedIdentity::Offline { name } = &hello.identity else {
        return Err(Rejection::Other("unsupported identity".into()));
    };
    let identity = Identity::offline(name).map_err(|_| Rejection::InvalidName)?;
    policy
        .admit(&identity)
        .map_err(|_| Rejection::OfflineNotAllowed)?;
    Ok(identity)
}

async fn handle(incoming: Incoming, shared: &Shared) -> Result<(), String> {
    let conn = incoming.await.map_err(|e| e.to_string())?;
    let timed_out = |conn: &Connection| {
        conn.close(close::TIMEOUT.into(), b"handshake timeout");
        "handshake timeout".to_string()
    };
    let (mut send, mut recv) = match tokio::time::timeout(HELLO_TIMEOUT, conn.accept_bi()).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => return Err(e.to_string()),
        Err(_) => return Err(timed_out(&conn)),
    };
    let body =
        match tokio::time::timeout(HELLO_TIMEOUT, read_frame(&mut recv, MAX_HELLO_BYTES)).await {
            Ok(Ok(b)) => b,
            Ok(Err(e)) => {
                conn.close(close::PROTOCOL.into(), b"bad handshake");
                return Err(e.to_string());
            }
            Err(_) => return Err(timed_out(&conn)),
        };
    let hello = match Hello::decode(&body) {
        Ok(h) => h,
        Err(e @ HandshakeError::VersionMismatch { .. }) => {
            refuse(
                &conn,
                send,
                &ServerHello::encode_version_mismatch(),
                close::VERSION,
                "protocol version mismatch",
            )
            .await;
            return Err(e.to_string());
        }
        Err(e) => {
            conn.close(close::PROTOCOL.into(), b"bad handshake");
            return Err(e.to_string());
        }
    };
    let identity = match admit(&hello, shared.policy) {
        Ok(i) => i,
        Err(r) => {
            let text = r.to_string();
            refuse(
                &conn,
                send,
                &ServerHello::Rejected(r).encode(),
                close::REJECTED,
                &text,
            )
            .await;
            return Err(format!("refused: {text}"));
        }
    };

    // Ask the world first: it rejects duplicates and full servers, and starts
    // loading the spawn area during the remaining handshake round trips.
    let (out_tx, out_rx) = unbounded_channel();
    let (admitted_tx, admitted_rx) = oneshot::channel();
    let (placed_tx, placed_rx) = oneshot::channel();
    let name = identity.name().as_str().to_owned();
    shared
        .world
        .send(WorldMsg::Join {
            player: identity.id().0,
            name: name.clone(),
            out: out_tx,
            admitted: admitted_tx,
            placed: placed_tx,
        })
        .map_err(|_| "the world thread is gone".to_string())?;
    let client = match tokio::time::timeout(HELLO_TIMEOUT, admitted_rx).await {
        Ok(Ok(Ok(c))) => c,
        Ok(Ok(Err(r))) => {
            let text = r.to_string();
            refuse(
                &conn,
                send,
                &ServerHello::Rejected(r).encode(),
                close::REJECTED,
                &text,
            )
            .await;
            return Err(format!("refused: {text}"));
        }
        _ => {
            let r = Rejection::Other("the server is too busy to admit the player".into());
            refuse(
                &conn,
                send,
                &ServerHello::Rejected(r).encode(),
                close::REJECTED,
                "join failed",
            )
            .await;
            return Err("join failed".into());
        }
    };
    let _leave = LeaveGuard {
        world: shared.world.clone(),
        client,
    };

    let welcome = ServerHello::Accepted(Welcome {
        server_info: shared.info.clone(),
        player_id: identity.id().0,
        // No mod manifest until stage 3: all zeros means "nothing to download".
        mod_manifest_hash: [0; 32],
    });
    send_hello(&mut send, &welcome.encode()).await?;
    write_frame(&mut send, &shared.registry_frame)
        .await
        .map_err(|e| e.to_string())?;
    let ready =
        match tokio::time::timeout(READY_TIMEOUT, read_frame(&mut recv, MAX_CONTROL_BYTES)).await {
            Ok(Ok(b)) => ClientControl::decode(&b).map_err(|e| e.to_string()),
            Ok(Err(e)) => Err(e.to_string()),
            Err(_) => return Err(timed_out(&conn)),
        };
    match ready {
        Ok(ClientControl::Ready) => {}
        Ok(_) => {
            conn.close(close::NORMAL.into(), b"left");
            return Ok(());
        }
        Err(e) => {
            conn.close(close::PROTOCOL.into(), b"bad control message");
            return Err(e);
        }
    }
    // A new player waits for the spawn column, which has been generating
    // since `Hello`.
    let Ok(Ok(spawn)) = tokio::time::timeout(JOIN_TIMEOUT, placed_rx).await else {
        let kick = ServerControl::Kick("the server could not place the player".into());
        if let Ok(f) = kick.encode_frame() {
            let _ = write_frame(&mut send, &f).await;
        }
        conn.close(close::NORMAL.into(), b"not placed");
        return Err("the player could not be placed".into());
    };
    let spawn = ServerControl::Spawn(spawn)
        .encode_frame()
        .map_err(|e| e.to_string())?;
    write_frame(&mut send, &spawn)
        .await
        .map_err(|e| e.to_string())?;
    println!("server: {name} is in the world");

    let mut tasks = JoinSet::new();
    tasks.spawn(stream_columns(
        conn.clone(),
        out_rx,
        shared.info.view_distance,
    ));
    {
        let conn = conn.clone();
        tasks.spawn(async move {
            // Control stream: `Disconnect`, or garbage (closes the connection).
            loop {
                match read_frame(&mut recv, MAX_CONTROL_BYTES).await {
                    Ok(b) => match ClientControl::decode(&b) {
                        Ok(ClientControl::Ready) => {}
                        Ok(_) => {
                            conn.close(close::NORMAL.into(), b"left");
                            return Ok(());
                        }
                        Err(e) => {
                            conn.close(close::PROTOCOL.into(), b"bad control message");
                            return Err(e.to_string());
                        }
                    },
                    Err(net::frame::FrameError::TooLarge { .. }) => {
                        conn.close(close::PROTOCOL.into(), b"oversized control frame");
                        return Err("oversized control frame".into());
                    }
                    Err(_) => return Ok(()), // connection gone
                }
            }
        });
    }
    {
        let (conn, world) = (conn.clone(), shared.world.clone());
        tasks.spawn(async move { read_inputs(conn, world, client).await });
    }
    // The first task to end (the client left, broke the protocol or the
    // connection died) ends the session.
    let first = tasks.join_next().await;
    tasks.abort_all();
    conn.close(close::NORMAL.into(), b"bye");
    match first {
        Some(Ok(r)) => r,
        Some(Err(e)) => Err(e.to_string()),
        None => Ok(()),
    }
}

/// Token bucket for input datagrams.
struct RateLimit {
    tokens: f64,
    last: Instant,
}

impl RateLimit {
    fn new() -> Self {
        Self {
            tokens: INPUT_BURST,
            last: Instant::now(),
        }
    }

    fn allow(&mut self, now: Instant) -> bool {
        let dt = now.duration_since(self.last).as_secs_f64();
        self.last = now;
        self.tokens = (self.tokens + dt * INPUT_RATE).min(INPUT_BURST);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

async fn read_inputs(conn: Connection, world: Sender<WorldMsg>, client: u32) -> Result<(), String> {
    let mut limit = RateLimit::new();
    let mut dropped = 0u64;
    while let Ok(datagram) = conn.read_datagram().await {
        if !limit.allow(Instant::now()) {
            dropped += 1;
            if dropped > MAX_DROPPED_INPUTS {
                conn.close(close::FLOOD.into(), b"input flood");
                return Err("input flood".into());
            }
            continue;
        }
        // A garbage datagram is dropped; datagrams may be corrupted by
        // nothing but a broken client, and dropping costs nothing.
        let Ok(input) = InputDatagram::decode(&datagram) else {
            continue;
        };
        if world
            .send(WorldMsg::Input {
                client,
                frames: input.frames,
            })
            .is_err()
        {
            return Ok(());
        }
    }
    Ok(())
}

/// Writes columns nearest first, at most [`MAX_COLUMN_STREAMS`] at a time;
/// each stream carries one framed `ColumnData` and is waited for until the
/// client has acknowledged it, so the limit bounds data in flight.
async fn stream_columns(
    conn: Connection,
    mut rx: UnboundedReceiver<ToClient>,
    view: u8,
) -> Result<(), String> {
    let mut queue = SendQueue::new(i32::from(view));
    let mut in_flight: JoinSet<Result<(), String>> = JoinSet::new();
    let apply = |q: &mut SendQueue, msg: ToClient| match msg {
        ToClient::Center(c) => q.set_center(c),
        ToClient::Column(pos, frame) => q.push(pos, frame),
    };
    loop {
        while in_flight.len() < MAX_COLUMN_STREAMS {
            let Some((_, frame, priority)) = queue.pop_nearest() else {
                break;
            };
            let mut stream = conn.open_uni().await.map_err(|e| e.to_string())?;
            // Open and prioritise here, not in the task: stream order then
            // follows distance exactly.
            stream.set_priority(priority).map_err(|e| e.to_string())?;
            in_flight.spawn(async move {
                stream.write_chunk(frame).await.map_err(|e| e.to_string())?;
                stream.finish().map_err(|e| e.to_string())?;
                stream.stopped().await.map_err(|e| e.to_string())?;
                Ok(())
            });
        }
        tokio::select! {
            msg = rx.recv() => {
                let Some(msg) = msg else { return Ok(()) };
                apply(&mut queue, msg);
                // Take everything queued so the nearest is chosen among all.
                while let Ok(msg) = rx.try_recv() {
                    apply(&mut queue, msg);
                }
            }
            Some(done) = in_flight.join_next(), if !in_flight.is_empty() => {
                match done {
                    Ok(Ok(())) => {}
                    // The connection is gone: the session ends.
                    Ok(Err(e)) => return Err(e),
                    Err(e) => return Err(e.to_string()),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limit_allows_steady_input_and_cuts_floods() {
        let mut r = RateLimit::new();
        let t0 = r.last;
        // A burst of 30 passes, then the bucket is empty.
        let burst = (0..100).filter(|_| r.allow(t0)).count();
        assert_eq!(burst, INPUT_BURST as usize);
        // 20 per second for 10 s all pass.
        let steady = (1..=200)
            .filter(|i| r.allow(t0 + Duration::from_millis(50 * i)))
            .count();
        assert_eq!(steady, 200);
    }

    fn hello(name: &str, jwt: Option<&str>) -> Hello {
        Hello {
            client_version: "test".into(),
            identity: ClaimedIdentity::Offline { name: name.into() },
            session_jwt: jwt.map(str::to_owned),
        }
    }

    #[test]
    fn admission() {
        let offline = AdmissionPolicy {
            allow_offline: true,
        };
        assert!(admit(&hello("Alex_1", None), offline).is_ok());
        assert_eq!(
            admit(&hello("Alex_1", None), AdmissionPolicy::default()).err(),
            Some(Rejection::OfflineNotAllowed)
        );
        assert_eq!(
            admit(&hello("no spaces", None), offline).err(),
            Some(Rejection::InvalidName)
        );
        assert_eq!(
            admit(&hello("Alex_1", Some("token")), offline).err(),
            Some(Rejection::SessionNotSupported)
        );
    }
}
