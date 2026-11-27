//! T2-03: prediction, reconciliation and other players on a real server.
//!
//! An in-process `server::Server`, game clients as `Session`s (prediction,
//! light and meshes) and light bots as bare `Remote`s that only send intents.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use auth::AdmissionPolicy;
use client::remote::Remote;
use client::session::{Session, SessionConfig};
use glam::DVec3;
use protocol::{Angle16, Buttons, InputDatagram, Intent};
use server::{Server, ServerConfig};
use sim::physics::PlayerInput;

fn data() -> world::ModSet {
    world::modset::source_build(world::modset::DEFAULT_BUILD).expect("build")
}

fn start(tag: &str, view: i32) -> (Server, PathBuf) {
    let dir = std::env::temp_dir().join(format!("client-repl-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut config = ServerConfig::new(
        "127.0.0.1:0".parse().expect("addr"),
        AdmissionPolicy {
            allow_offline: true,
        },
        dir.clone(),
        data(),
    );
    config.seed = Some(7);
    config.view_distance = view;
    config.threads = 2;
    (Server::start(config).expect("server"), dir)
}

fn session(addr: SocketAddr, name: &str) -> Session {
    let mut c = SessionConfig::new(data());
    c.radius = 8;
    c.threads = 2;
    Session::connect(addr, name, c).expect("join")
}

/// Frames: `update` every 5 ms, a tick every 50 ms; `each` sees every frame.
fn run(
    sessions: &mut [&mut Session],
    limit: Duration,
    mut input: impl FnMut(usize, u32) -> PlayerInput,
    mut each: impl FnMut(&[&mut Session]) -> bool,
) -> bool {
    let start = Instant::now();
    let mut next = start;
    let mut tick = 0u32;
    while start.elapsed() < limit {
        for s in sessions.iter_mut() {
            s.update().expect("update");
        }
        if each(sessions) {
            return true;
        }
        if Instant::now() >= next {
            for (i, s) in sessions.iter_mut().enumerate() {
                s.fixed_tick(&input(i, tick));
            }
            tick += 1;
            next += Duration::from_millis(50);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

fn wait_playing(s: &mut Session) {
    assert!(
        run(
            &mut [s],
            Duration::from_secs(60),
            |_, _| PlayerInput::default(),
            |s| s[0].is_playing()
        ),
        "playable"
    );
}

fn walk(t: u32) -> PlayerInput {
    PlayerInput {
        forward: 1.0,
        sprint: t % 80 < 60,
        jump: t % 37 == 0,
        strafe: if t % 100 < 50 { 0.0 } else { 1.0 },
        ..PlayerInput::default()
    }
}

/// Acceptance: at 2% loss / RTT 100 ms a scripted walk shows no visible
/// correction, because the server agrees with every prediction.
#[test]
fn scripted_walk_under_loss_needs_no_correction() {
    let (server, dir) = start("walk", 10);
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("rt");
    let relay = rt
        .block_on(net::lossy::LossyRelay::spawn(
            server.local_addr(),
            Duration::from_millis(50),
            0.02,
            42,
        ))
        .expect("relay");
    let mut s = session(relay.local_addr(), "Walker");
    wait_playing(&mut s);
    let from = s.player().pos;
    let mut turn = 0.0;
    run(
        &mut [&mut s],
        Duration::from_secs(12),
        |_, t| {
            turn = f64::from(t % 50) * 0.01;
            walk(t)
        },
        |_| false,
    );
    s.look(turn, 0.0);
    let stats = s.net_stats().expect("remote");
    println!(
        "walked {:.1} blocks; corrections {}, largest {:.4} blocks",
        (s.player().pos - from).length(),
        stats.corrections,
        stats.max_correction
    );
    assert!((s.player().pos - from).length() > 10.0, "the player walked");
    assert_eq!(stats.corrections, 0, "no correction in a scripted walk");
    s.close().expect("leave");
    rt.block_on(async { relay.abort() });
    drop(rt);
    server.shutdown();
    let _ = std::fs::remove_dir_all(dir);
}

/// Acceptance: a forced divergence is corrected, and the drawn position
/// glides back instead of jumping.
#[test]
fn forced_divergence_corrects_smoothly() {
    let (server, dir) = start("nudge", 10);
    let mut s = session(server.local_addr(), "Nudged");
    wait_playing(&mut s);
    // Stand still until the body settles.
    run(
        &mut [&mut s],
        Duration::from_secs(1),
        |_, _| PlayerInput::default(),
        |_| false,
    );
    let truth = s.player().pos;
    s.nudge_for_test(DVec3::new(0.6, 0.0, 0.0));
    let mut drawn = Vec::new();
    run(
        &mut [&mut s],
        Duration::from_secs(2),
        |_, _| PlayerInput::default(),
        |s| {
            drawn.push(s[0].camera(1.0).0);
            false
        },
    );
    let stats = s.net_stats().expect("remote");
    let back = (s.player().pos - truth).length();
    let max_step = drawn
        .windows(2)
        .map(|w| (w[1] - w[0]).length())
        .fold(0.0, f64::max);
    println!(
        "corrections {}, largest {:.3}; back within {back:.6}; largest drawn step per frame {max_step:.4}; offset left {:.5}",
        stats.corrections, stats.max_correction, stats.offset
    );
    assert!(stats.corrections >= 1, "the divergence was corrected");
    assert!(back < 1e-9, "the server's position wins");
    assert!(
        max_step < 0.25,
        "no visible jump: {max_step} blocks in one frame"
    );
    assert!(stats.offset < 0.01, "the smoothing finished");
    s.close().expect("leave");
    server.shutdown();
    let _ = std::fs::remove_dir_all(dir);
}

/// Two clients see each other move.
#[test]
fn two_players_see_each_other() {
    let (server, dir) = start("two", 10);
    let mut a = session(server.local_addr(), "Alice");
    let mut b = session(server.local_addr(), "Bob_2");
    wait_playing(&mut a);
    wait_playing(&mut b);
    let seen = run(
        &mut [&mut a, &mut b],
        Duration::from_secs(10),
        |i, t| {
            if i == 0 {
                walk(t)
            } else {
                PlayerInput::default()
            }
        },
        |s| {
            let alice = s[0].player().pos;
            s[1].others()
                .iter()
                .any(|o| (o.pos - alice).length() < 0.5 && s[0].others().len() == 1)
                && (alice - s[1].player().pos).length() > 3.0
        },
    );
    assert!(
        seen,
        "Bob sees Alice walking (within 0.5 blocks) and Alice sees Bob"
    );
    a.close().expect("leave");
    b.close().expect("leave");
    server.shutdown();
    let _ = std::fs::remove_dir_all(dir);
}

/// Acceptance: ≤ 50 kbit/s per player with 20 moving players nearby in
/// steady state (measured on the wire: QUIC UDP payload, received).
#[test]
fn traffic_with_twenty_moving_players_nearby() {
    let (server, dir) = start("traffic", 6);
    let addr = server.local_addr();
    let mods = data();
    let b = mods.registry_builder().expect("blocks");
    let registry = Arc::new(b.build().expect("registry"));
    let bots: Vec<Remote> = (0..20)
        .map(|i| Remote::connect(addr, &format!("bot_{i:02}"), Arc::clone(&registry)).expect("bot"))
        .collect();
    let mut observer = session(addr, "Observer");
    wait_playing(&mut observer);

    // Bots walk in circles near the spawn; the observer stands and watches.
    let mut tick = 0u32;
    let bot_tick = |bots: &[Remote], tick: u32| {
        for (i, bot) in bots.iter().enumerate() {
            let yaw = f32::from((tick % 120) as u16) / 120.0 * std::f32::consts::TAU + i as f32;
            let _ = bot.poll().count();
            bot.send_input(InputDatagram {
                ack_snapshot: 0,
                intents: vec![Intent {
                    tick,
                    buttons: Buttons(Buttons::FORWARD),
                    yaw: Angle16::from_radians(yaw),
                    pitch: Angle16(0),
                    fly_speed_steps: 0,
                    state_hash: 0,
                }],
            });
        }
    };
    let measure = |secs: u64, observer: &mut Session, tick: &mut u32| {
        let start = Instant::now();
        let mut next = start;
        while start.elapsed() < Duration::from_secs(secs) {
            observer.update().expect("update");
            if Instant::now() >= next {
                observer.fixed_tick(&PlayerInput::default());
                bot_tick(&bots, *tick);
                *tick += 1;
                next += Duration::from_millis(50);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    // Warm up: columns finish streaming, everyone is known.
    measure(6, &mut observer, &mut tick);
    let before = observer.net_stats().expect("remote");
    measure(10, &mut observer, &mut tick);
    let after = observer.net_stats().expect("remote");
    let rx = (after.bytes.0 - before.bytes.0) as f64 * 8.0 / 10.0 / 1000.0;
    let tx = (after.bytes.1 - before.bytes.1) as f64 * 8.0 / 10.0 / 1000.0;
    println!(
        "observer with {} players in view: received {rx:.1} kbit/s, sent {tx:.1} kbit/s (UDP payload; +28 B/datagram of UDP/IP headers not counted)",
        after.others
    );
    assert_eq!(after.others, 20, "all bots in view");
    assert!(rx <= 50.0, "received {rx:.1} kbit/s");
    observer.close().expect("leave");
    for bot in bots {
        bot.leave();
    }
    server.shutdown();
    let _ = std::fs::remove_dir_all(dir);
}
