//! T2-01: decoding garbage never panics (`networking-protocol.md`, "Tests").
//!
//! Random bytes, and valid encodings with random corruption, go into every
//! decoder. Each must return `Ok` or `Err` — a panic fails the test.

mod common;

use common::*;
use proptest::prelude::*;
use protocol::*;

fn decode_everything(bytes: &[u8]) {
    let _ = HandshakeHeader::decode(bytes).map(HandshakeHeader::check);
    let _ = split_frame(bytes);
    let _ = ClientControl::decode(bytes);
    let _ = ServerControl::decode(bytes);
    let _ = InputDatagram::decode(bytes);
    let _ = EntityDatagram::decode(bytes);
    let _ = ClientEvent::decode(bytes);
    let _ = ServerEvent::decode(bytes);
    if let Ok(ChunkStream::Column(c)) = ChunkStream::decode(bytes) {
        let _ = c.decode();
    }
    // The column body straight from garbage, as a peer could send it.
    let _ = ColumnPayload {
        pos: ColumnXz { x: -3, z: 12 },
        dimension: 0,
        zstd: bytes.to_vec(),
    }
    .decode();
}

fn valid_samples() -> Vec<Vec<u8>> {
    let mut v = vec![
        hello().encode().expect("e"),
        welcome().encode().expect("e"),
        registry().encode().expect("e"),
        spawn().encode().expect("e"),
        kick().encode().expect("e"),
        input(3).encode().expect("e"),
        entities(20).encode().expect("e"),
        moving(20).encode().expect("e"),
        corrected().encode().expect("e"),
        ChunkStream::Column(ColumnPayload::encode(&small_record(), 0).expect("e"))
            .encode()
            .expect("e"),
        ColumnPayload::encode(&small_record(), 0).expect("e").zstd,
    ];
    v.extend(client_events().iter().map(|e| e.encode().expect("e")));
    v.extend(server_events().iter().map(|e| e.encode().expect("e")));
    v
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 2000, ..ProptestConfig::default() })]

    #[test]
    fn random_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..2048)) {
        decode_everything(&bytes);
    }

    #[test]
    fn corrupted_messages_never_panic(
        which in any::<prop::sample::Index>(),
        edits in proptest::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 1..8),
        cut in any::<prop::sample::Index>(),
    ) {
        let samples = valid_samples();
        let mut bytes = samples[which.index(samples.len())].clone();
        for (at, value) in edits {
            let i = at.index(bytes.len());
            bytes[i] = value;
        }
        decode_everything(&bytes);
        // Truncated as well.
        let n = cut.index(bytes.len() + 1);
        decode_everything(&bytes[..n]);
    }
}
