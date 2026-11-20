//! T0-12 determinism acceptance test: the corpus hash must not depend on
//! generation order or thread count (docs/architecture/worldgen.md, "Tests").

use std::collections::HashMap;

use worldgen::test_corpus::{CorpusArea, TestColumn, test_column, test_corpus};

const SEED: u64 = 0x5eed_0012;

fn area() -> CorpusArea {
    CorpusArea {
        x: -2..3,
        y: -1..5,
        z: -1..4,
    }
}

fn hashes(columns: &[TestColumn]) -> HashMap<(i32, i32), u64> {
    columns.iter().map(|c| ((c.x, c.z), c.hash())).collect()
}

/// Generates the area's columns on `threads` threads, in an order rotated by
/// `shift`, and returns the per-column hashes.
fn generate_scattered(shift: usize, threads: usize) -> HashMap<(i32, i32), u64> {
    let area = area();
    let mut coords: Vec<(i32, i32)> = Vec::new();
    for x in area.x.clone() {
        for z in area.z.clone() {
            coords.push((x, z));
        }
    }
    let shift = shift % coords.len();
    coords.rotate_left(shift);
    let chunks: Vec<&[(i32, i32)]> = coords.chunks(coords.len().div_ceil(threads)).collect();
    let mut columns = Vec::new();
    std::thread::scope(|s| {
        let handles: Vec<_> = chunks
            .into_iter()
            .map(|chunk| {
                let y = area.y.clone();
                s.spawn(move || {
                    chunk
                        .iter()
                        .map(|&(x, z)| test_column(SEED, x, z, y.clone()))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        for handle in handles {
            columns.extend(handle.join().expect("generator thread panicked"));
        }
    });
    hashes(&columns)
}

#[test]
fn hash_is_independent_of_order_and_thread_count() {
    let reference = hashes(&test_corpus(SEED, &area()));
    assert_eq!(reference.len(), area().column_count());

    // Reverse order, single thread.
    let mut reversed: Vec<TestColumn> = area()
        .x
        .rev()
        .flat_map(|x| area().z.rev().map(move |z| (x, z)))
        .map(|(x, z)| test_column(SEED, x, z, area().y))
        .collect();
    reversed.reverse();
    assert_eq!(hashes(&reversed), reference);

    // Rotated orders on 1, 2, 3, 4 and 8 threads.
    for threads in [1usize, 2, 3, 4, 8] {
        for shift in [0usize, 1, 7, 13] {
            assert_eq!(
                generate_scattered(shift, threads),
                reference,
                "threads={threads} shift={shift}"
            );
        }
    }
}

#[test]
fn repeated_generation_is_identical() {
    let a = test_corpus(SEED, &area());
    let b = test_corpus(SEED, &area());
    assert_eq!(hashes(&a), hashes(&b));
}
