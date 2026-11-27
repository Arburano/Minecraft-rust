//! T1-12 measurement: CPU cost of a block edit until it is ready for upload
//! — `set_block` + re-mesh of the affected mesh chunks, the relight of the
//! edit, and the render batch re-copy. Prints p50 / p95 / max per stage for
//! three edit kinds: break a surface block (changes direct sky), place a
//! block in the air, place a torch underground.
//!
//! `cargo run --release -p client --example edit_latency -- [edits per kind]`

use std::sync::Arc;
use std::time::{Duration, Instant};

use client::edit::{BatchSync, Editor};
use client::loading::{LoadConfig, Loader, Viewer};
use engine_core::jobs::JobPool;
use light::WorldLight;
use world::coords::BlockPos;
use world::{BlockStateId, ColumnPos, WorldBounds, WorldMap};
use worldgen::{GenContext, Pipeline, WorldgenConfig};

fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(100);
    let mods = world::modset::source_build(world::modset::DEFAULT_BUILD).expect("build");
    let b = mods.registry_builder().expect("blocks");
    let reg = Arc::new(b.build().expect("registry"));
    let src = mods.read("worldgen/overworld.toml").expect("config");
    let bounds = WorldBounds::default();
    let ctx = Arc::new(GenContext::new(
        7,
        WorldgenConfig::from_toml(&src, &reg).expect("config"),
        bounds,
    ));
    let radius = 4;
    let mut loader = Loader::new(
        LoadConfig {
            radius,
            ..LoadConfig::default()
        },
        Arc::clone(&reg),
        Arc::new(Pipeline::overworld()),
        ctx,
    );
    loader.track_mesh_changes();
    let mut map = WorldMap::new(Arc::clone(&reg), bounds);
    let mut light = WorldLight::new();
    light.track_changes();
    let pool = JobPool::new(4).expect("pool");
    let viewer = Viewer {
        pos: [8.0, 80.0, 8.0],
        velocity: [0.0; 3],
    };
    let ready = |loader: &Loader, map: &WorldMap| {
        (-radius..=radius)
            .all(|x| (-radius..=radius).all(|z| loader.is_ready(map, ColumnPos { x, z })))
    };
    while !ready(&loader, &map) {
        loader.tick(&viewer, &mut map, &mut light, &pool);
        std::thread::sleep(Duration::from_millis(2));
    }
    let mut sync = BatchSync::new();
    sync.collect(&mut loader);
    sync.upload(|_, _| {});
    let mut changed = Vec::new();
    light.take_changed(&mut changed);

    let mut editor = Editor::new(&map);
    let stone = reg.state_id("core:stone").expect("stone");
    let torch = reg.state_id("core:torch").expect("torch");
    let mut rng = 0x2545_f491_4f6c_dd1du64;
    let mut next = |m: i32| {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        (rng % m as u64) as i32 - m / 2
    };
    println!(
        "edits per kind: {n}; world radius {radius}, y -64..320; VM numbers are not the laptop's"
    );
    for kind in [
        "break surface block",
        "place block in air",
        "torch underground",
    ] {
        let (mut mesh_t, mut light_t, mut batch_t, mut total) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let mut sections = Vec::new();
        for _ in 0..n {
            let (x, z) = (next(48), next(48));
            let top = map.height(x, z).expect("loaded") - 1;
            let (pos, id) = match kind {
                "break surface block" => (BlockPos::new(x, top, z), BlockStateId::AIR),
                "place block in air" => (BlockPos::new(x, top + 4, z), stone),
                _ => (BlockPos::new(x, top - 12, z), torch),
            };
            let t0 = Instant::now();
            editor
                .set_block(&mut map, &mut loader, pos, id)
                .expect("edit");
            let t1 = Instant::now();
            let mut processed = 0;
            while editor.pending_light() > 0 {
                processed += editor.process_light(&map, &mut light, 1).expect("light");
            }
            let t2 = Instant::now();
            sync.collect(&mut loader);
            sync.upload(|_, _| {});
            changed.clear();
            light.take_changed(&mut changed);
            let t3 = Instant::now();
            sections.push(processed);
            mesh_t.push(t1 - t0);
            light_t.push(t2 - t1);
            batch_t.push(t3 - t2);
            total.push(t3 - t0);
        }
        let stat = |v: &mut Vec<Duration>| {
            v.sort();
            format!(
                "p50 {:>9.3?} p95 {:>9.3?} max {:>9.3?}",
                v[v.len() / 2],
                v[(v.len() * 95 / 100).min(v.len() - 1)],
                v[v.len() - 1]
            )
        };
        println!("{kind}:");
        println!("  set_block + remesh   {}", stat(&mut mesh_t));
        println!("  relight              {}", stat(&mut light_t));
        println!("  batch re-copy        {}", stat(&mut batch_t));
        println!("  total                {}", stat(&mut total));
    }
}
