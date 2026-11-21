//! T1-07: conversion cost of the fallback set and cache load.
//!
//! Measures: generating + converting all 16 fallback textures into one texture
//! array with mip chains; encoding and decoding the cache format. Memory of the
//! converted set is printed (bytes of all mip levels).

use std::hint::black_box;

use assets::cache::{decode, encode};
use assets::{FALLBACK_KEYS, fallback_set};
use criterion::{Criterion, criterion_group, criterion_main};

fn bench_assets(c: &mut Criterion) {
    let set = fallback_set(FALLBACK_KEYS).expect("fallback set");
    let bytes: usize = set.arrays().iter().map(|a| a.byte_len()).sum();
    eprintln!(
        "fallback set: {} textures, {} arrays, {bytes} bytes with mips",
        FALLBACK_KEYS.len(),
        set.arrays().len()
    );
    let encoded = encode(&set.arrays()[0]);
    let mut g = c.benchmark_group("assets");
    g.bench_function("convert_fallback_set", |b| {
        b.iter(|| black_box(fallback_set(FALLBACK_KEYS).expect("fallback set")))
    });
    g.bench_function("encode_cache_entry", |b| {
        b.iter(|| black_box(encode(&set.arrays()[0])))
    });
    g.bench_function("decode_cache_entry", |b| {
        b.iter(|| black_box(decode(&encoded).expect("valid")))
    });
    g.finish();
}

criterion_group!(benches, bench_assets);
criterion_main!(benches);
