//! T0-08 lighting benchmarks on the fixed T0-12 test corpus.
//!
//! Budget rows (docs/process/performance-budgets.md, "Lighting: add / remove
//! per edit", <= 100 / <= 200 us p95):
//! - add: placing a torch, breaking a block (`relight_add`)
//! - remove: removing a torch, placing a block (`relight_remove`)
//!
//! Plus a full recompute of a 3x3x3 snapshot (spec: 1.4-2.2 ms on the research VM).
//!
//! Corpus: `worldgen::test_corpus`, seed 0, snapshots centered on sections
//! (1..=4, 2, 1..=4) whose center section has both air and diggable cells: the center section spans world y 32..48 (surface, sea
//! level 44, trees, caves). Every section of a snapshot gets 8 torches in
//! random air cells, so edits interact with existing block light and sky.
//! Edit positions are random cells of the center section (fixed RNG).
//!
//! Worst cases (synthetic, `LightNeighborhood::from_props`): a torch placed and
//! removed in a completely dark, empty snapshot (the whole 14-level diamond
//! changes), and the roof of a 28x7x28 cave under open sky broken and replaced
//! (direct sky floods the cave).
//!
//! p50/p95/max are measured per edit with `Instant` before Criterion runs
//! (Criterion reports means); both are printed.

use std::hint::black_box;
use std::time::{Duration, Instant};

use criterion::{Criterion, criterion_group, criterion_main};
use light::{COLUMNS, EDGE, LightNeighborhood, LightProps};
use world::coords::{BlockPos, SectionPos};
use world::{BlockStateId, SECTION_VOLUME, Section};
use worldgen::test_corpus::{CorpusArea, ids, test_corpus};

const SEED: u64 = 0;
const TORCH_ID: u16 = 100;
const TORCH: LightProps = LightProps::new(14, 0);
const TORCHES_PER_SECTION: usize = 8;
const EDITS_PER_SNAPSHOT: usize = 64;

fn props(id: BlockStateId) -> LightProps {
    match id.0 {
        ids::AIR | ids::GLASS => LightProps::AIR,
        ids::WATER | ids::LEAVES => LightProps::new(0, 1),
        TORCH_ID => TORCH,
        _ => LightProps::OPAQUE,
    }
}

struct Rng(u64);
impl Rng {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n as u64) as usize
    }
}

struct Snapshot {
    light: LightNeighborhood,
    /// Air cells of the center section.
    air: Vec<BlockPos>,
    /// Opaque cells of the center section next to a lit cell (diggable).
    surface: Vec<BlockPos>,
}

fn build() -> Vec<Snapshot> {
    let area = CorpusArea {
        x: 0..6,
        y: 0..8,
        z: 0..6,
    };
    let columns = test_corpus(SEED, &area);
    let column = |sx: i32, sz: i32| &columns[sx as usize * 6 + sz as usize];
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut out = Vec::new();
    let mut ids_buf = [BlockStateId::AIR; SECTION_VOLUME];
    for cz in 1..=4 {
        for cx in 1..=4 {
            let center = SectionPos { x: cx, y: 2, z: cz };
            let mut sections: Vec<Section> = (0..27)
                .map(|i| {
                    let (dx, dz, dy) = (i % 3, i / 3 % 3, i / 9);
                    column(cx + dx - 1, cz + dz - 1).section(1 + dy).clone()
                })
                .collect();
            for s in &mut sections {
                s.unpack_into(&mut ids_buf);
                let mut placed = 0;
                for _ in 0..4096 {
                    if placed == TORCHES_PER_SECTION {
                        break;
                    }
                    let j = rng.below(SECTION_VOLUME);
                    if ids_buf[j] == BlockStateId::AIR {
                        s.set(j, BlockStateId(TORCH_ID));
                        ids_buf[j] = BlockStateId(TORCH_ID);
                        placed += 1;
                    }
                }
            }
            // Heightmap over the whole corpus column (y 0..128), torches excluded.
            let mut heightmap = vec![i32::MIN; COLUMNS];
            for lz in 0..EDGE {
                for lx in 0..EDGE {
                    let (wx, wz) = ((cx - 1) * 16 + lx as i32, (cz - 1) * 16 + lz as i32);
                    let col = column(wx.div_euclid(16), wz.div_euclid(16));
                    let (x, z) = (wx.rem_euclid(16) as usize, wz.rem_euclid(16) as usize);
                    'scan: for sy in (0..8).rev() {
                        let sec = col.section(sy);
                        for y in (0..16).rev() {
                            if props(sec.get((y * 16 + z) * 16 + x)).opacity > 0 {
                                heightmap[lz * EDGE + lx] = sy * 16 + y as i32 + 1;
                                break 'scan;
                            }
                        }
                    }
                }
            }
            let refs: [&Section; 27] = std::array::from_fn(|i| &sections[i]);
            let mut light =
                LightNeighborhood::new(center, refs, &heightmap, props).expect("27 sections");
            light.full_relight();
            let center_section = &sections[13];
            let pos = |j: usize| {
                BlockPos::new(
                    cx * 16 + (j % 16) as i32,
                    32 + (j / 256) as i32,
                    cz * 16 + (j / 16 % 16) as i32,
                )
            };
            let mut air = Vec::new();
            let mut surface = Vec::new();
            for _ in 0..20_000 {
                let j = rng.below(SECTION_VOLUME);
                let id = center_section.get(j);
                if id == BlockStateId::AIR && air.len() < EDITS_PER_SNAPSHOT {
                    air.push(pos(j));
                } else if props(id) == LightProps::OPAQUE && surface.len() < EDITS_PER_SNAPSHOT {
                    let p = pos(j);
                    let touches_air = [
                        (1, 0, 0),
                        (-1, 0, 0),
                        (0, 1, 0),
                        (0, -1, 0),
                        (0, 0, 1),
                        (0, 0, -1),
                    ]
                    .iter()
                    .any(|(dx, dy, dz)| {
                        light
                            .sky_light(BlockPos::new(p.x + dx, p.y + dy, p.z + dz))
                            .unwrap_or(0)
                            > 0
                            || light
                                .block_light(BlockPos::new(p.x + dx, p.y + dy, p.z + dz))
                                .unwrap_or(0)
                                > 0
                    });
                    if touches_air {
                        surface.push(p);
                    }
                }
            }
            // Fully submerged or solid center sections have nothing to edit.
            if air.is_empty() || surface.is_empty() {
                continue;
            }
            out.push(Snapshot {
                light,
                air,
                surface,
            });
        }
    }
    out
}

#[derive(Clone, Copy)]
enum Edit {
    AddTorch,
    RemoveTorch,
    BreakBlock,
    PlaceBlock,
}

/// Performs one edit, timing only the measured half; leaves the snapshot unchanged.
fn timed_edit(s: &mut Snapshot, edit: Edit, k: usize) -> Duration {
    let ok = "inside the snapshot";
    match edit {
        Edit::AddTorch => {
            let p = s.air[k % s.air.len()];
            let t = Instant::now();
            s.light.relight_add(p, TORCH).expect(ok);
            let d = t.elapsed();
            s.light.relight_remove(p, LightProps::AIR).expect(ok);
            d
        }
        Edit::RemoveTorch => {
            let p = s.air[k % s.air.len()];
            s.light.relight_add(p, TORCH).expect(ok);
            let t = Instant::now();
            s.light.relight_remove(p, LightProps::AIR).expect(ok);
            t.elapsed()
        }
        Edit::BreakBlock => {
            let p = s.surface[k % s.surface.len()];
            let t = Instant::now();
            s.light.relight_add(p, LightProps::AIR).expect(ok);
            let d = t.elapsed();
            s.light.relight_remove(p, LightProps::OPAQUE).expect(ok);
            d
        }
        Edit::PlaceBlock => {
            let p = s.air[k % s.air.len()];
            let t = Instant::now();
            s.light.relight_remove(p, LightProps::OPAQUE).expect(ok);
            let d = t.elapsed();
            s.light.relight_add(p, LightProps::AIR).expect(ok);
            d
        }
    }
}

const EDITS: [(&str, Edit); 4] = [
    ("add_torch", Edit::AddTorch),
    ("remove_torch", Edit::RemoveTorch),
    ("break_block", Edit::BreakBlock),
    ("place_block", Edit::PlaceBlock),
];

fn percentiles(snapshots: &mut [Snapshot]) {
    for (name, edit) in EDITS {
        let mut samples = Vec::new();
        for _round in 0..3 {
            for s in snapshots.iter_mut() {
                for k in 0..EDITS_PER_SNAPSHOT {
                    samples.push(timed_edit(s, edit, k));
                }
            }
        }
        samples.sort();
        let q = |f: f64| samples[((samples.len() - 1) as f64 * f) as usize].as_secs_f64() * 1e6;
        eprintln!(
            "light/{name}: {} edits  p50 {:.1} us  p95 {:.1} us  p99 {:.1} us  max {:.1} us",
            samples.len(),
            q(0.5),
            q(0.95),
            q(0.99),
            q(1.0)
        );
    }
    let mut full = Vec::new();
    for _ in 0..5 {
        for s in snapshots.iter_mut() {
            let t = Instant::now();
            s.light.full_relight();
            full.push(t.elapsed());
        }
    }
    full.sort();
    eprintln!(
        "light/full_relight_3x3x3: {} runs  p50 {:.0} us  max {:.0} us",
        full.len(),
        full[full.len() / 2].as_secs_f64() * 1e6,
        full[full.len() - 1].as_secs_f64() * 1e6
    );
}

/// Synthetic worst cases: (name, snapshot, position, "before" props, "after" props).
fn worst_cases() -> Vec<(
    &'static str,
    LightNeighborhood,
    BlockPos,
    LightProps,
    LightProps,
)> {
    let at = |x: usize, y: usize, z: usize| BlockPos::new(x as i32, y as i32, z as i32);
    let mut out = Vec::new();

    let dark = vec![LightProps::AIR; light::VOLUME];
    let mut n = LightNeighborhood::from_props(BlockPos::new(0, 0, 0), &dark, &vec![false; COLUMNS])
        .expect("valid snapshot");
    n.full_relight();
    out.push(("dark_torch", n, at(24, 24, 24), LightProps::AIR, TORCH));

    let mut cave = vec![LightProps::AIR; light::VOLUME];
    for y in 0..24 {
        for z in 0..EDGE {
            for x in 0..EDGE {
                let inside =
                    (16..23).contains(&y) && (10..38).contains(&x) && (10..38).contains(&z);
                if !inside {
                    cave[(y * EDGE + z) * EDGE + x] = LightProps::OPAQUE;
                }
            }
        }
    }
    let mut n = LightNeighborhood::from_props(BlockPos::new(0, 0, 0), &cave, &vec![true; COLUMNS])
        .expect("valid snapshot");
    n.full_relight();
    out.push((
        "cave_roof",
        n,
        at(24, 23, 24),
        LightProps::OPAQUE,
        LightProps::AIR,
    ));
    out
}

fn worst_case_percentiles() {
    for (name, mut n, p, before, after) in worst_cases() {
        let (mut brighten, mut darken) = (Vec::new(), Vec::new());
        for _ in 0..500 {
            let t = Instant::now();
            n.relight_add(p, after).expect("brightening change");
            brighten.push(t.elapsed());
            let t = Instant::now();
            n.relight_remove(p, before).expect("inside the snapshot");
            darken.push(t.elapsed());
        }
        for (kind, mut v) in [("add", brighten), ("remove", darken)] {
            v.sort();
            let q = |f: f64| v[((v.len() - 1) as f64 * f) as usize].as_secs_f64() * 1e6;
            eprintln!(
                "light/worst_{name}_{kind}: {} edits  p50 {:.1} us  p95 {:.1} us  max {:.1} us",
                v.len(),
                q(0.5),
                q(0.95),
                q(1.0)
            );
        }
    }
}

fn bench_light(c: &mut Criterion) {
    let mut snapshots = build();
    let (air, surface): (usize, usize) = snapshots
        .iter()
        .fold((0, 0), |(a, s), x| (a + x.air.len(), s + x.surface.len()));
    eprintln!(
        "corpus: {} snapshots, {air} air / {surface} surface edit cells",
        snapshots.len()
    );
    percentiles(&mut snapshots);
    worst_case_percentiles();

    let mut g = c.benchmark_group("light");
    for (name, edit) in EDITS {
        g.bench_function(name, |b| {
            let mut k = 0usize;
            b.iter_custom(|iters| {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    let n = snapshots.len();
                    let s = &mut snapshots[k % n];
                    total += timed_edit(s, edit, k / n);
                    k += 1;
                }
                total
            });
        });
    }
    for (name, mut n, p, before, after) in worst_cases() {
        g.bench_function(format!("worst_{name}_add"), |b| {
            b.iter_custom(|iters| {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    let t = Instant::now();
                    n.relight_add(p, after).expect("brightening change");
                    total += t.elapsed();
                    n.relight_remove(p, before).expect("inside the snapshot");
                }
                total
            });
        });
        g.bench_function(format!("worst_{name}_remove"), |b| {
            b.iter_custom(|iters| {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    n.relight_add(p, after).expect("brightening change");
                    let t = Instant::now();
                    n.relight_remove(p, before).expect("inside the snapshot");
                    total += t.elapsed();
                }
                total
            });
        });
    }
    g.bench_function("full_relight_3x3x3", |b| {
        let mut k = 0usize;
        b.iter(|| {
            let n = snapshots.len();
            let s = &mut snapshots[k % n];
            k += 1;
            s.light.full_relight();
            black_box(s.light.block_light(BlockPos::new(0, 0, 0)));
        });
    });
    g.finish();
}

criterion_group!(benches, bench_light);
criterion_main!(benches);
