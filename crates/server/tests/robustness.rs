//! The server never panics on malformed client input and keeps serving
//! (T2-02 acceptance): garbage handshakes, oversized frames, other protocol
//! versions, rejected identities, garbage and flooded datagrams.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use auth::AdmissionPolicy;
use bytes::Bytes;
use net::client::{JoinError, JoinParams, PendingClient, Trust};
use net::frame::{frame_body, read_frame};
use protocol::handshake::encode_with_version;
use protocol::{ClaimedIdentity, HandshakeError, Hello, PROTOCOL_VERSION, Rejection, ServerHello};
use server::{ServerHandle, ServerOptions};

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mods/core/data")
}

fn temp_world(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "server-test-{tag}-{}-{:?}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn start(tag: &str, offline: bool) -> (ServerHandle, PathBuf) {
    let dir = temp_world(tag);
    let mut opts = ServerOptions::new(dir.clone(), data_dir());
    opts.listen = SocketAddr::from(([127, 0, 0, 1], 0));
    opts.policy = AdmissionPolicy {
        allow_offline: offline,
    };
    opts.seed = Some(7);
    opts.view_distance = 4;
    opts.threads = 2;
    (server::start(opts).expect("server starts"), dir)
}

fn hello(name: &str) -> Hello {
    Hello {
        client_version: "test".into(),
        identity: ClaimedIdentity::Offline { name: name.into() },
        session_jwt: None,
    }
}

fn join(
    server: &ServerHandle,
    name: &str,
) -> Result<(PendingClient, net::client::Joined), JoinError> {
    PendingClient::connect(&JoinParams {
        addr: server.local_addr(),
        server_name: "localhost".into(),
        hello: hello(name),
        trust: Trust::Certificate(server.certificate().clone()),
        timeout: Duration::from_secs(10),
    })
}

/// Opens a raw connection and the control stream, sends `bytes`, and returns
/// what the server answered (if anything) and how it closed the connection.
fn raw_exchange(server: &ServerHandle, bytes: Vec<u8>) -> (Option<Vec<u8>>, String) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let cert = server.certificate().clone();
    let addr = server.local_addr();
    rt.block_on(async move {
        let endpoint = net::client_endpoint(cert).expect("endpoint");
        let conn = endpoint
            .connect(addr, "localhost")
            .expect("connect")
            .await
            .expect("connected");
        let (mut send, mut recv) = conn.open_bi().await.expect("stream");
        send.write_all(&bytes).await.expect("write");
        let answer = tokio::time::timeout(Duration::from_secs(5), read_frame(&mut recv, 1 << 20))
            .await
            .ok()
            .and_then(Result::ok);
        let reason = tokio::time::timeout(Duration::from_secs(5), conn.closed())
            .await
            .map_or_else(|_| "still open".to_string(), |e| e.to_string());
        (answer, reason)
    })
}

#[test]
fn garbage_and_wrong_versions_are_refused_and_the_server_keeps_serving() {
    let (server, dir) = start("garbage", true);

    // Not our protocol at all.
    let (answer, reason) = raw_exchange(
        &server,
        frame_body(b"GET / HTTP/1.1\r\n\r\n").expect("frame"),
    );
    assert!(answer.is_none());
    assert!(reason.contains("bad handshake"), "{reason}");

    // A frame length of 4 GiB: refused before allocating.
    let (_, reason) = raw_exchange(&server, u32::MAX.to_le_bytes().to_vec());
    assert!(reason.contains("bad handshake"), "{reason}");

    // Random bytes behind a valid prefix.
    let mut seed = 0x1234_5678_u64;
    for _ in 0..20 {
        let body: Vec<u8> = (0..64)
            .map(|_| {
                seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                (seed >> 33) as u8
            })
            .collect();
        let bytes = encode_with_version(PROTOCOL_VERSION, &body);
        let (_, reason) = raw_exchange(&server, frame_body(&bytes).expect("frame"));
        assert!(reason.contains("bad handshake"), "{reason}");
    }

    // A newer client gets the version prefix back, so it can tell the player
    // which side must update.
    let newer = encode_with_version(PROTOCOL_VERSION + 1, b"future");
    let (answer, reason) = raw_exchange(&server, frame_body(&newer).expect("frame"));
    assert_eq!(
        answer.as_deref(),
        Some(ServerHello::encode_version_mismatch().as_slice())
    );
    assert!(reason.contains("version"), "{reason}");

    // After all that, a real client still joins.
    let (_pending, joined) = join(&server, "Survivor").expect("joins");
    assert!(!joined.registry.id_table.is_empty());
    assert_eq!(joined.welcome.server_info.view_distance, 4);
    drop(server);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn admission_rules() {
    let (online, dir) = start("online", false);
    match join(&online, "Alex") {
        Err(JoinError::Handshake(HandshakeError::Rejected(Rejection::OfflineNotAllowed))) => {}
        other => panic!("expected OfflineNotAllowed, got {:?}", other.map(|_| ())),
    }
    drop(online);
    let _ = std::fs::remove_dir_all(dir);

    let (server, dir) = start("admission", true);
    match join(&server, "x") {
        Err(JoinError::Handshake(HandshakeError::Rejected(Rejection::InvalidName))) => {}
        other => panic!("expected InvalidName, got {:?}", other.map(|_| ())),
    }
    let (first, _) = join(&server, "Alex").expect("joins");
    let client = first.start(|c| Ok(c.pos()));
    // Wait until the first player is placed (Spawn), then try the same name.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(net::client::NetEvent::Spawn(_)) = client.try_event() {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "no spawn");
        std::thread::sleep(Duration::from_millis(10));
    }
    match join(&server, "Alex") {
        Err(JoinError::Handshake(HandshakeError::Rejected(Rejection::AlreadyConnected))) => {}
        other => panic!("expected AlreadyConnected, got {:?}", other.map(|_| ())),
    }
    client.disconnect();
    drop(server);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn garbage_datagrams_and_floods_do_not_hurt() {
    let (server, dir) = start("datagrams", true);
    let (pending, _) = join(&server, "Flooder").expect("joins");
    let client = pending.start(|c| Ok(c.pos()));
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    // A separate raw connection sends garbage after a valid join.
    let cert = server.certificate().clone();
    let addr = server.local_addr();
    let closed = rt.block_on(async move {
        let endpoint = net::client_endpoint(cert).expect("endpoint");
        let conn = endpoint
            .connect(addr, "localhost")
            .expect("connect")
            .await
            .expect("connected");
        let (mut send, mut recv) = conn.open_bi().await.expect("stream");
        let hello = frame_body(&hello("Garbage").encode()).expect("frame");
        send.write_all(&hello).await.expect("write");
        let welcome = read_frame(&mut recv, 1 << 16).await.expect("welcome");
        ServerHello::decode(&welcome).expect("accepted");
        let _registry = read_frame(&mut recv, 1 << 22).await.expect("registry");
        let ready = protocol::ClientControl::Ready
            .encode_frame()
            .expect("frame");
        send.write_all(&ready).await.expect("ready");
        let _spawn = read_frame(&mut recv, 1 << 16).await.expect("spawn");
        // Garbage datagrams, then a flood far above the rate limit.
        let mut seed = 99u64;
        for i in 0..20_000u32 {
            let len = 1 + (i as usize % 1100);
            let junk: Vec<u8> = (0..len)
                .map(|_| {
                    seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                    (seed >> 40) as u8
                })
                .collect();
            if conn.send_datagram(Bytes::from(junk)).is_err() {
                break;
            }
            if i % 500 == 0 {
                tokio::task::yield_now().await;
            }
        }
        // Then an oversized control frame closes the connection.
        let _ = send.write_all(&(1u32 << 30).to_le_bytes()).await;
        tokio::time::timeout(Duration::from_secs(10), conn.closed())
            .await
            .map_or_else(|_| "still open".to_string(), |e| e.to_string())
    });
    assert!(
        closed.contains("oversized") || closed.contains("flood"),
        "{closed}"
    );
    // The well-behaved client is still connected and receives columns.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut columns = 0;
    while columns < 20 {
        match client.try_event() {
            Some(net::client::NetEvent::Column(_)) => columns += 1,
            Some(net::client::NetEvent::Closed(why)) => panic!("closed: {why}"),
            _ => std::thread::sleep(Duration::from_millis(5)),
        }
        assert!(std::time::Instant::now() < deadline, "columns stopped");
    }
    client.disconnect();
    drop(server);
    let _ = std::fs::remove_dir_all(dir);
}
