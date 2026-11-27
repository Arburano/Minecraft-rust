//! T2-07: entity simulation LOD and asynchronous pathfinding.
//!
//! 500 test entities wander around a player on the real world; the player
//! then moves away so the entities pass every LOD band (decisions every 1st,
//! 2nd, 4th, 8th tick). Physics and path following run every tick, so their
//! positions stay continuous; the path-request budget per region and tick
//! is respected.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use glam::DVec3;
use server::entities::{MAX_PATH_REQUESTS_PER_TICK, MAX_PATHS_IN_FLIGHT, decision_interval};
use server::game::{World, WorldSetup};
use server::storage::Storage;
use sim::physics::PhysicsConfig;
use tokio::sync::mpsc::unbounded_channel;
use world::registry::blocks_from_dir;
use world::storage::WorldStore;
use world::{RegistryBuilder, WorldBounds};
use worldgen::{GenContext, WorldgenConfig};

fn data() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mods/core/data")
}

fn world(tag: &str, view: i32, sim: i32, region_threads: usize) -> (World, PathBuf) {
    let dir = std::env::temp_dir().join(format!("server-regions-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    let mut b = RegistryBuilder::new();
    b.add_mod(
        "core",
        blocks_from_dir(&data().join("blocks")).expect("blocks"),
    );
    let registry = Arc::new(b.build().expect("registry"));
    let gen_src = std::fs::read_to_string(data().join("worldgen/overworld.toml")).expect("gen");
    let ctx = Arc::new(GenContext::new(
        7,
        WorldgenConfig::from_toml(&gen_src, &registry).expect("gen"),
        WorldBounds::default(),
    ));
    let physics = PhysicsConfig::from_toml(
        &std::fs::read_to_string(data().join("player/physics.toml")).expect("physics"),
    )
    .expect("physics");
    let store = WorldStore::open(&dir, &registry, WorldBounds::default()).expect("store");
    let storage = Storage::start(store, Arc::clone(&registry)).expect("storage");
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
        },
        storage,
    )
    .expect("world");
    (w, dir)
}

#[test]
fn five_hundred_entities_path_and_never_freeze_across_lod_bands() {
    let (mut w, dir) = world("entities", 12, 10, 2);
    let (tx, _keep) = unbounded_channel();
    let mut player = DVec3::new(8.5, 120.0, 8.5);
    w.insert_player_for_test("watcher", player, tx);
    let ids = w.spawn_walkers(DVec3::new(8.5, 0.0, 8.5), 500, 40.0);
    let (_tx, inbox) = crossbeam_channel::unbounded();
    // Let the world load and the walkers land.
    let start = Instant::now();
    while w.entity_stats().live < 500 || start.elapsed() < Duration::from_secs(2) {
        assert!(start.elapsed() < Duration::from_secs(120), "world loads");
        w.tick(&inbox);
        std::thread::sleep(Duration::from_millis(2));
    }
    for _ in 0..40 {
        w.tick(&inbox);
        std::thread::sleep(Duration::from_millis(2));
    }
    let _ = w.entity_stats();
    let origin: HashMap<_, _> = ids
        .iter()
        .map(|&id| (id, w.walker_pos(id).expect("walker")))
        .collect();
    let mut last = origin.clone();
    let mut max_step: f64 = 0.0;
    let mut bands = [0usize; 4];
    let mut min_moving_far = usize::MAX;
    // 300 ticks near, then the player walks 6 blocks per tick away to 170 blocks.
    for t in 0..700 {
        if t >= 300 && player.x < 178.0 {
            player.x += 6.0;
            w.teleport_for_test("watcher", player);
        }
        w.tick(&inbox);
        std::thread::sleep(Duration::from_millis(2));
        let mut moving = 0;
        for &id in &ids {
            let Some(p) = w.walker_pos(id) else { continue };
            let prev = last[&id];
            let h = ((p.x - prev.x).powi(2) + (p.z - prev.z).powi(2)).sqrt();
            max_step = max_step.max(h);
            if h > 1e-6 {
                moving += 1;
            }
            let d = p.distance(player);
            bands[decision_interval(d).trailing_zeros() as usize] += 1;
            last.insert(id, p);
        }
        if player.x >= 178.0 {
            min_moving_far = min_moving_far.min(moving);
        }
    }
    let stats = w.entity_stats();
    let moved = ids
        .iter()
        .filter(|id| {
            let (a, b) = (origin[id], last[id]);
            ((a.x - b.x).powi(2) + (a.z - b.z).powi(2)).sqrt() > 3.0
        })
        .count();
    let mut lat = stats.latency_ticks.clone();
    lat.sort_unstable();
    println!(
        "live {}, dormant {}; walked > 3 blocks: {moved}/500; largest horizontal step per tick {max_step:.3} blocks; \
         entity-ticks per band (1/2/4/8): {bands:?}; fewest moving in a far tick: {min_moving_far}; \
         decisions {}, path requests {}, max per region per tick {}, latency p50 {} / max {} ticks",
        stats.live,
        stats.dormant,
        stats.decisions,
        stats.requested,
        stats.max_requests_per_region,
        lat.get(lat.len() / 2).copied().unwrap_or(0),
        lat.last().copied().unwrap_or(0),
    );
    // Entities that wandered out of the player's territory are stored, not
    // simulated (beyond the simulation distance); none is lost.
    assert_eq!(stats.live + stats.dormant, 500, "none lost");
    assert!(
        stats.live >= 400,
        "most are inside the territory: {}",
        stats.live
    );
    assert!(moved >= 250, "paths were found and followed: {moved}");
    assert!(
        max_step < 0.5,
        "continuous positions: {max_step} blocks in one tick"
    );
    assert!(
        bands.iter().all(|&b| b > 0),
        "every LOD band was crossed: {bands:?}"
    );
    assert!(
        min_moving_far > 50,
        "far entities still move every tick: {min_moving_far}"
    );
    assert!(stats.max_requests_per_region <= MAX_PATH_REQUESTS_PER_TICK);
    assert!(stats.in_flight <= MAX_PATHS_IN_FLIGHT);
    drop(w);
    let _ = std::fs::remove_dir_all(dir);
}
