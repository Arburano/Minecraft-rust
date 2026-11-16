//! T0-04 benchmarks on a fixed synthetic terrain corpus (seeded Perlin
//! heightmap + caves + water + trees, ~8% transparent blocks).
//!
//! Budget rows (docs/process/performance-budgets.md):
//! - gather + masks of a 32^3 mesh chunk: <= 100 us p50
//! - fast_mesh of a 32^3 mesh chunk: <= 0.2 ms p50
//! - mask update per edit: <= 20 ns
//!
//! Corpus: 4x2x4 mesh chunks (blocks x,z 0..128, y 0..64), sections cut from
//! one global field so neighboring chunks share consistent padding.

use binary_greedy_meshing::Mesher;
use criterion::{Criterion, criterion_group, criterion_main};
use mesh::{CS, MaskCache, MeshChunkBuilder, MeshChunkQuads, P2, P3, SectionNeighborhood};
use noise::{NoiseFn, Perlin};
use std::hint::black_box;
use world::coords::MeshChunkPos;
use world::{BlockStateId, SECTION_VOLUME, Section};

/// One pre-gathered chunk: voxels + opaque + transparent masks.
type Gathered = (Box<[u16; P3]>, Box<[u64; P2]>, Box<[u64; P2]>);

const GX: i32 = 4;
const GY: i32 = 2;
const GZ: i32 = 4;
// Field extent in blocks (one section of margin around the chunk grid).
const FX: i32 = 160; // x: -16..144
const FY: i32 = 96; // y: -16..80
const FZ: i32 = 160; // z: -16..144
const SEA: i32 = 44;

fn is_transparent(b: BlockStateId) -> bool {
    matches!(b.0, 5..=7)
}

fn field_index(x: i32, y: i32, z: i32) -> usize {
    (((y + 16) * FX + (x + 16)) * FZ + (z + 16)) as usize
}

fn section_index(sx: i32, sy: i32, sz: i32) -> usize {
    (((sy + 1) * 10 + (sz + 1)) * 10 + (sx + 1)) as usize
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// ids: 0 air, 1 stone, 2 dirt, 3 grass, 4 sand, 5 water(T), 6 leaves(T), 7 glass(T)
fn build_corpus() -> (Vec<Section>, Vec<SectionNeighborhood>) {
    let height_noise = Perlin::new(1);
    let cave_noise = Perlin::new(2);
    let mut field = vec![0u16; (FX * FY * FZ) as usize];
    let mut height = vec![0i32; (FX * FZ) as usize];
    for x in 0..FX {
        for z in 0..FZ {
            let (wx, wz) = ((x - 16) as f64, (z - 16) as f64);
            let (mut a, mut f, mut s) = (1.0, 1.0 / 128.0, 0.0);
            for _ in 0..4 {
                s += a * height_noise.get([wx * f, wz * f]);
                a *= 0.5;
                f *= 2.0;
            }
            height[(x * FZ + z) as usize] = (36.0 + s * 16.0) as i32;
        }
    }
    for y in -16..80i32 {
        for x in -16..144i32 {
            for z in -16..144i32 {
                let h = height[((x + 16) * FZ + (z + 16)) as usize];
                let mut b = 0u16;
                if y <= h {
                    b = if y == h {
                        if h <= SEA + 1 { 4 } else { 3 }
                    } else if y >= h - 3 {
                        2
                    } else {
                        1
                    };
                    if y < h - 2
                        && cave_noise.get([
                            f64::from(x) / 24.0,
                            f64::from(y) / 16.0,
                            f64::from(z) / 24.0,
                        ]) > 0.55
                    {
                        b = 0;
                    }
                } else if y <= SEA {
                    b = 5;
                }
                field[field_index(x, y, z)] = b;
            }
        }
    }
    // Trees: deterministic per 32x32 surface column, trunks + leaf blobs.
    for cx in 0..GX {
        for cz in 0..GZ {
            let mut r = Rng((u64::from(cx.unsigned_abs()) * 73856093)
                ^ (u64::from(cz.unsigned_abs()) * 19349663)
                | 1);
            for _ in 0..24 {
                let x = cx * 32 + 3 + (r.next() % 26) as i32;
                let z = cz * 32 + 3 + (r.next() % 26) as i32;
                let h = height[((x + 16) * FZ + (z + 16)) as usize];
                if h <= SEA + 1 {
                    continue;
                }
                for dy in 1..=5 {
                    let i = field_index(x, h + dy, z);
                    if field[i] == 0 {
                        field[i] = 2;
                    }
                }
                for dy in 4..=7i32 {
                    for dx in -2..=2i32 {
                        for dz in -2..=2i32 {
                            if dx.abs() + dz.abs() > 3 {
                                continue;
                            }
                            let i = field_index(x + dx, h + dy, z + dz);
                            if field[i] == 0 {
                                field[i] = 6;
                            }
                        }
                    }
                }
            }
        }
    }
    // Composition over the mesh-chunk grid.
    let (mut air, mut opaque, mut trans) = (0usize, 0usize, 0usize);
    for y in 0..64i32 {
        for x in 0..128i32 {
            for z in 0..128i32 {
                let b = field[field_index(x, y, z)];
                if b == 0 {
                    air += 1;
                } else if is_transparent(BlockStateId(b)) {
                    trans += 1;
                } else {
                    opaque += 1;
                }
            }
        }
    }
    let total = (air + opaque + trans) as f64;
    eprintln!(
        "corpus: air {:.1}%  opaque {:.1}%  transparent {:.1}%",
        air as f64 / total * 100.0,
        opaque as f64 / total * 100.0,
        trans as f64 / total * 100.0
    );
    // Cut sections (x,z: -1..=8, y: -1..=4).
    let mut sections = Vec::with_capacity(600);
    for sy in -1..=4i32 {
        for sz in -1..=8i32 {
            for sx in -1..=8i32 {
                let mut arr = [BlockStateId::AIR; SECTION_VOLUME];
                for ly in 0..16i32 {
                    for lz in 0..16i32 {
                        for lx in 0..16i32 {
                            let i = field_index(sx * 16 + lx, sy * 16 + ly, sz * 16 + lz);
                            arr[((ly * 16 + lz) * 16 + lx) as usize] = BlockStateId(field[i]);
                        }
                    }
                }
                sections.push(Section::from_blocks(&arr));
            }
        }
    }
    let mut neighborhoods = Vec::new();
    for cy in 0..GY {
        for cz in 0..GZ {
            for cx in 0..GX {
                let chunk = MeshChunkPos {
                    x: cx,
                    y: cy,
                    z: cz,
                };
                neighborhoods.push(SectionNeighborhood::from_fn(chunk, |sp| {
                    sections[section_index(sp.x, sp.y, sp.z)].clone()
                }));
            }
        }
    }
    (sections, neighborhoods)
}

fn bench_mesh(c: &mut Criterion) {
    let (_sections, neighborhoods) = build_corpus();
    let n = neighborhoods.len();
    let mut builder = MeshChunkBuilder::new();

    let mut g = c.benchmark_group("mesh_chunk");

    g.bench_function("gather_masks", |b| {
        let mut i = 0;
        b.iter(|| {
            builder.gather(black_box(&neighborhoods[i]), is_transparent);
            i = (i + 1) % n;
        });
    });

    // Pre-gathered buffers for the pure fast_mesh measurement.
    let mut gathered: Vec<Gathered> = Vec::new();
    let mut quads_total = 0usize;
    let mut out = MeshChunkQuads::default();
    for nb in &neighborhoods {
        builder.build_into(nb, is_transparent, &mut out);
        quads_total += out.len();
        gathered.push((
            Box::new(*builder.voxels()),
            Box::new(*builder.masks().opaque()),
            Box::new(*builder.masks().transparent()),
        ));
    }
    eprintln!(
        "quads/chunk {:.0} ({:.1} KB at 8 B/quad)",
        quads_total as f64 / n as f64,
        quads_total as f64 / n as f64 * 8.0 / 1024.0
    );
    let mut mesher = Mesher::<CS>::new();
    g.bench_function("fast_mesh", |b| {
        let mut i = 0;
        b.iter(|| {
            let (v, o, t) = &gathered[i];
            mesher.clear();
            mesher.fast_mesh(black_box(&v[..]), &o[..], &t[..]);
            black_box(&mesher.quads);
            i = (i + 1) % n;
        });
    });

    g.bench_function("build", |b| {
        let mut i = 0;
        b.iter(|| {
            builder.build_into(black_box(&neighborhoods[i]), is_transparent, &mut out);
            black_box(&out);
            i = (i + 1) % n;
        });
    });

    g.bench_function("remesh", |b| {
        builder.gather(&neighborhoods[n / 2], is_transparent);
        b.iter(|| {
            builder.remesh_into(&mut out);
            black_box(&out);
        });
    });

    // Single-edit mask update on a surface chunk, alternating air <-> glass.
    builder.gather(&neighborhoods[n / 2], is_transparent);
    let mut toggle = false;
    g.bench_function("mask_update", |b| {
        b.iter(|| {
            toggle = !toggle;
            let v = if toggle {
                BlockStateId(7)
            } else {
                BlockStateId::AIR
            };
            builder.set_block(
                black_box(16),
                black_box(8),
                black_box(16),
                v,
                is_transparent,
            );
        });
    });

    // Mask rebuild from scratch for comparison with the incremental update.
    let voxels = *builder.voxels();
    let mut masks = MaskCache::new();
    g.bench_function("mask_rebuild", |b| {
        b.iter(|| {
            masks.rebuild(black_box(&voxels), is_transparent);
        });
    });

    g.finish();
}

criterion_group!(benches, bench_mesh);
criterion_main!(benches);
