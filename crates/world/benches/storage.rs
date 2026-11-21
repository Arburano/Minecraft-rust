//! T1-02: save/load of one column and of a 12-chunk-radius area (25×25 = 625
//! columns) in the region format, plus compressed size per column (data for
//! open question 7). Corpus: T0-12 `test_corpus`, seed 0, 8 sections per
//! column (y 0..128); the corpus has no light or entities. Writes include the
//! fsyncs of the copy-on-write protocol, so numbers depend on the disk.

use std::hint::black_box;
use std::path::PathBuf;

use criterion::{Criterion, criterion_group, criterion_main};
use world::storage::{ColumnRecord, WorldStore, encode_column};
use world::{BlockDef, BlockProps, ColumnPos, RegistryBuilder, WorldBounds};
use worldgen::test_corpus::{CorpusArea, test_corpus};

const BOUNDS: WorldBounds = WorldBounds {
    min_section_y: 0,
    section_count: 8,
};

fn bench_storage(c: &mut Criterion) {
    let mut b = RegistryBuilder::new();
    b.add_mod(
        "t",
        (2..10)
            .map(|i| BlockDef::simple(&format!("t:b{i}"), BlockProps::MISSING))
            .collect(),
    );
    let reg = b.build().expect("registry");
    let area = CorpusArea {
        x: -12..13,
        y: 0..8,
        z: -12..13,
    };
    let records: Vec<ColumnRecord> = test_corpus(0, &area)
        .into_iter()
        .map(|col| ColumnRecord {
            pos: ColumnPos { x: col.x, z: col.z },
            bounds: BOUNDS,
            sections: col.sections().to_vec(),
            light: Vec::new(),
            entities: Vec::new(),
        })
        .collect();
    let raw: usize = records.iter().map(|r| encode_column(r, 0).len()).sum();
    let dir: PathBuf =
        std::env::temp_dir().join(format!("world-store-bench-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut store = WorldStore::open(&dir, &reg, BOUNDS).expect("open");
    store.save_all(&records).expect("save");
    store.compact(0).expect("compact");
    let disk: u64 = std::fs::read_dir(dir.join("region"))
        .expect("regions")
        .filter_map(Result::ok)
        .map(|e| e.metadata().map_or(0, |m| m.len()))
        .sum();
    let headers = 4 * 16_392u64; // 625 columns span 4 regions here
    eprintln!(
        "{} columns: uncompressed records {:.0} B/column, region files {:.0} B/column \
         ({:.0} B/column without the 4 region headers)",
        records.len(),
        raw as f64 / records.len() as f64,
        disk as f64 / records.len() as f64,
        (disk - headers) as f64 / records.len() as f64
    );

    let mut g = c.benchmark_group("world_storage");
    g.sample_size(10);
    let one = records[records.len() / 2].clone();
    g.bench_function("save_one_column", |b| {
        b.iter(|| store.save(&one).expect("save"))
    });
    g.bench_function("load_one_column", |b| {
        b.iter(|| black_box(store.load(one.pos).expect("load").expect("present")))
    });
    g.bench_function("save_all_625_columns", |b| {
        b.iter(|| store.save_all(&records).expect("save"))
    });
    g.bench_function("load_all_625_columns", |b| {
        b.iter(|| {
            for r in &records {
                black_box(store.load(r.pos).expect("load").expect("present"));
            }
        })
    });
    g.finish();
    let _ = std::fs::remove_dir_all(dir);
}

criterion_group!(benches, bench_storage);
criterion_main!(benches);
