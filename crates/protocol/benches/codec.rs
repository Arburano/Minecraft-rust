//! T2-01: encode/decode cost of the main messages. Column: a real overworld
//! column (24 sections, `mods/core` data, seed 7) packed into a `ColumnData`
//! frame (section encoding + zstd 1) and unpacked back into sections; entity
//! delta of 20 moving entities; input datagram of 3 frames.

#[path = "../tests/common/mod.rs"]
mod common;

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use protocol::{ColumnData, EntityDelta, InputDatagram};
use world::registry::blocks_from_dir;
use world::{ColumnPos, RegistryBuilder, WorldBounds};
use worldgen::{GenContext, Pipeline, WorldgenConfig};

fn bench_codec(c: &mut Criterion) {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mods/core/data");
    let mut b = RegistryBuilder::new();
    b.add_mod(
        "core",
        blocks_from_dir(&dir.join("blocks")).expect("blocks"),
    );
    let reg = b.build().expect("registry");
    let src = std::fs::read_to_string(dir.join("worldgen/overworld.toml")).expect("config");
    let cfg = WorldgenConfig::from_toml(&src, &reg).expect("config");
    let bounds = WorldBounds::default();
    let ctx = GenContext::new(7, cfg, bounds);
    let pos = ColumnPos { x: 3, z: -2 };
    let col = Pipeline::overworld()
        .generate(&ctx, pos)
        .into_column(&reg)
        .expect("column");
    let frame = ColumnData::from_sections(pos, bounds, col.sections())
        .expect("packs")
        .encode_frame()
        .expect("fits");

    let mut g = c.benchmark_group("protocol");
    g.bench_function("column_encode", |b| {
        b.iter(|| {
            black_box(
                ColumnData::from_sections(pos, bounds, black_box(col.sections()))
                    .expect("packs")
                    .encode_frame()
                    .expect("fits"),
            )
        })
    });
    g.bench_function("column_decode", |b| {
        b.iter(|| {
            black_box(
                ColumnData::decode(black_box(&frame[4..]))
                    .expect("decodes")
                    .sections()
                    .expect("unpacks"),
            )
        })
    });

    let delta = common::entity_delta(20);
    let delta_bytes = delta.encode().expect("fits");
    g.bench_function("entity_delta_20_encode", |b| {
        b.iter(|| black_box(black_box(&delta).encode().expect("fits")))
    });
    g.bench_function("entity_delta_20_decode", |b| {
        b.iter(|| black_box(EntityDelta::decode(black_box(&delta_bytes)).expect("decodes")))
    });

    let input = common::input(3);
    let input_bytes = input.encode().expect("fits");
    g.bench_function("input_3_encode", |b| {
        b.iter(|| black_box(black_box(&input).encode().expect("fits")))
    });
    g.bench_function("input_3_decode", |b| {
        b.iter(|| black_box(InputDatagram::decode(black_box(&input_bytes)).expect("decodes")))
    });
    g.finish();
}

criterion_group!(benches, bench_codec);
criterion_main!(benches);
