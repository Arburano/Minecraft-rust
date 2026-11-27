//! One client connection (`networking-protocol.md`, "Handshake").
//!
//! The handshake runs on the control stream; then the connection forwards
//! input datagrams to the world thread, opens one prioritized stream per
//! column the world hands it, and sends column unloads on the events stream.
//! Everything the client sends is size-limited, validated and rate-limited;
//! a client that keeps sending garbage is disconnected, never the server.

use std::sync::Arc;
use std::time::{Duration, Instant};

use auth::{AdmissionPolicy, Identity};
use crossbeam_channel::Sender;
use net::messages::{read_header, read_message, write_header, write_message};
use protocol::{
    ClientControl, InputDatagram, KickReason, Message, RegistrySync, ServerControl, ServerEvent,
    Welcome,
};
use quinn::{Connection, SendStream, VarInt};
use tokio::sync::mpsc;

use crate::game::{TICK_RATE, ToConn, ToWorld};

/// Time allowed for the whole handshake.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Input datagrams per second a client may send (it needs 20); excess is dropped.
const DATAGRAMS_PER_SECOND: f64 = 60.0;
/// Burst allowance for the datagram rate.
const DATAGRAM_BURST: f64 = 120.0;
/// Malformed datagrams tolerated before the client is disconnected.
const MAX_BAD_DATAGRAMS: u32 = 50;

/// QUIC close codes.
mod code {
    pub const NORMAL: u32 = 0;
    pub const VERSION: u32 = 1;
    pub const REFUSED: u32 = 2;
    pub const PROTOCOL: u32 = 3;
}

/// Shared by all connections.
pub struct ConnContext {
    /// The world thread's inbox.
    pub world: Sender<ToWorld>,
    /// Who may join.
    pub policy: AdmissionPolicy,
    /// Server display name.
    pub server_name: String,
    /// Message of the day.
    pub motd: String,
    /// The block ID table sent to every client.
    pub block_states: Arc<Vec<String>>,
}

fn close(conn: &Connection, code: u32, reason: &str) {
    conn.close(VarInt::from_u32(code), reason.as_bytes());
}

/// Serves one connection to its end. Never panics on client input.
pub async fn serve(incoming: quinn::Incoming, ctx: Arc<ConnContext>) {
    let Ok(Ok(conn)) = tokio::time::timeout(HANDSHAKE_TIMEOUT, incoming).await else {
        return;
    };
    let joined = tokio::time::timeout(HANDSHAKE_TIMEOUT, handshake(&conn, &ctx)).await;
    let (id, name, ctrl) = match joined {
        Ok(Ok(j)) => j,
        Ok(Err(reason)) => {
            close(&conn, reason.0, &reason.1);
            return;
        }
        Err(_) => {
            close(&conn, code::PROTOCOL, "handshake timeout");
            return;
        }
    };
    play(conn, ctx, id, name, ctrl).await;
}

type Refusal = (u32, String);

async fn handshake(
    conn: &Connection,
    ctx: &ConnContext,
) -> Result<(String, String, (SendStream, quinn::RecvStream)), Refusal> {
    let proto = |e: &dyn std::fmt::Display| (code::PROTOCOL, e.to_string());
    let (mut send, mut recv) = conn.accept_bi().await.map_err(|e| proto(&e))?;
    let theirs = read_header(&mut recv).await.map_err(|e| proto(&e))?;
    write_header(&mut send).await.map_err(|e| proto(&e))?;
    if let Err(e) = theirs.check() {
        // The client reads our header and reports the mismatch itself.
        flush(send).await;
        return Err((code::VERSION, e.to_string()));
    }
    let hello = match read_message::<ClientControl>(&mut recv).await {
        Ok(Some(ClientControl::Hello(h))) => h,
        Ok(_) => return Err((code::PROTOCOL, "expected Hello".into())),
        Err(e) => return Err(proto(&e)),
    };
    let refusal = Identity::offline(&hello.player_name)
        .map_err(|e| (KickReason::BadName, e.to_string()))
        .and_then(|i| {
            ctx.policy
                .admit(&i)
                .map(|()| i)
                .map_err(|e| (KickReason::LoginRequired, e.to_string()))
        });
    let identity = match refusal {
        Ok(i) => i,
        Err((reason, message)) => {
            let _ = write_message(
                &mut send,
                &ServerControl::Kick {
                    reason,
                    message: message.clone(),
                },
            )
            .await;
            flush(send).await;
            return Err((code::REFUSED, message));
        }
    };
    let welcome = ServerControl::Welcome(Welcome {
        server_name: ctx.server_name.clone(),
        motd: ctx.motd.clone(),
        server_version: env!("CARGO_PKG_VERSION").into(),
        player_id: identity.id().0,
        tick_rate: TICK_RATE,
        mod_manifest_hash: [0; 32],
    });
    write_message(&mut send, &welcome)
        .await
        .map_err(|e| proto(&e))?;
    let registry = ServerControl::Registry(RegistrySync {
        block_states: ctx.block_states.as_ref().clone(),
        component_schema_hash: [0; 32],
    });
    write_message(&mut send, &registry)
        .await
        .map_err(|e| proto(&e))?;
    Ok((
        identity.id().to_string(),
        identity.name().to_string(),
        (send, recv),
    ))
}

async fn play(
    conn: Connection,
    ctx: Arc<ConnContext>,
    id: String,
    name: String,
    (mut ctrl_send, mut ctrl_recv): (SendStream, quinn::RecvStream),
) {
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<ToConn>();
    if ctx
        .world
        .send(ToWorld::Join {
            id: id.clone(),
            name: name.clone(),
            out: out_tx,
        })
        .is_err()
    {
        close(&conn, code::NORMAL, "server is shutting down");
        return;
    }
    let mut events: Option<SendStream> = None;

    // The control stream is read in its own task: `read_message` is not
    // cancel-safe, so it cannot sit in the `select!` below.
    let (ctrl_tx, mut ctrl_rx) = mpsc::channel(4);
    let reader = tokio::spawn(async move {
        loop {
            let msg = read_message::<ClientControl>(&mut ctrl_recv).await;
            let end = !matches!(msg, Ok(Some(_)));
            if ctrl_tx.send(msg).await.is_err() || end {
                break;
            }
        }
    });

    let mut tokens = DATAGRAM_BURST;
    let mut last = Instant::now();
    let mut bad = 0u32;
    let reason = loop {
        tokio::select! {
            out = out_rx.recv() => match out {
                None => break (code::NORMAL, "server is shutting down".to_string()),
                Some(ToConn::Spawn(s)) => {
                    if write_message(&mut ctrl_send, &ServerControl::Spawn(s)).await.is_err() {
                        break (code::NORMAL, "connection lost".into());
                    }
                }
                Some(ToConn::Column { pos, bytes, priority }) => {
                    let (conn, world, id) = (conn.clone(), ctx.world.clone(), id.clone());
                    tokio::spawn(async move {
                        let ok = send_column(&conn, &bytes, priority).await;
                        let _ = world.send(ToWorld::Delivered { id, pos, ok });
                    });
                }
                Some(ToConn::Datagram(bytes)) => {
                    // Unreliable by design: the next tick's datagram supersedes it.
                    let _ = conn.send_datagram(bytes.into());
                }
                Some(ToConn::Forget(c)) => {
                    let msg = ServerEvent::ForgetColumn(c.into());
                    if events.is_none() {
                        events = conn.open_bi().await.ok().map(|(s, _)| s);
                    }
                    let sent = match events.as_mut() {
                        Some(s) => write_message(s, &msg).await.is_ok(),
                        None => false,
                    };
                    if !sent {
                        break (code::NORMAL, "connection lost".into());
                    }
                }
                Some(ToConn::Kick(reason, message)) => {
                    let _ = write_message(
                        &mut ctrl_send,
                        &ServerControl::Kick { reason, message: message.clone() },
                    )
                    .await;
                    let _ = ctrl_send.finish();
                    let _ = tokio::time::timeout(Duration::from_secs(1), ctrl_send.stopped()).await;
                    break (code::REFUSED, message);
                }
            },
            d = conn.read_datagram() => match d {
                Err(_) => break (code::NORMAL, "connection lost".into()),
                Ok(bytes) => {
                    let now = Instant::now();
                    tokens = (tokens + now.duration_since(last).as_secs_f64() * DATAGRAMS_PER_SECOND)
                        .min(DATAGRAM_BURST);
                    last = now;
                    if tokens < 1.0 {
                        continue; // over the rate: dropped
                    }
                    tokens -= 1.0;
                    match InputDatagram::decode(&bytes) {
                        Ok(d) => {
                            let _ = ctx.world.send(ToWorld::Intents {
                                id: id.clone(),
                                intents: d.intents,
                                ack_snapshot: d.ack_snapshot,
                            });
                        }
                        Err(_) => {
                            bad += 1;
                            if bad > MAX_BAD_DATAGRAMS {
                                break (code::PROTOCOL, "too many malformed datagrams".into());
                            }
                        }
                    }
                }
            },
            c = ctrl_rx.recv() => match c {
                Some(Ok(Some(ClientControl::Leave))) | Some(Ok(None)) | None => {
                    break (code::NORMAL, "left".into());
                }
                Some(Ok(Some(ClientControl::Hello(_)))) => {
                    break (code::PROTOCOL, "Hello after login".into());
                }
                Some(Err(e)) => break (code::PROTOCOL, e.to_string()),
            },
        }
    };
    reader.abort();
    let _ = ctx.world.send(ToWorld::Leave { id });
    close(&conn, reason.0, &reason.1);
}

/// Finishes a stream and waits (briefly) until the peer has it, so that a
/// refusal is read before the connection closes.
async fn flush(mut send: SendStream) {
    let _ = send.finish();
    let _ = tokio::time::timeout(Duration::from_secs(1), send.stopped()).await;
}

async fn send_column(conn: &Connection, bytes: &[u8], priority: i32) -> bool {
    let Ok(mut s) = conn.open_uni().await else {
        return false;
    };
    let _ = s.set_priority(priority);
    s.write_all(bytes).await.is_ok() && s.finish().is_ok() && s.stopped().await.is_ok()
}
