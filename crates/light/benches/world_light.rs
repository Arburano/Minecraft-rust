//! T1-09: lighting through the world path. Initial light of generated columns
//! (default bounds y -64..320, real generator, seed 7) and single edits
//! through `LightUpdates::process` (snapshot from the world + stored light +
//! frozen border + incremental relight + export of 27 sections), against the
//! ≤ 100 / 200 µs p95 per-edit budget.

use std::hint::black_box;
use std::sync::Arc;
use std::time::Instant;

use criterion::{Criterion, criterion_group, criterion_main};
use light::{LightUpdates, WorldLight, light_column, props_from_registry};
use world::coords::BlockPos;
use world::{BlockStateId, ColumnPos, WorldBounds, WorldMap};
use worldgen::{GenContext, Pipeline, WorldgenConfig};

fn bench_world_light(c: &mut Criterion) {
    let mods = world::modset::source_build(world::modset::DEFAULT_BUILD).expect("build");
    let b = mods.registry_builder().expect("blocks");
    let reg = Arc::new(b.build().expect("registry"));
    let props = props_from_registry(&reg);
    let src = mods.read("worldgen/overworld.toml").expect("config");
    let bounds = WorldBounds::default();
    let ctx = GenContext::new(
        7,
        WorldgenConfig::from_toml(&src, &reg).expect("config"),
        bounds,
    );
    let p = Pipeline::overworld();
    let mut map = WorldMap::new(Arc::clone(&reg), bounds);
    for x in -4..=4 {
        for z in -4..=4 {
            map.insert_column(
                p.generate(&ctx, ColumnPos { x, z })
                    .into_column(&reg)
                    .expect("bounds"),
            )
            .expect("bounds");
        }
    }
    // Initial lighting of the 7x7 columns, timed per column.
    let mut light = WorldLight::new();
    let mut times = Vec::new();
    let mut totals = (0, 0, 0);
    for x in -3..=3 {
        for z in -3..=3 {
            let t = Instant::now();
            let s = light_column(&map, &mut light, ColumnPos { x, z }, &props).expect("lit");
            times.push(t.elapsed().as_secs_f64() * 1e3);
            totals = (
                totals.0 + s.opaque,
                totals.1 + s.open_sky,
                totals.2 + s.computed,
            );
        }
    }
    times.sort_by(f64::total_cmp);
    eprintln!(
        "initial light: {} columns, p50 {:.1} ms, max {:.1} ms per column; sections opaque/sky/computed = {:?}",
        times.len(),
        times[times.len() / 2],
        times[times.len() - 1],
        totals
    );
    // Single edits through the world path: torch placed / removed at the surface.
    let torch = reg.state_id("core:torch").expect("torch");
    let stone = reg.state_id("core:stone").expect("stone");
    let spots: Vec<BlockPos> = (0..64)
        .map(|i| {
            let (x, z) = ((i * 7) % 48 - 16, (i * 13) % 48 - 16);
            BlockPos::new(x, map.height(x, z).expect("loaded"), z)
        })
        .collect();
    let mut updates = LightUpdates::new();
    let mut edit = |map: &mut WorldMap, light: &mut WorldLight, pos: BlockPos, id: BlockStateId| {
        let old = map.get_block(pos).expect("loaded");
        map.set_block(pos, id).expect("valid");
        updates.record(pos, props(old));
        let t = Instant::now();
        updates.process(map, light, &props, 1).expect("lit");
        t.elapsed()
    };
    for (name, id) in [("torch", torch), ("stone_on_surface", stone)] {
        let (mut add, mut remove) = (Vec::new(), Vec::new());
        for _ in 0..3 {
            for &pos in &spots {
                let before = map.get_block(pos).expect("loaded");
                add.push(edit(&mut map, &mut light, pos, id));
                remove.push(edit(&mut map, &mut light, pos, before));
            }
        }
        for (kind, mut v) in [("place", add), ("restore", remove)] {
            v.sort();
            let q = |f: f64| v[((v.len() - 1) as f64 * f) as usize].as_secs_f64() * 1e6;
            eprintln!(
                "world edit {name} {kind}: {} edits, p50 {:.0} us, p95 {:.0} us, max {:.0} us",
                v.len(),
                q(0.5),
                q(0.95),
                q(1.0)
            );
        }
    }
    // A roof placed over / removed from a 60-block shaft: the case that still
    // needs the downward direct-sky recompute (T1-16).
    let mut shafts = LightUpdates::new();
    let roofs: Vec<BlockPos> = (0..16)
        .map(|i| {
            let (x, z) = ((i * 11) % 40 - 12, (i * 17) % 40 - 12);
            let top = map.height(x, z).expect("loaded");
            for y in top - 60..top {
                let pos = BlockPos::new(x, y, z);
                let old = map.get_block(pos).expect("loaded");
                map.set_block(pos, BlockStateId::AIR).expect("valid");
                shafts.record(pos, props(old));
            }
            BlockPos::new(x, top, z)
        })
        .collect();
    while !shafts.is_empty() {
        shafts.process(&map, &mut light, &props, 8).expect("lit");
    }
    let (mut add, mut remove) = (Vec::new(), Vec::new());
    for _ in 0..3 {
        for &pos in &roofs {
            add.push(edit(&mut map, &mut light, pos, stone));
            remove.push(edit(&mut map, &mut light, pos, BlockStateId::AIR));
        }
    }
    for (kind, mut v) in [("place", add), ("remove", remove)] {
        v.sort();
        let q = |f: f64| v[((v.len() - 1) as f64 * f) as usize].as_secs_f64() * 1e6;
        eprintln!(
            "world edit roof_over_shaft {kind}: {} edits, p50 {:.0} us, p95 {:.0} us, max {:.0} us",
            v.len(),
            q(0.5),
            q(0.95),
            q(1.0)
        );
    }
    let mut g = c.benchmark_group("world_light");
    g.sample_size(10);
    g.bench_function("initial_light_one_column", |b| {
        b.iter(|| {
            black_box(
                light_column(&map, &mut light, ColumnPos { x: 0, z: 0 }, &props).expect("lit"),
            )
        })
    });
    g.finish();
}

criterion_group!(benches, bench_world_light);
criterion_main!(benches);
