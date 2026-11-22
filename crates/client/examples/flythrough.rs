//! T1-10 measurement: load a world around a spawning viewer, then fly in a
//! straight line, ticking at 60 Hz in real time. Prints time until the nearest
//! 3×3 columns and the full radius are ready, main-thread apply/tick maxima,
//! cancellations, and how many ready columns lag behind while flying.
//!
//! `cargo run --release -p client --example flythrough -- [radius] [threads] [speed]`
//! (`TIMELINE=1` prints progress near the viewer; `NORMAL_PRIORITY=1` keeps
//! workers at normal OS priority for comparison.)

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use client::loading::{LoadConfig, Loader, Viewer};
use engine_core::jobs::JobPool;
use light::WorldLight;
use world::registry::blocks_from_dir;
use world::{ColumnPos, RegistryBuilder, WorldBounds, WorldMap};
use worldgen::{GenContext, Pipeline, WorldgenConfig};

fn main() {
    let mut args = std::env::args().skip(1);
    let radius: i32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(12);
    let threads: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(4);
    let speed: f64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(20.0);
    let data = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mods/core/data");
    let mut b = RegistryBuilder::new();
    b.add_mod(
        "core",
        blocks_from_dir(&data.join("blocks")).expect("blocks"),
    );
    let reg = Arc::new(b.build().expect("registry"));
    let src = std::fs::read_to_string(data.join("worldgen/overworld.toml")).expect("config");
    let bounds = WorldBounds::default();
    let ctx = Arc::new(GenContext::new(
        7,
        WorldgenConfig::from_toml(&src, &reg).expect("config"),
        bounds,
    ));
    let config = LoadConfig {
        radius,
        ..LoadConfig::default()
    };
    let mut loader = Loader::new(
        config,
        Arc::clone(&reg),
        Arc::new(Pipeline::overworld()),
        ctx,
    );
    let mut map = WorldMap::new(Arc::clone(&reg), bounds);
    let mut light = WorldLight::new();
    let low = std::env::var_os("NORMAL_PRIORITY").is_none();
    let pool = if low {
        JobPool::with_low_priority_workers(threads)
    } else {
        JobPool::new(threads)
    }
    .expect("pool");
    println!("workers: {threads}, low priority: {low}");
    let mut viewer = Viewer {
        pos: [8.0, 80.0, 8.0],
        velocity: [0.0; 3],
    };
    let frame = Duration::from_micros(16_667);
    let area = |c: ColumnPos, r: i32| {
        (-r..=r).flat_map(move |dx| {
            (-r..=r).map(move |dz| ColumnPos {
                x: c.x + dx,
                z: c.z + dz,
            })
        })
    };
    let start = Instant::now();
    let (mut near, mut full) = (None, None);
    while full.is_none() && start.elapsed() < Duration::from_secs(300) {
        let t = Instant::now();
        loader.tick(&viewer, &mut map, &mut light, &pool);
        if std::env::var_os("TIMELINE").is_some() && near.is_none() {
            let g = area(viewer.column(), 3)
                .filter(|&c| map.column(c).is_some())
                .count();
            let l = area(viewer.column(), 2)
                .filter(|&c| {
                    map.column(c)
                        .is_some_and(|col| col.state == world::ColumnState::Lit)
                })
                .count();
            let m = loader
                .meshes()
                .keys()
                .filter(|m| (-1..=0).contains(&m.x) && (-1..=0).contains(&m.z))
                .count();
            println!(
                "t={:?} gen(7x7)={g}/49 lit(5x5)={l}/25 near meshes={m}/48 inflight-ish submitted={}",
                start.elapsed(),
                loader.stats().submitted
            );
        }
        if near.is_none() && area(viewer.column(), 1).all(|c| loader.is_ready(&map, c)) {
            near = Some(start.elapsed());
        }
        if area(viewer.column(), radius).all(|c| loader.is_ready(&map, c)) {
            full = Some(start.elapsed());
        }
        std::thread::sleep(frame.saturating_sub(t.elapsed()));
    }
    let spawn = loader.stats();
    println!(
        "spawn: 3x3 ready {near:?}, radius {radius} ready {full:?}; max apply {:?}, max tick {:?} (unload {:?}, scan {:?}, one submit {:?}), jobs {}",
        spawn.max_apply,
        spawn.max_tick,
        spawn.max_unload,
        spawn.max_scan,
        spawn.max_submit,
        spawn.submitted
    );
    // Fly for 10 s.
    viewer.velocity = [speed, 0.0, 0.0];
    let fly = Instant::now();
    let mut worst_missing_near = 0;
    let mut last = Instant::now();
    while fly.elapsed() < Duration::from_secs(10) {
        let t = Instant::now();
        let dt = last.elapsed().as_secs_f64();
        last = Instant::now();
        viewer.pos[0] += speed * dt;
        loader.tick(&viewer, &mut map, &mut light, &pool);
        let missing = area(viewer.column(), 2)
            .filter(|&c| !loader.is_ready(&map, c))
            .count();
        worst_missing_near = worst_missing_near.max(missing);
        std::thread::sleep(frame.saturating_sub(t.elapsed()));
    }
    let s = loader.stats();
    println!(
        "flying {speed} blocks/s for 10 s: max apply {:?}, max tick {:?}, cancelled {}, worst not-ready columns within 2 of the viewer: {worst_missing_near}/25",
        s.max_apply, s.max_tick, s.cancelled
    );
}
