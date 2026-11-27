//! T2-04: block edits over the network.
//!
//! Real `server::Server` in the process, game clients as `Session`s.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use auth::AdmissionPolicy;
use client::session::{Session, SessionConfig};
use light::{WorldLight, light_column, props_from_registry};
use server::{Server, ServerConfig};
use sim::physics::PlayerInput;
use world::coords::{BlockPos, SectionPos};
use world::{BlockStateId, ColumnPos};

fn data() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mods/core/data")
}

fn start(dir: &Path) -> Server {
    let mut config = ServerConfig::new(
        "127.0.0.1:0".parse().expect("addr"),
        AdmissionPolicy {
            allow_offline: true,
        },
        dir.to_owned(),
        data(),
    );
    config.seed = Some(7);
    config.view_distance = 10;
    config.threads = 2;
    Server::start(config).expect("server")
}

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("client-edits-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn session(addr: SocketAddr, name: &str) -> Session {
    let mut c = SessionConfig::new(data());
    c.radius = 8;
    c.threads = 2;
    c.light_sections_per_frame = 64;
    Session::connect(addr, name, c).expect("join")
}

/// Frames for all sessions until `done` or `limit`; true if done.
fn run(
    sessions: &mut [&mut Session],
    limit: Duration,
    mut done: impl FnMut(&[&mut Session]) -> bool,
) -> bool {
    let start = Instant::now();
    let mut next = start;
    while start.elapsed() < limit {
        for s in sessions.iter_mut() {
            s.update().expect("update");
        }
        if done(sessions) {
            return true;
        }
        if Instant::now() >= next {
            for s in sessions.iter_mut() {
                s.fixed_tick(&PlayerInput::default());
            }
            next += Duration::from_millis(50);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    false
}

fn playing(s: &mut Session) {
    assert!(run(&mut [s], Duration::from_secs(60), |s| s[0].is_playing()));
    // Settle on the ground.
    run(&mut [s], Duration::from_millis(500), |_| false);
}

/// An air cell near the player's head, in reach on both sides.
fn air_near(s: &Session, dx: i32, dy: i32) -> BlockPos {
    let p = s.player().pos;
    let base = BlockPos::new(
        p.x.floor() as i32 + dx,
        p.y.floor() as i32 + dy,
        p.z.floor() as i32,
    );
    for up in 0..4 {
        let c = BlockPos::new(base.x, base.y + up, base.z);
        if s.map().get_block(c) == Some(BlockStateId::AIR) {
            return c;
        }
    }
    panic!("no air near {base:?}");
}

fn assert_light_fresh(s: &Session, around: BlockPos) {
    let props = props_from_registry(s.registry());
    let col = ColumnPos::of_block(around);
    let mut fresh = WorldLight::new();
    light_column(s.map(), &mut fresh, col, &props).expect("light");
    let b = s.map().bounds();
    for i in 0..b.section_count as i32 {
        let sp = SectionPos {
            x: col.x,
            y: b.min_section_y + i,
            z: col.z,
        };
        assert_eq!(
            s.light_for_test().get(sp),
            fresh.get(sp),
            "light of {sp:?} equals a fresh lighting"
        );
    }
}

#[test]
fn edits_reach_other_clients_and_light_matches() {
    let dir = temp("two");
    let server = start(&dir);
    let mut a = session(server.local_addr(), "Alice");
    let mut b = session(server.local_addr(), "Bob_2");
    playing(&mut a);
    playing(&mut b);
    let torch = a.registry().state_id("core:torch").expect("torch");
    let stone = a.registry().state_id("core:stone").expect("stone");

    // Alice places a torch and a stone block; Bob sees both.
    let t = air_near(&a, 2, 1);
    let s = air_near(&a, -2, 1);
    let sent = Instant::now();
    assert!(a.edit_for_test(t, Some(torch)));
    assert!(a.edit_for_test(s, Some(stone)));
    assert!(
        run(&mut [&mut a, &mut b], Duration::from_secs(5), |x| {
            x[1].map().get_block(t) == Some(torch) && x[1].map().get_block(s) == Some(stone)
        }),
        "Bob sees Alice's edits"
    );
    let seen = sent.elapsed();
    println!("edit seen by the other client after {seen:?} (loopback)");
    assert!(
        seen < Duration::from_millis(500),
        "within a tick plus RTT: {seen:?}"
    );

    // Bob digs Alice's stone; Alice sees it.
    assert!(b.edit_for_test(s, None));
    assert!(run(&mut [&mut a, &mut b], Duration::from_secs(5), |x| {
        x[0].map().get_block(s) == Some(BlockStateId::AIR)
    }));

    // Everything acknowledged and relit: light equals a fresh lighting on both.
    assert!(run(&mut [&mut a, &mut b], Duration::from_secs(10), |x| {
        x.iter()
            .all(|s| s.pending_light() == 0 && s.net_stats().is_some_and(|n| n.pending_edits == 0))
    }));
    for c in [&a, &b] {
        assert_light_fresh(c, t);
        assert_light_fresh(c, s);
        assert_eq!(c.net_stats().expect("remote").rejected_edits, 0);
    }
    a.close().expect("leave");
    b.close().expect("leave");
    server.shutdown();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_refused_edit_rolls_back() {
    let dir = temp("refuse");
    let server = start(&dir);
    let mut a = session(server.local_addr(), "Alice");
    playing(&mut a);
    let stone = a.registry().state_id("core:stone").expect("stone");
    // Far out of reach: the server refuses, the prediction is undone.
    let far = air_near(&a, 12, 2);
    assert!(a.edit_for_test(far, Some(stone)));
    assert_eq!(a.map().get_block(far), Some(stone), "predicted at once");
    assert!(run(&mut [&mut a], Duration::from_secs(5), |x| {
        x[0].net_stats().is_some_and(|n| n.pending_edits == 0)
    }));
    assert_eq!(
        a.map().get_block(far),
        Some(BlockStateId::AIR),
        "rolled back"
    );
    assert_eq!(a.net_stats().expect("remote").rejected_edits, 1);
    a.close().expect("leave");
    server.shutdown();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn edits_survive_a_server_restart() {
    let dir = temp("restart");
    let server = start(&dir);
    let mut a = session(server.local_addr(), "Alice");
    playing(&mut a);
    let stone = a.registry().state_id("core:stone").expect("stone");
    let at = air_near(&a, 2, 2);
    assert!(a.edit_for_test(at, Some(stone)));
    assert!(run(&mut [&mut a], Duration::from_secs(5), |x| {
        x[0].net_stats().is_some_and(|n| n.pending_edits == 0)
    }));
    assert_eq!(a.map().get_block(at), Some(stone));
    a.close().expect("leave");
    server.shutdown();

    let server = start(&dir);
    let mut b = session(server.local_addr(), "Bob_2");
    assert!(run(&mut [&mut b], Duration::from_secs(60), |x| {
        x[0].map().get_block(at).is_some()
    }));
    assert_eq!(b.map().get_block(at), Some(stone), "the edit was saved");
    b.close().expect("leave");
    server.shutdown();
    let _ = std::fs::remove_dir_all(dir);
}

/// "Within one tick plus RTT" with a real round trip: both clients behind
/// relays with RTT 100 ms (no loss).
#[test]
fn edits_arrive_within_a_tick_plus_rtt() {
    let dir = temp("rtt");
    let server = start(&dir);
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("rt");
    let relay = |seed| {
        rt.block_on(net::lossy::LossyRelay::spawn(
            server.local_addr(),
            Duration::from_millis(50),
            0.0,
            seed,
        ))
        .expect("relay")
    };
    let (ra, rb) = (relay(1), relay(2));
    let mut a = session(ra.local_addr(), "Alice");
    let mut b = session(rb.local_addr(), "Bob_2");
    playing(&mut a);
    playing(&mut b);
    let stone = a.registry().state_id("core:stone").expect("stone");
    let mut worst = Duration::ZERO;
    for i in 0..5 {
        let at = air_near(&a, 2, i % 2);
        let sent = Instant::now();
        assert!(a.edit_for_test(at, Some(stone)));
        assert!(run(&mut [&mut a, &mut b], Duration::from_secs(5), |x| {
            x[1].map().get_block(at) == Some(stone)
        }));
        worst = worst.max(sent.elapsed());
        assert!(b.edit_for_test(at, None));
        assert!(run(&mut [&mut a, &mut b], Duration::from_secs(5), |x| {
            x[0].map().get_block(at) == Some(BlockStateId::AIR)
                && x.iter()
                    .all(|s| s.net_stats().is_some_and(|n| n.pending_edits == 0))
        }));
    }
    // One-way to the server (50 ms) + up to one tick (50 ms) + one-way to the
    // other client (50 ms) + frame and scheduling slack.
    println!("worst edit latency to the other client at RTT 100 ms: {worst:?}");
    assert!(worst < Duration::from_millis(300), "{worst:?}");
    a.close().expect("leave");
    b.close().expect("leave");
    rt.block_on(async {
        ra.abort();
        rb.abort();
    });
    drop(rt);
    server.shutdown();
    let _ = std::fs::remove_dir_all(dir);
}
