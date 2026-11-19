//! T0-04 benchmarks on the fixed T0-12 test corpus (`worldgen::test_corpus`,
//! seeded Perlin heightmap + caves + ores + water + trees, ~8% transparent
//! blocks).
//!
//! Budget rows (docs/process/performance-budgets.md):
//! - gather + masks of a 32^3 mesh chunk: <= 100 us p50
//! - fast_mesh of a 32^3 mesh chunk: <= 0.2 ms p50
//! - mask update per edit: <= 20 ns
//!
//! T0-05 adds the `render_batch` group on the same corpus (one 64^3 batch =
//! the 2x2x2 chunks at the grid origin): sub-range re-copy vs full re-copy
//! after an edit (open question No. 8, budget <= 50 us).
//!
//! Corpus: 4x2x4 mesh chunks (blocks x,z 0..128, y 0..64), sections cut from
//! the corpus columns so neighboring chunks share consistent padding.

use binary_greedy_meshing::Mesher;
use criterion::{Criterion, criterion_group, criterion_main};
use mesh::{
    CS, MaskCache, MeshChunkBuilder, MeshChunkQuads, P2, P3, PackedBatch, RenderBatch,
    SectionNeighborhood,
};
use std::hint::black_box;
use world::BlockStateId;
use world::coords::MeshChunkPos;
use worldgen::test_corpus::{CorpusArea, composition, is_transparent, test_corpus};

/// One pre-gathered chunk: voxels + opaque + transparent masks.
type Gathered = (Box<[u16; P3]>, Box<[u64; P2]>, Box<[u64; P2]>);

const GX: i32 = 4;
const GY: i32 = 2;
const GZ: i32 = 4;
/// Fixed corpus seed (docs/process/testing-and-benchmarks.md).
const SEED: u64 = 0;

fn build_corpus() -> Vec<SectionNeighborhood> {
    // One section of margin around the chunk grid: x,z -1..=8, y -1..=4.
    let area = CorpusArea {
        x: -1..9,
        y: -1..5,
        z: -1..9,
    };
    let columns = test_corpus(SEED, &area);
    // Composition over the mesh-chunk grid only (x,z 0..128, y 0..64).
    let comp = composition(
        columns
            .iter()
            .filter(|c| (0..8).contains(&c.x) && (0..8).contains(&c.z))
            .flat_map(|c| c.sections()[1..5].iter()),
    );
    let total = comp.total() as f64;
    eprintln!(
        "corpus: air {:.1}%  opaque {:.1}%  transparent {:.1}%",
        comp.air as f64 / total * 100.0,
        comp.opaque as f64 / total * 100.0,
        comp.transparent as f64 / total * 100.0
    );
    let z_len = (area.z.end - area.z.start) as usize;
    let column = |sx: i32, sz: i32| {
        &columns[(sx - area.x.start) as usize * z_len + (sz - area.z.start) as usize]
    };
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
                    column(sp.x, sp.z).section(sp.y).clone()
                }));
            }
        }
    }
    neighborhoods
}

fn bench_mesh(c: &mut Criterion) {
    let neighborhoods = build_corpus();
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

/// T0-05: one 64^3 render batch from the 2x2x2 mesh chunks at the grid
/// origin. Compares the two update variants after an edit (open question
/// No. 8): re-copying only the edited slot's sub-ranges vs re-copying and
/// packing the whole batch into 6 per-direction ranges.
fn bench_render_batch(c: &mut Criterion) {
    let neighborhoods = build_corpus();
    let mut builder = MeshChunkBuilder::new();
    // Slot order (sy, sz, sx); corpus index (cy * GZ + cz) * GX + cx.
    let mut chunks: [MeshChunkQuads; 8] = std::array::from_fn(|_| MeshChunkQuads::default());
    let mut batch = RenderBatch::new();
    for (s, chunk) in chunks.iter_mut().enumerate() {
        let (sy, sz, sx) = (s / 4, (s / 2) % 2, s % 2);
        let nb = &neighborhoods[(sy * GZ as usize + sz) * GX as usize + sx];
        builder.build_into(nb, is_transparent, chunk);
        batch.set_chunk(s, chunk);
    }
    let mut packed = PackedBatch::default();
    batch.pack_into(&mut packed);
    eprintln!(
        "render batch: {} quads ({:.1} KB at 8 B/quad)",
        batch.len(),
        batch.len() as f64 * 8.0 / 1024.0
    );

    let mut g = c.benchmark_group("render_batch");

    // Variant B (sub-ranges): after an edit, re-copy only the edited slot.
    g.bench_function("edit_subrange", |b| {
        let mut s = 0;
        b.iter(|| {
            batch.set_chunk(black_box(s), black_box(&chunks[s]));
            s = (s + 1) % 8;
        });
    });

    // Full pack of all 8 slots into 6 contiguous ranges (the extra cost of
    // variant A on top of the sub-range copy).
    g.bench_function("pack_all", |b| {
        b.iter(|| {
            batch.pack_into(black_box(&mut packed));
            black_box(&packed);
        });
    });

    // Variant A (full re-copy): after an edit, re-copy the slot and re-pack
    // the whole batch.
    g.bench_function("edit_recopy_all", |b| {
        let mut s = 0;
        b.iter(|| {
            batch.set_chunk(black_box(s), black_box(&chunks[s]));
            batch.pack_into(&mut packed);
            black_box(&packed);
            s = (s + 1) % 8;
        });
    });

    g.finish();
}

criterion_group!(benches, bench_mesh, bench_render_batch);
criterion_main!(benches);
