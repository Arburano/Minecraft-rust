//! A headless client joins a real server (in this process) and plays in the
//! streamed world (T2-02 acceptance). The link goes through `LossyRelay`:
//! 2% loss per direction, RTT 100 ms.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use auth::{AdmissionPolicy, Identity};
use client::session::{ConnectParams, Session, SessionConfig};
use net::lossy::LossyRelay;
use server::{ServerHandle, ServerOptions};
use sim::physics::PlayerInput;
use world::ColumnPos;

const TICK: Duration = Duration::from_millis(50);

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mods/core/data")
}

fn temp_world(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "mp-test-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn start_server(tag: &str, view: u8) -> (ServerHandle, PathBuf) {
    let dir = temp_world(tag);
    let mut opts = ServerOptions::new(dir.clone(), data_dir());
    opts.listen = SocketAddr::from(([127, 0, 0, 1], 0));
    opts.policy = AdmissionPolicy {
        allow_offline: true,
    };
    opts.seed = Some(42);
    opts.view_distance = view;
    opts.threads = 2;
    (server::start(opts).expect("server starts"), dir)
}

fn connect(addr: SocketAddr, name: &str) -> Session {
    let mut config = SessionConfig::new(data_dir());
    config.threads = 2;
    Session::connect(
        &ConnectParams {
            addr,
            identity: Identity::offline(name).expect("valid name"),
            timeout: Duration::from_secs(10),
        },
        config,
    )
    .expect("joins")
}

/// Runs frames until `done` or the limit; fixed ticks at 20 TPS.
fn run_until(
    s: &mut Session,
    limit: Duration,
    input: impl Fn(u32) -> PlayerInput,
    mut done: impl FnMut(&Session) -> bool,
) -> bool {
    let start = Instant::now();
    let mut next_tick = start;
    let mut ticks = 0u32;
    while start.elapsed() < limit {
        s.update().expect("session runs");
        if done(s) {
            return true;
        }
        if Instant::now() >= next_tick {
            s.fixed_tick(&input(ticks));
            ticks += 1;
            next_tick += TICK;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    false
}

#[test]
fn joins_over_a_lossy_link_and_controls_the_player_quickly() {
    let (server, dir) = start_server("lossy", 8);
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let relay = rt
        .block_on(LossyRelay::spawn(
            server.local_addr(),
            Duration::from_millis(50),
            0.02,
            0x5EED,
        ))
        .expect("relay");
    let mut s = connect(relay.local_addr(), "Lossy");
    assert!(s.is_remote());
    assert_eq!(s.info().name, "Server");
    let playing = run_until(
        &mut s,
        Duration::from_secs(20),
        |_| PlayerInput::default(),
        |s| s.is_playing(),
    );
    assert!(playing, "the player never became controllable");
    let to_control = s.time_to_control().expect("measured");
    eprintln!(
        "lossy join: connect + handshake {:?}, to control {:?}, columns {}",
        s.connect_time().unwrap_or_default(),
        to_control,
        s.columns_received().unwrap_or(0)
    );
    // Networking-protocol.md: the nearest 3×3 controllable in ≤ 1 s at 2%
    // loss / RTT 100 ms, counted from the start of connecting and including
    // the client's lighting and meshing of the 3×3 (VM dev build: ≈ 0.7 s).
    assert!(
        to_control <= Duration::from_secs(1),
        "time to control {to_control:?}"
    );
    // The rest of the view streams in the background: (2·8+1)² columns.
    let all = run_until(
        &mut s,
        Duration::from_secs(30),
        |_| PlayerInput::default(),
        |s| s.columns_received().unwrap_or(0) >= 289,
    );
    assert!(all, "only {:?} columns", s.columns_received());
    // Standing still the client and the server agree on the position:
    // the player stands on the ground.
    assert!(s.player().on_ground || s.player().in_fluid);
    s.close().expect("leaves");
    relay.abort();
    drop(server);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn walking_streams_new_columns_and_drops_old_ones() {
    let (server, dir) = start_server("walk", 5);
    let mut s = connect(server.local_addr(), "Walker");
    assert!(run_until(
        &mut s,
        Duration::from_secs(20),
        |_| PlayerInput::default(),
        |s| { s.is_playing() }
    ));
    let start = s.player().pos;
    let start_col = ColumnPos {
        x: (start.x / 16.0).floor() as i32,
        z: (start.z / 16.0).floor() as i32,
    };
    // Fly forward (yaw 0 = −Z) fast for a while: toggle flight, full speed.
    let fly = |t: u32| PlayerInput {
        forward: 1.0,
        toggle_fly: t == 0,
        sprint: true,
        ..PlayerInput::default()
    };
    s.set_fly_speed(f64::MAX);
    let moved = run_until(&mut s, Duration::from_secs(30), fly, |s| {
        (s.player().pos.z - start.z).abs() > 16.0 * 12.0
    });
    assert!(moved, "flew only to {:?} from {start:?}", s.player().pos);
    // The world follows the player: once it stops, the columns around the new
    // place are there and the start area has been unloaded.
    let ready = run_until(
        &mut s,
        Duration::from_secs(20),
        |_| PlayerInput::default(),
        |s| s.is_playing(),
    );
    assert!(ready, "the new area never became ready");
    assert!(
        s.map().column(start_col).is_none(),
        "the start column should be unloaded"
    );
    s.close().expect("leaves");

    // Back again: the server resends what the client dropped; rejoining
    // places the player where it left (the server saved it).
    let left_at = s_pos_after_rejoin(&server);
    assert!(
        (left_at.z - start.z).abs() > 16.0 * 10.0,
        "rejoined at {left_at:?}, start {start:?}"
    );
    drop(server);
    let _ = std::fs::remove_dir_all(dir);
}

fn s_pos_after_rejoin(server: &ServerHandle) -> glam::DVec3 {
    // Give the server a moment to save the player who left.
    std::thread::sleep(Duration::from_millis(300));
    let mut s = connect(server.local_addr(), "Walker");
    assert!(run_until(
        &mut s,
        Duration::from_secs(20),
        |_| PlayerInput::default(),
        |s| { s.is_playing() }
    ));
    let pos = s.player().pos;
    s.close().expect("leaves");
    pos
}

#[test]
fn unreachable_server_is_a_clear_error() {
    let mut config = SessionConfig::new(data_dir());
    config.threads = 1;
    // Nothing listens on this port: a clear error, no hang.
    let nobody = Session::connect(
        &ConnectParams {
            addr: SocketAddr::from(([127, 0, 0, 1], 9)),
            identity: Identity::offline("Nobody").expect("valid"),
            timeout: Duration::from_secs(2),
        },
        config,
    );
    let err = nobody.err().expect("no server");
    assert!(
        err.to_string().contains("connect") || err.to_string().contains("time"),
        "{err}"
    );
}
