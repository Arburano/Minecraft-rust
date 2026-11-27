//! T2-02: a client joins the dedicated server and receives the world.
//!
//! The tests talk to a real `Server` over QUIC with a minimal protocol client
//! (the game client's own test is `crates/client/tests/remote.rs`). Loss and
//! delay are emulated with `net::lossy::LossyRelay`.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use auth::AdmissionPolicy;
use net::messages::{read_header, read_message, write_header, write_message};
use protocol::{
    Buttons, ChunkStream, ClientControl, HandshakeHeader, Hello, InputDatagram, Intent, Message,
    ProtocolError, ServerControl, ServerEvent, Spawn,
};
use quinn::Connection;
use server::{Server, ServerConfig};
use world::ColumnPos;

fn temp_world(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("server-join-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn start(tag: &str, view: i32) -> (Server, PathBuf) {
    let dir = temp_world(tag);
    let data = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mods/core/data");
    let mut config = ServerConfig::new(
        "127.0.0.1:0".parse().expect("addr"),
        AdmissionPolicy {
            allow_offline: true,
        },
        dir.clone(),
        data,
    );
    config.seed = Some(7);
    config.view_distance = view;
    config.threads = 2;
    (Server::start(config).expect("server starts"), dir)
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("runtime")
}

struct Joined {
    conn: Connection,
    ctrl: (quinn::SendStream, quinn::RecvStream),
    spawn: Spawn,
    states: usize,
}

async fn connect(addr: SocketAddr) -> anyhow::Result<Connection> {
    let endpoint = net::client_endpoint_offline("0.0.0.0:0".parse()?)?;
    Ok(endpoint.connect(addr, "localhost")?.await?)
}

async fn join(addr: SocketAddr, name: &str) -> anyhow::Result<Joined> {
    let conn = connect(addr).await?;
    let (mut send, mut recv) = conn.open_bi().await?;
    write_header(&mut send).await?;
    write_message(
        &mut send,
        &ClientControl::Hello(Hello {
            client_version: "test".into(),
            player_name: name.into(),
            session_jwt: None,
        }),
    )
    .await?;
    read_header(&mut recv).await?.check()?;
    let Some(ServerControl::Welcome(w)) = read_message(&mut recv).await? else {
        anyhow::bail!("expected Welcome");
    };
    assert_eq!(w.tick_rate, 20);
    let Some(ServerControl::Registry(r)) = read_message(&mut recv).await? else {
        anyhow::bail!("expected Registry");
    };
    let Some(ServerControl::Spawn(spawn)) = read_message(&mut recv).await? else {
        anyhow::bail!("expected Spawn");
    };
    Ok(Joined {
        conn,
        ctrl: (send, recv),
        spawn,
        states: r.block_states.len(),
    })
}

/// Receives columns until `want` are all there or `limit` passes.
async fn receive_until(
    conn: &Connection,
    got: &mut HashSet<ColumnPos>,
    want: &HashSet<ColumnPos>,
    limit: Duration,
) -> anyhow::Result<()> {
    let deadline = Instant::now() + limit;
    while !want.is_subset(got) {
        let left = deadline.saturating_duration_since(Instant::now());
        let mut s = tokio::time::timeout(left, conn.accept_uni()).await??;
        let bytes = s.read_to_end(ChunkStream::MAX_LEN).await?;
        let ChunkStream::Column(c) = ChunkStream::decode(&bytes)?;
        let record = c.decode()?;
        got.insert(record.pos);
    }
    Ok(())
}

fn around(c: ColumnPos, r: i32) -> HashSet<ColumnPos> {
    let mut s = HashSet::new();
    for dx in -r..=r {
        for dz in -r..=r {
            s.insert(ColumnPos {
                x: c.x + dx,
                z: c.z + dz,
            });
        }
    }
    s
}

fn column_of(p: [f64; 3]) -> ColumnPos {
    ColumnPos {
        x: (p[0] / 16.0).floor() as i32,
        z: (p[2] / 16.0).floor() as i32,
    }
}

/// Acceptance: with 2% loss and RTT 100 ms the nearest 3×3 columns arrive
/// within 1 s of the connection attempt (handshake and spawn included).
#[test]
fn nearest_columns_arrive_within_a_second_under_loss() {
    let (server, dir) = start("loss", 10);
    let rt = runtime();
    let elapsed = rt
        .block_on(async {
            // Warm the spawn area so the measurement is the network, not the
            // first-ever generation of the world (a server keeps its world).
            let warm = join(server.local_addr(), "Warmup").await?;
            let mut got = HashSet::new();
            let want = around(column_of(warm.spawn.position), 1);
            receive_until(&warm.conn, &mut got, &want, Duration::from_secs(20)).await?;
            warm.conn.close(0u32.into(), b"done");

            let relay = net::lossy::LossyRelay::spawn(
                server.local_addr(),
                Duration::from_millis(50),
                0.02,
                42,
            )
            .await?;
            let start = Instant::now();
            let j = join(relay.local_addr(), "Tester").await?;
            let want = around(column_of(j.spawn.position), 1);
            let mut got = HashSet::new();
            receive_until(&j.conn, &mut got, &want, Duration::from_secs(10)).await?;
            let t = start.elapsed();
            relay.abort();
            anyhow::Ok(t)
        })
        .expect("join under loss");
    println!("nearest 3x3 columns at 2% loss, RTT 100 ms: {elapsed:?}");
    assert!(
        elapsed <= Duration::from_secs(1),
        "nearest 3x3 took {elapsed:?}, budget is 1 s"
    );
    drop(rt);
    server.shutdown();
    let _ = std::fs::remove_dir_all(dir);
}

/// Columns follow the player: flying away makes the server stream the new
/// area and tell the client to forget the old one; the player file is saved.
#[test]
fn streaming_follows_the_player_and_forgets_behind() {
    let (server, dir) = start("move", 3);
    let rt = runtime();
    rt.block_on(async {
        let j = join(server.local_addr(), "Flyer").await?;
        assert!(j.states > 2);
        let start_col = column_of(j.spawn.position);
        let mut got = HashSet::new();
        receive_until(
            &j.conn,
            &mut got,
            &around(start_col, 3),
            Duration::from_secs(20),
        )
        .await?;

        // Events stream (ForgetColumn) is opened by the server when needed.
        let conn = j.conn.clone();
        let forgot = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = std::sync::Arc::clone(&forgot);
        tokio::spawn(async move {
            let (_, mut recv) = conn.accept_bi().await?;
            while let Some(ServerEvent::ForgetColumn(_)) = read_message(&mut recv).await? {
                counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            anyhow::Ok(())
        });

        // Toggle flight, max fly speed, then sprint forward (yaw 0 = −Z).
        let mut tick = 0u32;
        let mut recent: Vec<Intent> = Vec::new();
        let mut send = |buttons: u16, steps: i8| {
            recent.push(Intent {
                tick,
                buttons: Buttons(buttons),
                yaw: protocol::Angle16(0),
                pitch: protocol::Angle16(0),
                fly_speed_steps: steps,
                state_hash: 0,
            });
            if recent.len() > 3 {
                recent.remove(0);
            }
            tick += 1;
            let d = InputDatagram {
                ack_snapshot: 0,
                intents: recent.clone(),
            };
            j.conn.send_datagram(d.encode().expect("encode").into())
        };
        send(Buttons::FLY_TOGGLE | Buttons::JUMP, 10)?;
        for _ in 0..60 {
            send(Buttons::FORWARD | Buttons::SPRINT, 0)?;
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while forgot.load(std::sync::atomic::Ordering::Relaxed) < 3 && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let n = forgot.load(std::sync::atomic::Ordering::Relaxed);
        assert!(n >= 3, "forgot {n} columns");
        // The server moved the player far towards −Z: columns there arrive.
        let mut far = HashSet::new();
        let ahead = ColumnPos {
            x: start_col.x,
            z: start_col.z - 8,
        };
        receive_until(
            &j.conn,
            &mut far,
            &around(ahead, 0),
            Duration::from_secs(20),
        )
        .await?;
        let (mut send, _) = j.ctrl;
        write_message(&mut send, &ClientControl::Leave).await?;
        send.finish()?;
        tokio::time::sleep(Duration::from_millis(300)).await;
        anyhow::Ok(())
    })
    .expect("moving player");
    drop(rt);
    server.shutdown();
    let players = std::fs::read_dir(dir.join("players"))
        .expect("players folder")
        .count();
    assert_eq!(players, 1, "the player was saved");
    let _ = std::fs::remove_dir_all(dir);
}

/// Malformed input never takes the server down: a wrong version is told
/// apart, garbage is refused, and a normal client still joins afterwards.
#[test]
fn garbage_and_wrong_versions_do_not_hurt_the_server() {
    let (server, dir) = start("garbage", 2);
    let addr = server.local_addr();
    let rt = runtime();
    rt.block_on(async {
        // A newer client: the server answers with its header and refuses.
        let conn = connect(addr).await?;
        let (mut send, mut recv) = conn.open_bi().await?;
        let newer = HandshakeHeader {
            protocol: protocol::PROTOCOL_VERSION + 1,
        };
        send.write_all(&newer.encode()).await?;
        let theirs = read_header(&mut recv).await?;
        assert_eq!(theirs, HandshakeHeader::current());
        let err = newer.check().expect_err("mismatch seen by the newer side");
        assert!(matches!(err, ProtocolError::VersionMismatch { .. }));

        // Not this protocol at all.
        let conn = connect(addr).await?;
        let (mut send, _recv) = conn.open_bi().await?;
        send.write_all(b"GET / HTTP/1.1\r\n\r\n").await?;
        let reason = tokio::time::timeout(Duration::from_secs(5), conn.closed()).await?;
        println!("garbage header: {reason}");

        // A huge frame length after a valid header.
        let conn = connect(addr).await?;
        let (mut send, _recv) = conn.open_bi().await?;
        write_header(&mut send).await?;
        send.write_all(&u32::MAX.to_le_bytes()).await?;
        tokio::time::timeout(Duration::from_secs(5), conn.closed()).await?;

        // A bad name is refused with a Kick.
        let conn = connect(addr).await?;
        let (mut send, mut recv) = conn.open_bi().await?;
        write_header(&mut send).await?;
        write_message(
            &mut send,
            &ClientControl::Hello(Hello {
                client_version: "test".into(),
                player_name: "no spaces allowed".into(),
                session_jwt: None,
            }),
        )
        .await?;
        read_header(&mut recv).await?;
        let kick = read_message::<ServerControl>(&mut recv).await?;
        assert!(matches!(kick, Some(ServerControl::Kick { .. })), "{kick:?}");

        // A joined client sending garbage datagrams is disconnected.
        let j = join(addr, "Noisy").await?;
        for i in 0..200u32 {
            let _ = j
                .conn
                .send_datagram(vec![0xff; (i % 50) as usize + 1].into());
            if i % 20 == 0 {
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
        }
        let reason = tokio::time::timeout(Duration::from_secs(10), j.conn.closed()).await?;
        println!("noisy client: {reason}");

        // The server still serves a normal client.
        let ok = join(addr, "Normal").await?;
        let mut got = HashSet::new();
        let want = around(column_of(ok.spawn.position), 1);
        receive_until(&ok.conn, &mut got, &want, Duration::from_secs(20)).await?;
        anyhow::Ok(())
    })
    .expect("server survives");
    drop(rt);
    server.shutdown();
    let _ = std::fs::remove_dir_all(dir);
}
