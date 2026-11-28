//! T2-10: overload protection.
//!
//! - 50 players fly at full speed straight away from the spawn over a world
//!   that was never generated: generation cannot keep up, so they are held
//!   at the edge of the loaded terrain; none is ever inside an unloaded
//!   column or below the world, and the tick stays bounded.
//! - 10,000 entities in one region: only the per-region limit
//!   (`mods/core/data/entities/limits.toml`, merged over the build) is simulated, the rest stay
//!   stored, none is lost.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use glam::DVec3;
use protocol::{Angle16, Buttons, Intent};
use server::game::{ToWorld, World, WorldSetup};
use server::metrics::Percentiles;
use server::storage::Storage;
use sim::physics::PhysicsConfig;
use tokio::sync::mpsc::unbounded_channel;
use world::WorldBounds;
use world::storage::WorldStore;
use worldgen::{GenContext, WorldgenConfig};

fn data() -> world::ModSet {
    world::modset::source_build(world::modset::DEFAULT_BUILD).expect("build")
}

fn world(tag: &str, view: i32, sim: i32, region_threads: usize) -> (World, PathBuf) {
    let dir = std::env::temp_dir().join(format!("server-regions-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    let mods = data();
    let b = mods.registry_builder().expect("blocks");
    let registry = Arc::new(b.build().expect("registry"));
    let gen_src = mods.read("worldgen/overworld.toml").expect("gen");
    let ctx = Arc::new(GenContext::new(
        7,
        WorldgenConfig::from_toml(&gen_src, &registry).expect("gen"),
        WorldBounds::default(),
    ));
    let physics = PhysicsConfig::from_toml(&mods.read("movement/profile.toml").expect("physics"))
        .expect("physics");
    let store = WorldStore::open(&dir, &registry, WorldBounds::default()).expect("store");
    let storage = Storage::start(store, Arc::clone(&registry), 0).expect("storage");
    let w = World::new(
        WorldSetup {
            registry,
            context: ctx,
            physics,
            dir: dir.clone(),
            view_distance: view,
            sim_distance: sim,
            threads: 2,
            region_threads,
            entity_limits: server::entities::EntityLimits::from_mods(&data()).expect("limits"),
            subsystems: Default::default(),
            dimension: (0, world::dimension::OVERWORLD.to_owned()),
        },
        storage,
    )
    .expect("world");
    (w, dir)
}

#[test]
fn fifty_fast_flyers_are_held_at_the_edge_never_inside_unloaded_terrain() {
    let (mut w, dir) = world("flyers", 10, 6, 2);
    let mut keep = Vec::new();
    let n = 50;
    for i in 0..n {
        let (tx, rx) = unbounded_channel();
        keep.push(rx);
        w.insert_player_for_test(&format!("f{i:02}"), DVec3::new(8.5, 120.0, 8.5), tx);
    }
    let (tx, inbox) = crossbeam_channel::unbounded();
    let min_y = f64::from(WorldBounds::default().min_y());
    let period = Duration::from_millis(50);
    let mut times = Vec::new();
    let mut tick = 0u32;
    let mut farthest: f64 = 0.0;
    let mut violations = 0;
    // A player counts from the first tick it stands on loaded terrain (they
    // are placed before the spawn area exists).
    let mut landed = vec![false; n];
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(20) {
        let t = Instant::now();
        for i in 0..n {
            // Toggle flight, maximum fly speed, then sprint outwards.
            let buttons = if tick == 0 {
                Buttons::FLY_TOGGLE | Buttons::JUMP
            } else {
                Buttons::FORWARD | Buttons::SPRINT
            };
            let yaw = i as f32 / n as f32 * std::f32::consts::TAU;
            let _ = tx.send(ToWorld::Intents {
                id: format!("f{i:02}"),
                intents: vec![Intent {
                    tick,
                    buttons: Buttons(buttons),
                    yaw: Angle16::from_radians(yaw),
                    pitch: Angle16(0),
                    fly_speed_steps: if tick == 0 { 10 } else { 0 },
                    state_hash: 0,
                }],
                ack_snapshot: 0,
            });
        }
        tick += 1;
        w.tick(&inbox);
        times.push(w.times().total);
        for rx in &mut keep {
            while rx.try_recv().is_ok() {}
        }
        for (i, landed) in landed.iter_mut().enumerate() {
            let (loaded, pos) = w.player_on_loaded(&format!("f{i:02}")).expect("player");
            *landed |= loaded;
            if *landed && (!loaded || pos.y < min_y) {
                violations += 1;
            }
            farthest = farthest.max((pos.x * pos.x + pos.z * pos.z).sqrt());
        }
        if let Some(rest) = period.checked_sub(t.elapsed()) {
            std::thread::sleep(rest);
        }
    }
    let held = w.held_ticks();
    let p = Percentiles::of(&times);
    println!(
        "50 flyers for 20 s: farthest {farthest:.0} blocks; held {held} of {} player-ticks ({:.0}%); tick {}",
        n * times.len(),
        held as f64 * 100.0 / (n * times.len()) as f64,
        p.line()
    );
    assert_eq!(
        violations, 0,
        "a player was inside unloaded terrain or below the world"
    );
    assert!(farthest > 100.0, "they flew: {farthest}");
    assert!(landed.iter().all(|&l| l), "everyone reached loaded terrain");
    assert!(held > 0, "generation could not keep up, so they were held");
    drop(w);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ten_thousand_entities_keep_to_the_region_limit() {
    let (mut w, dir) = world("limit", 6, 4, 2);
    let (tx, _keep) = unbounded_channel();
    w.insert_player_for_test("watcher", DVec3::new(8.5, 120.0, 8.5), tx);
    let ids = w.spawn_walkers(DVec3::new(8.5, 0.0, 8.5), 10_000, 30.0);
    let (_tx, inbox) = crossbeam_channel::unbounded();
    let limit = server::entities::EntityLimits::from_mods(&data())
        .expect("limits")
        .of(server::entities::WALKER_NAME);
    let mut times = Vec::new();
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        w.tick(&inbox);
        times.push(w.times().total);
        std::thread::sleep(Duration::from_millis(5));
    }
    let s = w.entity_stats();
    println!(
        "10,000 entities: {} simulated (limit {limit}), {} stored, {} over the limit; tick {}",
        s.live,
        s.dormant,
        s.over_limit,
        Percentiles::of(&times[times.len() / 2..]).line()
    );
    assert_eq!(ids.len(), 10_000);
    assert_eq!(s.live, limit, "the region simulates exactly its limit");
    assert_eq!(s.live + s.dormant, 10_000, "none lost");
    assert!(s.over_limit > 0);
    drop(w);
    let _ = std::fs::remove_dir_all(dir);
}
