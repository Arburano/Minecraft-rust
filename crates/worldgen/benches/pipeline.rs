//! T1-03: generation speed (worldgen.md, "a column ≤ N ms per core").
//! One full column (16×384×16, all stages) on one thread, the 2D surface pass
//! alone, packing into palette sections, and 64 columns on a 4-thread pool —
//! for each landscape engine (`classic`: build `classic`; `climate`: build
//! `classic_plus`).

use std::hint::black_box;
use std::sync::Arc;

use criterion::{Criterion, criterion_group, criterion_main};
use engine_core::jobs::JobPool;
use world::{ColumnPos, WorldBounds};
use worldgen::{GenContext, Pipeline, WorldgenConfig};

fn bench_pipeline(c: &mut Criterion) {
    bench_build(c, "worldgen", "classic");
    bench_build(c, "worldgen_climate", "classic_plus");
}

fn bench_build(c: &mut Criterion, group: &str, build: &str) {
    let mods = world::modset::source_build(build).expect("build");
    let b = mods.registry_builder().expect("blocks");
    let reg = b.build().expect("registry");
    let src = mods.read("worldgen/overworld.toml").expect("config");
    let cfg = WorldgenConfig::from_toml(&src, &reg).expect("config");
    let ctx = Arc::new(GenContext::new(7, cfg, WorldBounds::default()));
    let pipeline = Arc::new(Pipeline::overworld());
    let mut g = c.benchmark_group(group);
    g.sample_size(20);
    let mut k = 0i32;
    g.bench_function("column_all_stages", |b| {
        b.iter(|| {
            k += 1;
            black_box(pipeline.generate(
                &ctx,
                ColumnPos {
                    x: k % 64,
                    z: k / 64,
                },
            ))
        })
    });
    g.bench_function("surface_pass_256", |b| {
        b.iter(|| {
            k += 1;
            for z in 0..16 {
                for x in 0..16 {
                    black_box(ctx.surface(k * 16 + x, z));
                }
            }
        })
    });
    g.bench_function("pack_into_column", |b| {
        b.iter_batched(
            || pipeline.generate(&ctx, ColumnPos::default()),
            |p| black_box(p.into_column(&reg)),
            criterion::BatchSize::LargeInput,
        )
    });
    let pool = JobPool::new(4).expect("pool");
    let positions: Vec<ColumnPos> = (0..8)
        .flat_map(|x| (0..8).map(move |z| ColumnPos { x, z }))
        .collect();
    g.bench_function("64_columns_4_threads", |b| {
        b.iter(|| black_box(pipeline.generate_parallel(&ctx, &pool, &positions)))
    });
    g.finish();
}

criterion_group!(benches, bench_pipeline);
criterion_main!(benches);
