//! T2-02: the game client plays on a dedicated server.
//!
//! A real `server::Server` in the same process, the client's `Session` in
//! multiplayer mode (columns from the network, light and meshes on the
//! client), optionally through `net::lossy::LossyRelay`.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use auth::AdmissionPolicy;
use client::session::{Session, SessionConfig};
use server::{Server, ServerConfig};
use sim::physics::PlayerInput;

fn data() -> world::ModSet {
    world::modset::source_build(world::modset::DEFAULT_BUILD).expect("build")
}

fn start(tag: &str) -> (Server, PathBuf) {
    start_build(tag, data())
}

fn start_build(tag: &str, mods: world::ModSet) -> (Server, PathBuf) {
    let dir = std::env::temp_dir().join(format!("client-remote-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut config = ServerConfig::new(
        "127.0.0.1:0".parse().expect("addr"),
        AdmissionPolicy {
            allow_offline: true,
        },
        dir.clone(),
        mods,
    );
    config.seed = Some(7);
    config.view_distance = 10;
    config.threads = 2;
    (Server::start(config).expect("server"), dir)
}

fn session_config() -> SessionConfig {
    let mut c = SessionConfig::new(data());
    c.radius = 8;
    c.threads = 2;
    c
}

/// Runs frames (`update` every 5 ms, a tick every 50 ms) until `done` or `limit`.
fn run(
    s: &mut Session,
    limit: Duration,
    input: &PlayerInput,
    mut done: impl FnMut(&Session) -> bool,
) -> Option<Duration> {
    let start = Instant::now();
    let mut next_tick = start;
    while start.elapsed() < limit {
        s.update().expect("session update");
        if done(s) {
            return Some(start.elapsed());
        }
        if Instant::now() >= next_tick {
            s.fixed_tick(input);
            next_tick += Duration::from_millis(50);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    None
}

fn join_time(addr: SocketAddr, name: &str) -> (Session, Duration) {
    let start = Instant::now();
    let mut s = Session::connect(addr, name, session_config()).expect("join");
    let joined = start.elapsed();
    println!("  {name}: login + spawn {joined:?}");
    let playing = run(
        &mut s,
        Duration::from_secs(60),
        &PlayerInput::default(),
        |s| s.is_playing(),
    )
    .expect("the nearest columns become playable");
    (s, joined + playing)
}

#[test]
fn walking_on_the_server_matches_the_server_simulation() {
    let (server, dir) = start("walk");
    let (mut s, t) = join_time(server.local_addr(), "Walker");
    println!("join to control (loopback, debug build): {t:?}");
    assert!(s.is_remote());

    // Walk forward and turn; the server runs the same physics on the intents.
    let start = s.player().pos;
    let walk = PlayerInput {
        forward: 1.0,
        sprint: true,
        ..PlayerInput::default()
    };
    let mut ticks = 0;
    run(&mut s, Duration::from_secs(10), &walk, |_| {
        ticks += 1;
        false
    });
    s.look(0.7, 0.0);
    run(&mut s, Duration::from_secs(2), &walk, |_| false);
    let end = s.player().pos;
    assert!(
        (end - start).length() > 5.0,
        "the player moved: {start} → {end}"
    );
    // Let the last intents reach the server, then leave (it saves the player).
    run(
        &mut s,
        Duration::from_millis(800),
        &PlayerInput::default(),
        |_| false,
    );
    let final_pos = s.player().pos;
    s.close().expect("leave");
    std::thread::sleep(Duration::from_millis(500));
    server.shutdown();

    let file = std::fs::read_dir(dir.join("players"))
        .expect("players")
        .next()
        .expect("one player")
        .expect("entry")
        .path();
    let text = std::fs::read_to_string(file).expect("player file");
    let saved: toml::Value = toml::from_str(&text).expect("toml");
    let pos: Vec<f64> = saved["pos"]
        .as_array()
        .expect("pos")
        .iter()
        .map(|v| v.as_float().expect("float"))
        .collect();
    let diff = ((pos[0] - final_pos.x).powi(2)
        + (pos[1] - final_pos.y).powi(2)
        + (pos[2] - final_pos.z).powi(2))
    .sqrt();
    println!("client {final_pos} vs server {pos:?}: {diff:.6} blocks apart");
    assert!(
        diff < 1e-6,
        "client and server simulations diverged by {diff} blocks"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// Acceptance (T2-02): at 2% loss and RTT 100 ms the nearest 3×3 columns are
/// controllable (received, lit and meshed on the client) quickly. The 1 s
/// target is checked in release builds; debug builds report the time and
/// only guard against a gross regression.
#[test]
fn nearest_columns_are_controllable_under_loss() {
    let (server, dir) = start("loss");
    // Warm the server's spawn area: a running server keeps its world.
    let (warm, _) = join_time(server.local_addr(), "Warmup");
    warm.close().expect("leave");

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("runtime");
    let relay = rt
        .block_on(net::lossy::LossyRelay::spawn(
            server.local_addr(),
            Duration::from_millis(50),
            std::env::var("LOSS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.02),
            std::env::var("SEED")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(42),
        ))
        .expect("relay");
    let (s, t) = join_time(relay.local_addr(), "Tester");
    println!("join to control at 2% loss, RTT 100 ms: {t:?}");
    let limit = if cfg!(debug_assertions) {
        Duration::from_secs(10)
    } else {
        Duration::from_secs(1)
    };
    assert!(t <= limit, "took {t:?}, limit {limit:?}");
    s.close().expect("leave");
    rt.block_on(async { relay.abort() });
    drop(rt);
    server.shutdown();
    let _ = std::fs::remove_dir_all(dir);
}

/// T1-18: with a movement profile that has abilities (the `classic_agile`
/// build), the client's prediction and the server's simulation agree: dash,
/// double jump, glide and crawl cause no corrections.
#[test]
fn abilities_predict_without_corrections() {
    let agile = world::modset::source_build("classic_agile").expect("build");
    let (server, dir) = start_build("agile", agile.clone());
    let mut config = session_config();
    config.mods = agile;
    let mut s = Session::connect(server.local_addr(), "Agile", config).expect("join");
    run(
        &mut s,
        Duration::from_secs(60),
        &PlayerInput::default(),
        |s| s.is_playing(),
    )
    .expect("playable");
    // Settle on the ground.
    run(
        &mut s,
        Duration::from_secs(2),
        &PlayerInput::default(),
        |_| false,
    );
    let walk = PlayerInput {
        forward: 1.0,
        ..PlayerInput::default()
    };
    let crouch = PlayerInput {
        descend: true,
        ..PlayerInput::default()
    };
    let script: [(PlayerInput, u64); 11] = [
        (crouch, 500), // crawl box on the spawn
        (PlayerInput::default(), 300),
        (walk, 600),
        (
            PlayerInput {
                sprint: true,
                ..walk
            },
            150,
        ), // dash
        (walk, 400),
        (PlayerInput { jump: true, ..walk }, 150), // jump
        (walk, 150),
        (PlayerInput { jump: true, ..walk }, 150), // double jump
        (
            PlayerInput {
                descend: true,
                ..walk
            },
            600,
        ), // glide, then crawl
        (walk, 600),
        (PlayerInput::default(), 1000),
    ];
    let start = s.player().pos;
    let mut fired = 0u8;
    for (input, ms) in script {
        run(&mut s, Duration::from_millis(ms), &input, |s| {
            fired |= s.player().abilities.active;
            false
        });
    }
    println!("abilities that fired: {fired:#07b}");
    // dash (bit 0), double jump (bit 1), crawl (bit 3) at least.
    assert_eq!(fired & 0b1011, 0b1011, "{fired:#07b}");
    let end = s.player().pos;
    let stats = s.net_stats().expect("remote");
    println!(
        "moved {:.1} blocks; corrections {} (max {:.4})",
        (end - start).length(),
        stats.corrections,
        stats.max_correction
    );
    assert!((end - start).length() > 3.0, "the player moved");
    assert_eq!(stats.corrections, 0, "prediction matched the server");
    s.close().expect("leave");
    server.shutdown();
    let _ = std::fs::remove_dir_all(dir);
}
