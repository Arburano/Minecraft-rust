//! Decoding garbage never panics (networking-protocol.md, "Tests": the server
//! never panics on garbage input).
//!
//! Two sources of input for every public decoder:
//! - arbitrary bytes;
//! - valid encodings with random byte flips, truncations and insertions,
//!   which reach much deeper into the decoders than pure noise.
//!
//! A decoder may accept a mutated message (bitcode has no checksum; QUIC
//! already authenticates every packet); it must only never panic and never
//! return something that breaks its own limits.

mod common;

use proptest::prelude::*;
use protocol::*;
use world::{BlockStateId, ColumnPos, Section, WorldBounds};

/// Runs every decoder on `bytes`; asserts the documented limits on success.
fn decode_all(bytes: &[u8]) {
    let _ = Hello::decode(bytes);
    let _ = ServerHello::decode(bytes);
    let _ = ClientControl::decode(bytes);
    let _ = ServerControl::decode(bytes);
    let _ = ClientEvent::decode(bytes);
    let _ = ServerEvent::decode(bytes);
    if let Ok(d) = InputDatagram::decode(bytes) {
        assert!(d.frames.len() <= messages::MAX_INPUT_FRAMES);
    }
    let _ = EntityDelta::decode(bytes);
    if let Ok(c) = ColumnData::decode(bytes)
        && let Ok(s) = c.sections()
    {
        assert!(s.len() <= column::MAX_COLUMN_SECTIONS as usize);
    }
    let mut r = FrameReader::new();
    r.push(bytes);
    while let Ok(Some(_)) = r.next_frame() {}
}

/// Valid encodings of every message kind, as frame bodies or datagrams.
fn corpus() -> Vec<Vec<u8>> {
    let mut blocks = [BlockStateId::AIR; 4096];
    for (i, b) in blocks.iter_mut().enumerate() {
        *b = BlockStateId((i * 7 % 13) as u16);
    }
    let column = ColumnData::from_sections(
        ColumnPos { x: 1, z: -2 },
        WorldBounds {
            min_section_y: 0,
            section_count: 2,
        },
        &[
            Section::from_blocks(&blocks),
            Section::uniform(BlockStateId(3)),
        ],
    )
    .expect("packs");
    let body = |f: Vec<u8>| f[4..].to_vec();
    vec![
        common::hello().encode(),
        ServerHello::Accepted(common::welcome()).encode(),
        ServerHello::Rejected(Rejection::Other("x".into())).encode(),
        body(ClientControl::Ready.encode_frame().expect("fits")),
        body(
            ServerControl::Registry(common::registry())
                .encode_frame()
                .expect("fits"),
        ),
        body(
            ServerControl::Spawn(common::spawn())
                .encode_frame()
                .expect("fits"),
        ),
        body(
            ClientEvent::Chat("hello".into())
                .encode_frame()
                .expect("fits"),
        ),
        body(
            ServerEvent::Chat {
                from: "a".into(),
                text: "b".into(),
            }
            .encode_frame()
            .expect("fits"),
        ),
        common::input(3).encode().expect("fits"),
        common::entity_delta(20).encode().expect("fits"),
        body(column.encode_frame().expect("fits")),
    ]
}

#[derive(Debug, Clone)]
enum Mutation {
    Flip(usize, u8),
    Truncate(usize),
    Insert(usize, u8),
    Set(usize, u8),
}

fn mutate(mut bytes: Vec<u8>, muts: &[Mutation]) -> Vec<u8> {
    for m in muts {
        let n = bytes.len().max(1);
        match *m {
            Mutation::Flip(i, mask) => {
                if let Some(b) = bytes.get_mut(i % n) {
                    *b ^= mask;
                }
            }
            Mutation::Truncate(i) => bytes.truncate(i % n),
            Mutation::Insert(i, v) => bytes.insert(i % (bytes.len() + 1), v),
            Mutation::Set(i, v) => {
                if let Some(b) = bytes.get_mut(i % n) {
                    *b = v;
                }
            }
        }
    }
    bytes
}

fn arb_mutation() -> impl Strategy<Value = Mutation> {
    prop_oneof![
        (any::<usize>(), 1u8..).prop_map(|(i, m)| Mutation::Flip(i, m)),
        any::<usize>().prop_map(Mutation::Truncate),
        (any::<usize>(), any::<u8>()).prop_map(|(i, v)| Mutation::Insert(i, v)),
        (
            any::<usize>(),
            prop_oneof![Just(0u8), Just(0xff), Just(0x7f), any::<u8>()]
        )
            .prop_map(|(i, v)| Mutation::Set(i, v)),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(4096))]

    #[test]
    fn garbage_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..2048)) {
        decode_all(&bytes);
    }

    #[test]
    fn garbage_with_handshake_prefix_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
        let mut b = b"VXLH".to_vec();
        b.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
        b.extend_from_slice(&bytes);
        decode_all(&b);
    }

    #[test]
    fn mutated_messages_never_panic(
        which in any::<prop::sample::Index>(),
        muts in proptest::collection::vec(arb_mutation(), 1..6),
    ) {
        let corpus = corpus();
        let bytes = mutate(corpus[which.index(corpus.len())].clone(), &muts);
        decode_all(&bytes);
    }
}

#[test]
fn every_truncation_of_every_message() {
    for msg in corpus() {
        for cut in 0..msg.len() {
            decode_all(&msg[..cut]);
        }
    }
}

#[test]
fn zstd_bomb_is_bounded() {
    // 4 MiB of zeros compresses to a few hundred bytes; a column claiming one
    // section must not decompress it.
    let bomb = zstd::bulk::compress(&vec![0u8; 4 << 20], 19).expect("compresses");
    let c = ColumnData {
        x: 0,
        z: 0,
        min_section_y: 0,
        section_count: 1,
        payload: bomb,
    };
    let frame = c.encode_frame().expect("small");
    let back = ColumnData::decode(&frame[4..]).expect("frame decodes");
    assert_eq!(back.sections().err(), Some(ColumnError::Compression));
}
