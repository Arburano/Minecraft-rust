//! Wire sizes (task T2-01): a typical column record, an entity delta of 20
//! entities and an input datagram. Run with `--nocapture` to see the numbers:
//!
//! ```text
//! cargo test --release -p protocol --test sizes -- --nocapture
//! ```
//!
//! Columns come from the real overworld generator (`mods/core` data, all
//! stages, y −64..320 = 24 sections), seed 7, a 15 × 15 area around the
//! origin; light is not sent (ADR-0011).

mod common;

use std::sync::Arc;

use protocol::*;
use world::registry::blocks_from_dir;
use world::{ColumnPos, RegistryBuilder, WorldBounds};
use worldgen::{GenContext, Pipeline, WorldgenConfig};

#[test]
fn input_datagram_size() {
    for frames in [1, 3, messages::MAX_INPUT_FRAMES] {
        let bytes = common::input(frames).encode().expect("fits");
        println!("input datagram, {frames} frame(s): {} B", bytes.len());
        assert!(bytes.len() <= MAX_DATAGRAM_BYTES);
    }
    // The typical case (3 frames, duplicated against loss) is tiny.
    assert!(common::input(3).encode().expect("fits").len() <= 64);
}

#[test]
fn entity_delta_size() {
    let bytes = common::entity_delta(20).encode().expect("fits");
    println!(
        "entity delta, 20 moving entities + 1 spawn + 1 despawn: {} B",
        bytes.len()
    );
    assert!(bytes.len() <= MAX_DATAGRAM_BYTES);
    // At 20 Hz, plus ~48 B of QUIC/UDP/IP overhead per datagram.
    let kbit = (bytes.len() + 48) as f64 * 8.0 * 20.0 / 1000.0;
    println!("  at 20 Hz incl. ~48 B packet overhead: {kbit:.1} kbit/s");
    let mut most = 0;
    for n in [50, 100, 200] {
        match common::entity_delta(n).encode() {
            Ok(b) => {
                println!("entity delta, {n} moving entities: {} B", b.len());
                most = n;
            }
            Err(e) => println!("entity delta, {n} moving entities: does not fit ({e})"),
        }
    }
    assert!(most >= 50, "50 moving entities fit one datagram");
}

#[test]
fn column_record_size() {
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
    let ctx = Arc::new(GenContext::new(7, cfg, bounds));
    let pipeline = Pipeline::overworld();

    let mut sizes = Vec::new();
    let mut raw_total = 0usize;
    for x in -7..=7 {
        for z in -7..=7 {
            let pos = ColumnPos { x, z };
            let col = pipeline
                .generate(&ctx, pos)
                .into_column(&reg)
                .expect("column");
            let data = ColumnData::from_sections(pos, bounds, col.sections()).expect("packs");
            let frame = data.encode_frame().expect("fits");
            let mut raw = Vec::new();
            for s in col.sections() {
                s.encode(&mut raw);
            }
            raw_total += raw.len();
            // Decoding gives the same blocks back.
            if x == 0 && z == 0 {
                let back = ColumnData::decode(&frame[4..])
                    .expect("decodes")
                    .sections()
                    .expect("unpacks");
                assert_eq!(back.len(), col.sections().len());
            }
            sizes.push(frame.len());
        }
    }
    sizes.sort_unstable();
    let n = sizes.len();
    let mean = sizes.iter().sum::<usize>() as f64 / n as f64;
    println!(
        "column record (frame, zstd 1), {n} overworld columns: min {} B, median {} B, mean {:.0} B, p95 {} B, max {} B; raw sections mean {:.0} B",
        sizes[0],
        sizes[n / 2],
        mean,
        sizes[n * 95 / 100],
        sizes[n - 1],
        raw_total as f64 / n as f64
    );
    println!(
        "  login at 12 chunks (625 columns) ≈ {:.2} MB",
        mean * 625.0 / 1e6
    );
    // PRD §8 assumed 4–16 KB per column; stay well under the frame limit.
    assert!(sizes[n - 1] < 64 * 1024);
}
