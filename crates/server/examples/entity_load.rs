//! T2-07 load measurement: test entities pathing around one player in one
//! region, on the real world thread, without the network.
//!
//! ```text
//! cargo run -p server --release --example entity_load -- [entities] [seconds] [player_distance]
//! cargo run -p server --release --example entity_load -- 500 30 0     # all near the player
//! cargo run -p server --release --example entity_load -- 500 30 120   # most in the 4-tick band
//! ```

use std::sync::Arc;
use std::time::{Duration, Instant};

use glam::DVec3;
use server::game::{TICK_RATE, World, WorldSetup};
use server::storage::Storage;
use sim::physics::PhysicsConfig;
use tokio::sync::mpsc::unbounded_channel;
use world::WorldBounds;
use world::storage::WorldStore;
use worldgen::{GenContext, WorldgenConfig};

fn percentile(v: &mut [Duration], p: f64) -> Duration {
    v.sort_unstable();
    v[((v.len() - 1) as f64 * p).round() as usize]
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |i: usize, d: usize| args.get(i).and_then(|a| a.parse().ok()).unwrap_or(d);
    let entities = arg(1, 500);
    let seconds = arg(2, 30) as u64;
    let distance = arg(3, 0) as f64;
    let cores = std::thread::available_parallelism().map_or(2, |n| n.get());
    let (view, sim) = (12, 10);

    let data = world::modset::source_build(world::modset::DEFAULT_BUILD).expect("build");
    let dir = std::env::temp_dir().join(format!("entity-load-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    let mods = world::modset::source_build(world::modset::DEFAULT_BUILD).expect("build");
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
    let mut w = World::new(
        WorldSetup {
            registry,
            context: ctx,
            physics,
            dir: dir.clone(),
            view_distance: view,
            sim_distance: sim,
            threads: cores.saturating_sub(1).max(1),
            region_threads: (cores * 4 / 5).max(1),
            entity_limits: server::entities::EntityLimits::from_mods(&data).expect("limits"),
            subsystems: Default::default(),
            dimension: (0, world::dimension::OVERWORLD.to_owned()),
        },
        storage,
    )
    .expect("world");
    let (tx, _keep) = unbounded_channel();
    w.insert_player_for_test("watcher", DVec3::new(8.5 + distance, 120.0, 8.5), tx);
    w.spawn_walkers(DVec3::new(8.5, 0.0, 8.5), entities, 40.0);
    let (_tx, inbox) = crossbeam_channel::unbounded();
    let period = Duration::from_secs(1) / u32::from(TICK_RATE);
    let warm = Instant::now();
    while w.entity_stats().live < entities * 9 / 10 || warm.elapsed() < Duration::from_secs(5) {
        if warm.elapsed() > Duration::from_secs(120) {
            break;
        }
        let t = Instant::now();
        w.tick(&inbox);
        if let Some(rest) = period.checked_sub(t.elapsed()) {
            std::thread::sleep(rest);
        }
    }
    let _ = w.entity_stats();
    let mut total = Vec::new();
    let mut region = Vec::new();
    let mut max_in_flight = 0;
    let mut latency = Vec::new();
    let (mut decisions, mut requested, mut done, mut max_req, mut simulated) = (0, 0, 0, 0, 0);
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(seconds) {
        let t = Instant::now();
        w.tick(&inbox);
        total.push(w.times().total);
        region.push(w.times().slowest_region);
        let s = w.entity_stats();
        max_in_flight = max_in_flight.max(s.in_flight);
        decisions += s.decisions;
        requested += s.requested;
        done += s.paths_done;
        simulated += s.simulated;
        max_req = max_req.max(s.max_requests_per_region);
        latency.extend(s.latency_ticks);
        if let Some(rest) = period.checked_sub(t.elapsed()) {
            std::thread::sleep(rest);
        }
    }
    let s = w.entity_stats();
    let ticks = total.len() as f64;
    println!("  stored over the region limit: {}", s.over_limit);
    println!(
        "{entities} entities ({} live, {} dormant), player {distance} blocks from their center, {} ticks, {cores} logical CPUs:",
        s.live,
        s.dormant,
        total.len()
    );
    for (name, v) in [("tick", &mut total), ("slowest region", &mut region)] {
        println!(
            "  {name:>14}: p50 {:.2} ms, p95 {:.2} ms, p99 {:.2} ms, max {:.2} ms",
            percentile(v, 0.5).as_secs_f64() * 1e3,
            percentile(v, 0.95).as_secs_f64() * 1e3,
            percentile(v, 0.99).as_secs_f64() * 1e3,
            percentile(v, 1.0).as_secs_f64() * 1e3,
        );
    }
    latency.sort_unstable();
    println!(
        "  per tick: {:.0} simulated, {:.1} decisions, {:.2} path requests (max {max_req} per region in one tick, budget {}), \
         {max_in_flight} max in flight (budget {}); {done} paths done; latency p50 {} / p99 {} / max {} ticks",
        simulated as f64 / ticks,
        decisions as f64 / ticks,
        requested as f64 / ticks,
        server::entities::MAX_PATH_REQUESTS_PER_TICK,
        server::entities::MAX_PATHS_IN_FLIGHT,
        latency.get(latency.len() / 2).copied().unwrap_or(0),
        latency.get(latency.len() * 99 / 100).copied().unwrap_or(0),
        latency.last().copied().unwrap_or(0),
    );
    drop(w);
    let _ = std::fs::remove_dir_all(dir);
}
