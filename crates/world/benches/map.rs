//! T1-01: block access through `WorldMap` vs direct `Section` access.
//!
//! Budget (proposed in T1-01, performance-budgets "World map get/set"): map
//! `get_block` / `set_block` within 2x of section `get` / `set` on the same cells.
//! Corpus: T0-12 `test_corpus`, seed 0, 8x8 columns, sections 0..8. 4,096 random
//! cells per iteration (fixed RNG); results are per batch, divide by 4,096.

use std::hint::black_box;
use std::sync::Arc;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use world::coords::BlockPos;
use world::{
    BlockDef, BlockProps, BlockStateId, Column, ColumnPos, RegistryBuilder, Section, WorldBounds,
    WorldMap,
};
use worldgen::test_corpus::{CorpusArea, test_corpus};

const N: usize = 4096;

struct Rng(u64);
impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % n
    }
}

fn bench_map(c: &mut Criterion) {
    // Registry with ten states so the corpus' numeric IDs 0..=9 are all valid.
    let mut b = RegistryBuilder::new();
    b.add_mod(
        "bench",
        (2..10)
            .map(|i| BlockDef::simple(&format!("bench:b{i}"), BlockProps::MISSING))
            .collect(),
    );
    let registry = Arc::new(b.build().expect("valid registry"));
    let bounds = WorldBounds {
        min_section_y: 0,
        section_count: 8,
    };
    let area = CorpusArea {
        x: 0..8,
        y: 0..8,
        z: 0..8,
    };
    let columns = test_corpus(0, &area);
    let mut map = WorldMap::new(Arc::clone(&registry), bounds);
    let mut sections: Vec<Section> = Vec::new();
    for col in &columns {
        sections.extend(col.sections().iter().cloned());
        let column = Column::from_sections(
            ColumnPos { x: col.x, z: col.z },
            bounds,
            col.sections().to_vec(),
            &registry,
        )
        .expect("8 sections");
        map.insert_column(column).expect("same bounds");
    }
    // Same random cells for both paths: (block position, section slot, index).
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    let cells: Vec<(BlockPos, usize, usize)> = (0..N)
        .map(|_| {
            let (cx, cz) = (rng.below(8) as i32, rng.below(8) as i32);
            let (x, y, z) = (
                rng.below(16) as i32,
                rng.below(128) as i32,
                rng.below(16) as i32,
            );
            let slot = (cx * 8 + cz) as usize * 8 + (y / 16) as usize;
            let index = (((y % 16) * 16 + z) * 16 + x) as usize;
            (BlockPos::new(cx * 16 + x, y, cz * 16 + z), slot, index)
        })
        .collect();

    let mut g = c.benchmark_group("world_map");
    g.throughput(Throughput::Elements(N as u64));
    g.bench_function("section_get_4096", |b| {
        b.iter(|| {
            let mut acc = 0u32;
            for &(_, slot, index) in &cells {
                acc = acc.wrapping_add(u32::from(sections[slot].get(index).0));
            }
            black_box(acc)
        })
    });
    g.bench_function("map_get_4096", |b| {
        b.iter(|| {
            let mut acc = 0u32;
            for &(pos, _, _) in &cells {
                acc = acc.wrapping_add(u32::from(map.get_block(pos).map_or(0, |id| id.0)));
            }
            black_box(acc)
        })
    });
    let mut flip = false;
    g.bench_function("section_set_4096", |b| {
        b.iter(|| {
            flip = !flip;
            let id = BlockStateId(if flip { 3 } else { 4 });
            for &(_, slot, index) in &cells {
                sections[slot].set(index, id);
            }
        })
    });
    let mut dirty = Vec::new();
    g.bench_function("map_set_4096", |b| {
        b.iter(|| {
            flip = !flip;
            let id = BlockStateId(if flip { 3 } else { 4 });
            for &(pos, _, _) in &cells {
                black_box(map.set_block(pos, id).expect("loaded"));
            }
            // Drain so the dirty lists stay small, as a consumer would each frame.
            map.take_dirty(world::DirtyKind::Mesh, &mut dirty);
            map.take_dirty(world::DirtyKind::Light, &mut dirty);
            map.take_dirty(world::DirtyKind::Save, &mut dirty);
            dirty.clear();
        })
    });
    g.finish();
}

criterion_group!(benches, bench_map);
criterion_main!(benches);
