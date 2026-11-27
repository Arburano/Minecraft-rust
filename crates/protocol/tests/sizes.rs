//! T2-01: measured message sizes. Run with `--nocapture` to see the numbers:
//! `cargo test -p protocol --release --test sizes -- --nocapture`.
//!
//! Asserts only the hard limits (datagrams ≤ 1,200 bytes); the numbers are
//! recorded in `docs/architecture/networking-protocol.md`.

mod common;

use common::*;
use protocol::*;
use world::registry::blocks_from_dir;
use world::storage::ColumnRecord;
use world::{ColumnPos, RegistryBuilder, WorldBounds};
use worldgen::{GenContext, Pipeline, WorldgenConfig};

#[test]
fn datagrams_fit_and_sizes_are_reported() {
    for n in [1, 3] {
        let len = input(n).encode().expect("input").len();
        println!("input datagram, {n} intent(s): {len} B");
        assert!(len <= MAX_DATAGRAM);
    }
    let len = input(MAX_INTENTS).encode().expect("input").len();
    println!("input datagram, {MAX_INTENTS} intents (max): {len} B");

    let twenty = moving(20).encode().expect("entities").len();
    println!("entity delta, 20 known entities moving and turning (steady state): {twenty} B");
    assert!(twenty <= MAX_DATAGRAM);
    let kbit = twenty as f64 * 8.0 * 20.0 / 1000.0;
    println!("  at 20 Hz: {kbit:.1} kbit/s (payload only, without QUIC/UDP/IP headers)");
    let new20 = entities(20).encode().expect("entities").len();
    println!("entity delta, 20 entities appearing (kind + position + look + flags): {new20} B");
    let c = corrected().encode().expect("c").len() - moving(2).encode().expect("m").len();
    println!("a correction adds {c} B");
    let ack = EntityDatagram {
        snapshot: 2,
        baseline: 1,
        ack_tick: Some(1000),
        ..EntityDatagram::default()
    };
    println!(
        "empty datagram (ack only): {} B",
        ack.encode().expect("ack").len()
    );

    // The most appearing entities that fit one datagram.
    let mut fit = 0;
    for n in 1..=MAX_ENTITY_UPDATES {
        if entities(n).encode().is_ok() {
            fit = n;
        }
    }
    println!("entity delta: up to {fit} appearing entities per datagram");
    assert!(fit >= 40);
}

#[test]
fn generated_columns_compress_as_expected() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mods/core/data");
    let mut b = RegistryBuilder::new();
    b.add_mod(
        "core",
        blocks_from_dir(&dir.join("blocks")).expect("blocks"),
    );
    let reg = b.build().expect("registry");
    let src = std::fs::read_to_string(dir.join("worldgen/overworld.toml")).expect("config");
    let cfg = WorldgenConfig::from_toml(&src, &reg).expect("config");
    let ctx = GenContext::new(7, cfg, WorldBounds::default());
    let pipeline = Pipeline::overworld();

    let mut wire = Vec::new();
    let mut raw = Vec::new();
    for x in 0..8 {
        for z in 0..8 {
            let pos = ColumnPos {
                x: x * 5 - 20,
                z: z * 5 - 20,
            };
            let column = pipeline
                .generate(&ctx, pos)
                .into_column(&reg)
                .expect("column");
            let record = ColumnRecord {
                pos,
                bounds: column.bounds(),
                sections: column.sections().to_vec(),
                light: Vec::new(),
                entities: Vec::new(),
            };
            raw.push(world::storage::encode_column(&record, 0).len());
            let msg = ChunkStream::Column(ColumnPayload::encode(&record).expect("payload"));
            let bytes = msg.encode().expect("encode");
            wire.push(bytes.len());
            let ChunkStream::Column(back) = ChunkStream::decode(&bytes).expect("decode");
            assert_eq!(back.decode().expect("record").pos, pos);
        }
    }
    let stats = |v: &mut Vec<usize>| {
        v.sort_unstable();
        (
            v[0],
            v[v.len() / 2],
            v.iter().sum::<usize>() / v.len(),
            v[v.len() - 1],
        )
    };
    let (rmin, rmed, rmean, rmax) = stats(&mut raw);
    let (wmin, wmed, wmean, wmax) = stats(&mut wire);
    println!(
        "generated column (24 sections, no light), {} columns: raw record min {rmin} / median {rmed} / mean {rmean} / max {rmax} B",
        wire.len()
    );
    println!(
        "  on the wire (zstd 1 + framing): min {wmin} / median {wmed} / mean {wmean} / max {wmax} B"
    );
    println!(
        "  login at 12 chunks (625 columns): ≈ {:.2} MB",
        wmean as f64 * 625.0 / 1e6
    );
}
