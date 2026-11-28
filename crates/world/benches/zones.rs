//! T4-08: zone lookup cost per entity per tick.
//!
//! Each iteration resolves the rules at 1,000 random positions (one lookup
//! per entity), so a result divided by 1,000 is the cost of one entity's
//! lookup. Zones are random boxes of 16–256 blocks over an 8,192 × 8,192
//! area, plus two zones spanning the whole map (the "wide" list); positions
//! are uniform over the same area. `reference` scans every area without the
//! tile index (the comparison the optimization is checked against).

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use world::coords::BlockPos;
use world::zones::{Area, ZoneDef, ZoneIndex, ZoneRules};

const ENTITIES: usize = 1000;
const SPAN: i32 = 8192;

struct Rng(u64);
impl Rng {
    fn below(&mut self, n: i32) -> i32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n as u64) as i32
    }
}

fn zone(i: usize, x: [i32; 2], z: [i32; 2], priority: i32) -> ZoneDef {
    ZoneDef {
        id: format!("bench:z{i}"),
        name: String::new(),
        priority,
        difficulty: 0,
        areas: vec![Area { x, z, y: None }],
        members: Vec::new(),
        rules: ZoneRules {
            build: Some(i % 2 == 0),
            entry: (i % 3 == 0).then_some(true),
            pvp: Some(false),
        },
        spawns: None,
        replaces: false,
    }
}

fn index(n: usize, rng: &mut Rng) -> ZoneIndex {
    let mut defs = Vec::with_capacity(n + 2);
    for i in 0..n {
        let (x, z) = (rng.below(SPAN) - SPAN / 2, rng.below(SPAN) - SPAN / 2);
        let (w, d) = (16 + rng.below(240), 16 + rng.below(240));
        defs.push(zone(i, [x, x + w], [z, z + d], rng.below(8)));
    }
    if n > 0 {
        let world = [-SPAN * 1000, SPAN * 1000];
        defs.push(zone(n, world, world, -1));
        defs.push(zone(n + 1, world, world, -2));
    }
    ZoneIndex::build(defs).expect("index")
}

fn bench_zones(c: &mut Criterion) {
    let mut g = c.benchmark_group("zones_lookup");
    g.throughput(Throughput::Elements(ENTITIES as u64));
    for n in [0usize, 10, 100, 1000, 4000] {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let idx = index(n, &mut rng);
        let points: Vec<BlockPos> = (0..ENTITIES)
            .map(|_| BlockPos {
                x: rng.below(SPAN) - SPAN / 2,
                y: rng.below(320) - 64,
                z: rng.below(SPAN) - SPAN / 2,
            })
            .collect();
        g.bench_with_input(BenchmarkId::new("resolve", n), &idx, |b, idx| {
            b.iter(|| {
                for p in &points {
                    black_box(idx.resolve(black_box(*p)));
                }
            });
        });
        g.bench_with_input(BenchmarkId::new("may_build", n), &idx, |b, idx| {
            b.iter(|| {
                for p in &points {
                    black_box(idx.may_build(black_box(*p), "player_name"));
                }
            });
        });
        if n <= 1000 {
            g.bench_with_input(BenchmarkId::new("reference", n), &idx, |b, idx| {
                b.iter(|| {
                    for p in &points {
                        black_box(idx.resolve_reference(black_box(*p)));
                    }
                });
            });
        }
    }
    g.finish();
}

criterion_group!(benches, bench_zones);
criterion_main!(benches);
