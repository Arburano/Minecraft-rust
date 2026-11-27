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

/// Frames with a 20 TPS tick every third frame, until `done`. Ticks run at
/// the real rate: single-player runs on the embedded server (T2-08), which
/// applies a client's intents at 20 TPS.
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
        std::thread::sleep(Duration::from_micros(16_667));
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
    // Place against whatever is in reach: sweep the view around and down.
    for id in [dirt, glass] {
        'aim: for turn in 0..8 {
            s.look(0.8, 0.0);
            for pitch in [0.3, -0.2, -0.6, -1.0, -1.4] {
                s.look(0.0, pitch - s.view_dir().y.asin());
                if let Some(hit) = s.target() {
                    let at = hit.place_pos();
                    if s.place(id) {
                        changed.push(at);
                        let _ = turn;
                        break 'aim;
                    }
                }
            }
        }
    }
    assert!(changed.len() >= 3, "edits made: {changed:?}");
    let before: Vec<(BlockPos, Option<BlockStateId>)> =
        changed.iter().map(|&p| (p, s.map().get_block(p))).collect();
    let columns: Vec<ColumnPos> = changed.iter().map(|&p| ColumnPos::of_block(p)).collect();
    let sections_before: Vec<_> = columns
        .iter()
        .map(|&c| s.map().column(c).expect("loaded").sections().to_vec())
        .collect();
    // Single-player runs on the embedded server (T2-08): digging under the
    // feet is predicted at once and applied by the server a tick later, so
    // the server may correct the position. Stand still until it agrees.
    run(&mut s, PlayerInput::default(), |_, f| f >= 90);
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

/// Flying away unloads columns; their sections must be reported to the
/// renderer (else the GPU light atlas fills up and stale light shows as dark
/// patches far from spawn — owner report 2026-11-22).
#[test]
fn unloaded_columns_are_reported_for_the_renderer() {
    let root = std::env::temp_dir().join(format!("client-unload-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let dir = worlds::create(&root, "Away", 3).expect("create");
    let mut s = Session::open(&dir, config()).expect("open");
    run(&mut s, PlayerInput::default(), |s, _| s.is_playing());
    for _ in 0..10 {
        s.fixed_tick(&PlayerInput::default());
    }
    s.fixed_tick(&PlayerInput {
        jump: true,
        ..PlayerInput::default()
    });
    s.fixed_tick(&PlayerInput {
        toggle_fly: true,
        ..PlayerInput::default()
    });
    s.adjust_fly_speed(100);
    let start = s.player().pos;
    let mut removed = std::collections::HashSet::new();
    let fly = PlayerInput {
        forward: 1.0,
        jump: true,
        sprint: true,
        ..PlayerInput::default()
    };
    let t0 = Instant::now();
    while (s.player().pos - start).length() < 300.0 && t0.elapsed() < Duration::from_secs(90) {
        s.update().expect("update");
        s.fixed_tick(&fly);
        s.render_updates(
            |sp| {
                removed.insert(sp);
            },
            |_, _| {},
            |_, _| {},
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!((s.player().pos - start).length() >= 300.0, "flew away");
    assert!(!removed.is_empty(), "unloaded sections were reported");
    for sp in &removed {
        assert!(
            s.map().column(ColumnPos { x: sp.x, z: sp.z }).is_none(),
            "a reported section's column is not loaded: {sp:?}"
        );
    }
    s.close().expect("close");
    let _ = std::fs::remove_dir_all(root);
}

/// T2-08: a world saved by the stage 1 client (player in `world.toml`,
/// columns written straight by `WorldStore`, no `server.toml`, no player
/// file) opens through the embedded server where it was left.
#[test]
fn stage_1_saves_open_on_the_embedded_server() {
    use world::storage::{ColumnRecord, WorldStore};
    use world::{RegistryBuilder, Section, registry::blocks_from_dir};

    let root = std::env::temp_dir().join(format!("client-migrate-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let dir = worlds::create(&root, "Old world", 11).expect("create");
    let bounds = config().bounds;
    // A column at (3, 3) that is solid glass up to y = 40: not what the
    // generator makes, so seeing it proves the save was read.
    let data = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mods/core/data");
    let mut b = RegistryBuilder::new();
    b.add_mod(
        "core",
        blocks_from_dir(&data.join("blocks")).expect("blocks"),
    );
    let reg = b.build().expect("registry");
    let glass = reg.state_id("core:glass").expect("glass");
    {
        let mut store = WorldStore::open(&dir, &reg, bounds).expect("store");
        let sections = (0..bounds.section_count)
            .map(|i| {
                if i < 3 {
                    Section::uniform(glass)
                } else {
                    Section::new()
                }
            })
            .collect();
        store
            .save(&ColumnRecord {
                pos: ColumnPos { x: 3, z: 3 },
                bounds,
                sections,
                light: Vec::new(),
                entities: Vec::new(),
            })
            .expect("save");
    }
    let mut info = worlds::WorldInfo::load(&dir).expect("info");
    info.player.pos = Some([56.5, 48.0, 56.5]);
    info.player.flying = true;
    info.player.fly_speed = 2.0;
    info.save(&dir).expect("save info");
    assert!(!dir.join("server.toml").exists());

    let mut s = Session::open(&dir, config()).expect("open");
    run(&mut s, PlayerInput::default(), |s, _| s.is_playing());
    assert_eq!(
        s.player().pos,
        glam::DVec3::new(56.5, 48.0, 56.5),
        "the saved position"
    );
    assert!(s.is_flying());
    assert!((s.fly_speed() - 2.0).abs() < 1e-12);
    assert_eq!(
        s.map().get_block(BlockPos::new(50, 20, 50)),
        Some(glass),
        "the saved column"
    );
    s.close().expect("close");
    assert!(
        dir.join("server.toml").exists(),
        "the server keeps its seed now"
    );
    let text = std::fs::read_to_string(dir.join("server.toml")).expect("server.toml");
    assert!(text.contains("seed = 11"), "{text}");
    let _ = std::fs::remove_dir_all(root);
}
