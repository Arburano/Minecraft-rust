//! T2-06 load measurement: N simulated players in K clusters far apart,
//! walking in circles, on the real world thread (regions, physics, column
//! streaming, replication), without the network.
//!
//! ```text
//! cargo run -p server --release --example region_load -- [players] [clusters] [seconds] [region_threads]
//! cargo run -p server --release --example region_load -- 200 8 30      # the stage 2 exit shape
//! cargo run -p server --release --example region_load -- 400 1 30      # one region's capacity
//! ```
//!
//! Prints tick time percentiles (whole tick and the slowest region) after a
//! warm-up in which the columns around everybody are generated.

use std::sync::Arc;
use std::time::{Duration, Instant};

use glam::DVec3;
use protocol::{Angle16, Buttons, Intent};
use server::game::{TICK_RATE, ToConn, ToWorld, World, WorldSetup};
use server::storage::Storage;
use sim::physics::PhysicsConfig;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
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
    let players = arg(1, 200);
    let clusters = arg(2, 8).max(1);
    let seconds = arg(3, 30) as u64;
    let cores = std::thread::available_parallelism().map_or(2, |n| n.get());
    let region_threads = arg(4, (cores * 4 / 5).max(1));
    let (view, sim) = (10, 6);
    let dir = std::env::temp_dir().join(format!("region-load-{}", std::process::id()));
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
    let physics = PhysicsConfig::from_toml(&mods.read("player/physics.toml").expect("physics"))
        .expect("physics");
    let store = WorldStore::open(&dir, &registry, WorldBounds::default()).expect("store");
    let storage = Storage::start(store, Arc::clone(&registry)).expect("storage");
    let mut w = World::new(
        WorldSetup {
            registry,
            context: ctx,
            physics,
            dir: dir.clone(),
            view_distance: view,
            sim_distance: sim,
            threads: cores.saturating_sub(1).max(1),
            region_threads,
            entity_limits: Default::default(),
        },
        storage,
    )
    .expect("world");

    let mut outs: Vec<(String, UnboundedReceiver<ToConn>)> = Vec::new();
    for i in 0..players {
        let c = (i % clusters) as f64;
        // Clusters 1,200 blocks apart on a line; players within ±40 blocks.
        let jitter = |k: usize| ((i * 7919 + k * 104_729) % 80) as f64 - 40.0;
        let pos = DVec3::new(c * 1200.0 + jitter(1), 120.0, jitter(2));
        let (tx, rx) = unbounded_channel();
        let id = format!("bot{i:04}");
        w.insert_player_for_test(&id, pos, tx);
        outs.push((id, rx));
    }
    let (tx, inbox) = crossbeam_channel::unbounded();
    let period = Duration::from_secs(1) / u32::from(TICK_RATE);
    let mut tick = 0u32;
    type Rec = (Vec<Duration>, Vec<Duration>, Vec<Duration>, Vec<Duration>);
    let mut step = |w: &mut World, record: Option<&mut Rec>| {
        // Every bot walks a circle: forward with a slowly turning yaw.
        for (i, (id, _)) in outs.iter().enumerate() {
            let yaw = (tick as f32 / 60.0 + i as f32) % std::f32::consts::TAU;
            let _ = tx.send(ToWorld::Intents {
                id: id.clone(),
                intents: vec![Intent {
                    tick,
                    buttons: Buttons(Buttons::FORWARD),
                    yaw: Angle16::from_radians(yaw),
                    pitch: Angle16(0),
                    fly_speed_steps: 0,
                    state_hash: 0,
                }],
                ack_snapshot: 0,
            });
        }
        let start = Instant::now();
        w.tick(&inbox);
        if let Some(rec) = record {
            let t = w.times();
            rec.0.push(t.total);
            rec.1.push(t.slowest_region);
            rec.2.push(t.simulate);
            rec.3.push(t.pre + t.restructure + t.exchange);
        }
        // The "connections": acknowledge column streams at once.
        for (id, rx) in &mut outs {
            while let Ok(m) = rx.try_recv() {
                if let ToConn::Column { pos, .. } = m {
                    let _ = tx.send(ToWorld::Delivered {
                        id: id.clone(),
                        pos,
                        ok: true,
                    });
                }
            }
        }
        tick += 1;
        if let Some(rest) = period.checked_sub(start.elapsed()) {
            std::thread::sleep(rest);
        }
    };
    // Warm-up: columns around everybody generated and streamed.
    let warm = Instant::now();
    let mut last = 0;
    let mut stable = 0;
    while warm.elapsed() < Duration::from_secs(120) {
        step(&mut w, None);
        let (live, cached) = w.columns();
        if cached == last && live > 0 {
            stable += 1;
            if stable > 2 * u32::from(TICK_RATE) {
                break;
            }
        } else {
            stable = 0;
            last = cached;
        }
    }
    let (live, cached) = w.columns();
    println!(
        "warm-up {:.1} s: {live} live columns, {cached} cached; {} regions with players",
        warm.elapsed().as_secs_f64(),
        w.regions()
            .iter()
            .filter(|(_, ids)| !ids.is_empty())
            .count()
    );
    let mut rec: Rec = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let measure = Instant::now();
    while measure.elapsed() < Duration::from_secs(seconds) {
        step(&mut w, Some(&mut rec));
    }
    let (mut total, mut region, mut simulate, mut sequential) = rec;
    println!(
        "{players} players, {clusters} clusters, {region_threads} region threads ({cores} logical CPUs), {} ticks:",
        total.len()
    );
    for (name, v) in [
        ("tick", &mut total),
        ("simulate", &mut simulate),
        ("slowest region", &mut region),
        ("sequential", &mut sequential),
    ] {
        println!(
            "  {name:>14}: p50 {:.2} ms, p95 {:.2} ms, p99 {:.2} ms, max {:.2} ms",
            percentile(v, 0.5).as_secs_f64() * 1e3,
            percentile(v, 0.95).as_secs_f64() * 1e3,
            percentile(v, 0.99).as_secs_f64() * 1e3,
            percentile(v, 1.0).as_secs_f64() * 1e3,
        );
    }
    drop(w);
    let _ = std::fs::remove_dir_all(dir);
}
