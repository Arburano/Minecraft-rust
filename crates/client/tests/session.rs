//! T1-13 acceptance: create → play → quit → load returns the same world.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use client::session::{Session, SessionConfig};
use client::worlds;
use sim::physics::PlayerInput;
use world::coords::BlockPos;
use world::{BlockStateId, ColumnPos, WorldBounds};

fn config() -> SessionConfig {
    let data = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mods/core/data");
    SessionConfig {
        radius: 2,
        threads: 4,
        bounds: WorldBounds {
            min_section_y: 0,
            section_count: 8,
        },
        autosave: Duration::from_secs(3600),
        ..SessionConfig::new(data)
    }
}

/// Frames at 60 Hz with a 20 TPS tick every third frame, until `done`.
fn run(s: &mut Session, input: PlayerInput, mut done: impl FnMut(&Session, u32) -> bool) {
    let start = Instant::now();
    let mut frame = 0u32;
    while !done(s, frame) {
        s.update().expect("update");
        if frame % 3 == 0 {
            s.fixed_tick(&input);
        }
        frame += 1;
        assert!(start.elapsed() < Duration::from_secs(120), "timed out");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn create_play_quit_load_returns_the_same_world() {
    let root = std::env::temp_dir().join(format!("client-session-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let dir = worlds::create(&root, "Test world", 5).expect("create");

    let mut s = Session::open(&dir, config()).expect("open");
    run(&mut s, PlayerInput::default(), |s, _| s.is_playing());
    let dirt = s.registry().state_id("core:dirt").expect("dirt");
    let glass = s.registry().state_id("core:glass").expect("glass");

    // Look down, dig two blocks, walk, then build.
    s.look(0.0, -1.5);
    let mut changed: Vec<BlockPos> = Vec::new();
    for _ in 0..2 {
        let hit = s.target().expect("looking at the ground");
        assert!(s.dig());
        changed.push(hit.block);
        run(&mut s, PlayerInput::default(), |_, f| f >= 30); // fall into the hole
    }
    let walk = PlayerInput {
        forward: 1.0,
        jump: true,
        ..PlayerInput::default()
    };
    run(&mut s, walk, |_, f| f >= 120);
    s.look(0.0, 1.0); // forward and a little down
    for id in [dirt, glass] {
        if let Some(hit) = s.target() {
            let at = hit.place_pos();
            if s.place(id) {
                changed.push(at);
            }
        }
        s.look(0.0, -0.4);
    }
    assert!(changed.len() >= 3, "edits made: {changed:?}");
    let before: Vec<(BlockPos, Option<BlockStateId>)> =
        changed.iter().map(|&p| (p, s.map().get_block(p))).collect();
    let columns: Vec<ColumnPos> = changed.iter().map(|&p| ColumnPos::of_block(p)).collect();
    let sections_before: Vec<_> = columns
        .iter()
        .map(|&c| s.map().column(c).expect("loaded").sections().to_vec())
        .collect();
    let player = s.player().pos;
    s.close().expect("save and close");

    let mut s = Session::open(&dir, config()).expect("reopen");
    run(&mut s, PlayerInput::default(), |s, _| s.is_playing());
    assert_eq!(s.player().pos, player, "player position restored");
    for (p, id) in &before {
        assert_eq!(s.map().get_block(*p), *id, "block at {p:?}");
    }
    for (c, sections) in columns.iter().zip(&sections_before) {
        let now = s.map().column(*c).expect("loaded").sections();
        for (a, b) in now.iter().zip(sections) {
            assert!((0..4096).all(|i| a.get(i) == b.get(i)), "column {c:?}");
        }
    }
    assert_eq!(s.info().name, "Test world");
    s.close().expect("close");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn flight_and_fly_speed_are_saved() {
    let root = std::env::temp_dir().join(format!("client-fly-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let dir = worlds::create(&root, "Fly", 9).expect("create");
    let mut s = Session::open(&dir, config()).expect("open");
    run(&mut s, PlayerInput::default(), |s, _| s.is_playing());
    for _ in 0..10 {
        s.fixed_tick(&PlayerInput::default()); // stand on the ground
    }
    // Double tap: a jump, then the toggle while rising.
    s.fixed_tick(&PlayerInput {
        jump: true,
        ..PlayerInput::default()
    });
    s.fixed_tick(&PlayerInput::default());
    s.fixed_tick(&PlayerInput {
        toggle_fly: true,
        ..PlayerInput::default()
    });
    assert!(s.is_flying());
    s.adjust_fly_speed(3);
    assert!((s.fly_speed() - 1.25f64.powi(3)).abs() < 1e-12);
    s.adjust_fly_speed(100);
    assert_eq!(s.fly_speed(), 8.0, "clamped to the profile's range");
    let up = PlayerInput {
        jump: true,
        ..PlayerInput::default()
    };
    for _ in 0..20 {
        s.fixed_tick(&up);
    }
    let pos = s.player().pos;
    s.close().expect("close");
    let mut s = Session::open(&dir, config()).expect("reopen");
    run(&mut s, PlayerInput::default(), |s, _| s.is_playing());
    assert!(s.is_flying(), "still flying after loading");
    assert_eq!(s.fly_speed(), 8.0);
    assert_eq!(s.player().pos, pos);
    s.close().expect("close");
    let _ = std::fs::remove_dir_all(root);
}
