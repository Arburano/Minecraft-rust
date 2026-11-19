//! Palette section access benchmarks (`world::Section`).
//!
//! Budget rows in `docs/process/performance-budgets.md`:
//! - "`set` in a section, any bit width" (target ≤ 10 ns).
//! - `get` ≈ 3.8 ns and `unpack_into` ≤ 5 µs per section (world-storage.md).
//!
//! `get`/`set` run over a random index order; `set` writes states already in the
//! palette so no growth or allocation happens on the measured hot path.
//! `unpack_into` measures a full word-by-word unpack of one section.
//!
//! The `section_corpus` group (T0-12) runs the same operations on real terrain
//! sections from `worldgen::test_corpus` (fixed seed 0, column (0, 0)):
//! deep underground, surface and tree canopy. The width-sweep groups above stay
//! synthetic so every bit width is covered.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use world::{BlockStateId, Section};
use worldgen::test_corpus::test_column;

const VOLUME: usize = 4096;

/// Deterministic xorshift for a fixed random index order.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

fn random_order() -> Vec<usize> {
    let mut r = Rng(0x1234_5678_9abc_def1);
    (0..VOLUME)
        .map(|_| (r.next() % VOLUME as u64) as usize)
        .collect()
}

/// A section whose palette has exactly `distinct` states (capped at 4096).
fn section_with(distinct: usize) -> (Section, Vec<BlockStateId>) {
    let mut arr = [BlockStateId::AIR; VOLUME];
    for (i, v) in arr.iter_mut().enumerate() {
        *v = BlockStateId((i % distinct) as u16);
    }
    let base = arr.to_vec();
    (Section::from_blocks(&arr), base)
}

fn bench_get(c: &mut Criterion) {
    let order = random_order();
    let mut g = c.benchmark_group("section_get");
    for (name, distinct) in [
        ("single", 1),
        ("1bit", 2),
        ("2bit", 4),
        ("3bit", 8),
        ("4bit", 16),
        ("5bit", 32),
        ("6bit", 64),
        ("7bit", 128),
        ("8bit", 256),
        ("direct", 4096),
    ] {
        let (section, _) = section_with(distinct);
        g.bench_with_input(BenchmarkId::new("random", name), &order, |b, order| {
            b.iter(|| {
                let mut acc = 0u64;
                for &i in order {
                    acc += black_box(&section).get(i).0 as u64;
                }
                acc
            });
        });
    }
    g.finish();
}

fn bench_set(c: &mut Criterion) {
    let mut g = c.benchmark_group("section_set");
    for (name, distinct) in [
        ("single", 1),
        ("1bit", 2),
        ("2bit", 4),
        ("3bit", 8),
        ("4bit", 16),
        ("5bit", 32),
        ("6bit", 64),
        ("7bit", 128),
        ("8bit", 256),
        ("direct", 4096),
    ] {
        let (section, _) = section_with(distinct);
        // Indices and values are generated inline so no large auxiliary vector
        // shares cache lines with the section data (which would add 4K-aliasing
        // stalls unrelated to the section itself). Values stay within the palette
        // so the measured hot path never grows or allocates.
        g.bench_with_input(
            BenchmarkId::new("existing", name),
            &distinct,
            |b, &distinct| {
                let mut section = section.clone();
                b.iter(|| {
                    let mut r = Rng(0x1234_5678_9abc_def1);
                    for _ in 0..VOLUME {
                        let i = (r.next() % VOLUME as u64) as usize;
                        let v = BlockStateId((r.next() % distinct as u64) as u16);
                        black_box(&mut section).set(i, v);
                    }
                });
            },
        );
    }
    g.finish();
}

fn bench_unpack(c: &mut Criterion) {
    let mut g = c.benchmark_group("section_unpack_into");
    for (name, distinct) in [
        ("single", 1),
        ("3bit", 8),
        ("4bit", 16),
        ("8bit", 256),
        ("direct", 4096),
    ] {
        let (section, _) = section_with(distinct);
        g.bench_with_input(BenchmarkId::new("section", name), &(), |b, _| {
            let mut out = [BlockStateId::AIR; VOLUME];
            b.iter(|| {
                black_box(&section).unpack_into(black_box(&mut out));
            });
        });
    }
    g.finish();
}

/// T0-12: get/set/unpack on terrain sections of the fixed test corpus.
fn bench_corpus(c: &mut Criterion) {
    let column = test_column(0, 0, 0, -1..5);
    let order = random_order();
    let mut g = c.benchmark_group("section_corpus");
    for (name, sy) in [("deep", 0), ("surface", 2), ("canopy", 3)] {
        let section = column.section(sy).clone();
        // The section's own states, so `set` stays on the no-growth hot path.
        let mut states = [BlockStateId::AIR; VOLUME];
        section.unpack_into(&mut states);
        let mut states = states.to_vec();
        states.sort_unstable();
        states.dedup();

        g.bench_with_input(BenchmarkId::new("get", name), &order, |b, order| {
            b.iter(|| {
                let mut acc = 0u64;
                for &i in order {
                    acc += black_box(&section).get(i).0 as u64;
                }
                acc
            });
        });
        g.bench_with_input(BenchmarkId::new("set", name), &states, |b, states| {
            let mut section = section.clone();
            b.iter(|| {
                let mut r = Rng(0x1234_5678_9abc_def1);
                for _ in 0..VOLUME {
                    let i = (r.next() % VOLUME as u64) as usize;
                    let v = states[(r.next() % states.len() as u64) as usize];
                    black_box(&mut section).set(i, v);
                }
            });
        });
        g.bench_with_input(BenchmarkId::new("unpack", name), &(), |b, _| {
            let mut out = [BlockStateId::AIR; VOLUME];
            b.iter(|| {
                black_box(&section).unpack_into(black_box(&mut out));
            });
        });
    }
    g.finish();
}

criterion_group!(benches, bench_get, bench_set, bench_unpack, bench_corpus);
criterion_main!(benches);
