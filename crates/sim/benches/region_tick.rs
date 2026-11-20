//! T0-09 benchmarks: full tick for 100/500/2 000 bots on 1 and 8 regions,
//! and the entity transfer primitive (despawn + spawn by stable `NetId`
//! between two `World`s, the ≤ 5 µs hypothesis in performance-budgets).

use std::time::Instant;

use criterion::{Criterion, criterion_group, criterion_main};
use glam::Vec3;
use sim::fixture::{bot_intents, spawn_bots};
use sim::{Body, GROUND_Y, NetId, Region, RegionId, RegionNode, SectionRect, Tick};

fn tick_matrix(c: &mut Criterion) {
    let arena = SectionRect::new(0, 0, 63, 7).expect("valid");
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    for regions in [1usize, 8] {
        for bots in [100u64, 500, 2_000] {
            let threads = regions.min(cores);
            let mut node = RegionNode::new(arena, threads).expect("pool");
            let width = arena.width() / regions as i32;
            for i in 0..regions as i32 {
                node.add_region(
                    SectionRect::new(i * width, arena.min_z, (i + 1) * width - 1, arena.max_z)
                        .expect("valid"),
                )
                .expect("tiling");
            }
            let ids = spawn_bots(&mut node, bots).expect("spawn");
            let mut intents = Vec::new();
            let mut t = 0u64;
            c.bench_function(&format!("tick_{bots}_bots_{regions}_regions"), |b| {
                b.iter_custom(|iters| {
                    let start = Instant::now();
                    for _ in 0..iters {
                        bot_intents(&ids, Tick(t), &mut intents);
                        node.tick(&intents);
                        t += 1;
                    }
                    start.elapsed()
                });
            });
        }
    }
}

fn entity_transfer(c: &mut Criterion) {
    let mut a = Region::new(RegionId(1), SectionRect::new(0, 0, 7, 7).expect("valid"));
    let mut b = Region::new(RegionId(2), SectionRect::new(8, 0, 15, 7).expect("valid"));
    a.spawn(
        NetId(1),
        Body {
            position: Vec3::new(40.0, GROUND_Y, 40.0),
            velocity: Vec3::ZERO,
        },
    );
    let mut src = &mut a;
    let mut dst = &mut b;
    c.bench_function("entity_transfer_between_worlds", |ben| {
        ben.iter(|| {
            let body = src.take(NetId(1)).expect("resident");
            dst.spawn(NetId(1), body);
            std::mem::swap(&mut src, &mut dst);
        });
    });
}

criterion_group!(benches, tick_matrix, entity_transfer);
criterion_main!(benches);
