//! End-to-end hook, typed-event and transactional block-batch benchmarks.

#[path = "../tests/support/mod.rs"]
mod support;

use std::hint::black_box;
use std::time::Duration;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use modhost::{BlockPos, Event, ModHost, Region};
use world::BlockStateId;

fn hooks(criterion: &mut Criterion) {
    let host = ModHost::new().expect("pooled engine");
    let prepared = host
        .load(support::hello_component().expect("hello component"))
        .expect("linked component");
    let region = Region::new(
        BlockPos { x: 0, y: 0, z: 0 },
        [
            ("hello:example".into(), BlockStateId(1)),
            ("hello:alternate".into(), BlockStateId(2)),
        ],
    );
    let mut worker = prepared
        .instantiate(region, Duration::from_secs(1))
        .expect("worker instance");
    worker.init(Duration::from_secs(1)).expect("guest init");
    let mut group = criterion.benchmark_group("modhost");
    group.bench_function("empty_run_system", |b| {
        b.iter(|| {
            worker
                .run_system(black_box(0), Duration::from_secs(1))
                .expect("empty hook");
        })
    });
    let events = vec![Event::Message(("hello:message".into(), vec![1, 2, 3, 4])); 1_000];
    group.throughput(Throughput::Elements(1_000));
    group.bench_function("on_events_1000_strings", |b| {
        b.iter(|| {
            worker
                .on_events(black_box(&events), Duration::from_secs(1))
                .expect("events hook");
        })
    });
    group.throughput(Throughput::Elements(4_000));
    let mut alternate = false;
    group.bench_function("set_blocks_4000", |b| {
        b.iter(|| {
            alternate = !alternate;
            worker
                .run_system(
                    black_box(if alternate { 1 } else { 5 }),
                    Duration::from_secs(1),
                )
                .expect("block hook");
        })
    });
    group.finish();

    // Budget row "Cold start of a mod with cache ≤ 50 ms" (T0-17). There is no
    // disk compilation cache yet, so this measures the upper bound: compile +
    // link from the .wasm file, instantiate and run `init`, per iteration.
    // The hello component is small; real mods will compile longer.
    let path = support::hello_component().expect("hello component");
    let mut cold = criterion.benchmark_group("modhost_cold_start");
    cold.sample_size(20);
    cold.bench_function("compile_link_instantiate_init_no_cache", |b| {
        b.iter(|| {
            let prepared = host.load(path).expect("linked component");
            let region = Region::new(
                BlockPos { x: 0, y: 0, z: 0 },
                [
                    ("hello:example".into(), BlockStateId(1)),
                    ("hello:alternate".into(), BlockStateId(2)),
                ],
            );
            let mut worker = prepared
                .instantiate(region, Duration::from_secs(1))
                .expect("worker instance");
            black_box(worker.init(Duration::from_secs(1)).expect("guest init"));
        })
    });
    cold.finish();
}

criterion_group!(benches, hooks);
criterion_main!(benches);
