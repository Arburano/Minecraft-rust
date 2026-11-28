//! T2-01: every message round-trips; the handshake bytes match golden files.
//!
//! Golden files pin the wire format: a failure means the format changed. If
//! that was deliberate, raise `PROTOCOL_VERSION` and refresh them with
//! `UPDATE_GOLDEN=1 cargo test -p protocol --test messages`.

mod common;

use std::path::PathBuf;

use common::*;
use protocol::*;

fn round_trip<M: Message + PartialEq + std::fmt::Debug>(msg: &M) -> Vec<u8> {
    let bytes = msg.encode().expect("encode");
    assert!(bytes.len() <= M::MAX_LEN);
    assert_eq!(&M::decode(&bytes).expect("decode"), msg);
    // Framed form as well.
    let mut framed = Vec::new();
    encode_frame(msg, &mut framed).expect("frame");
    let (body, used) = split_frame(&framed).expect("split").expect("complete");
    assert_eq!(used, framed.len());
    assert_eq!(&M::decode(body).expect("decode framed"), msg);
    bytes
}

#[test]
fn every_message_round_trips() {
    round_trip(&hello());
    round_trip(&ClientControl::Leave);
    for m in [welcome(), registry(), spawn(), kick()] {
        round_trip(&m);
    }
    for n in 1..=MAX_INTENTS {
        round_trip(&input(n));
    }
    round_trip(&entities(0));
    round_trip(&entities(20));
    round_trip(&moving(20));
    round_trip(&corrected());
    for e in client_events() {
        round_trip(&e);
    }
    for e in server_events() {
        round_trip(&e);
    }
    let payload = ColumnPayload::encode(&small_record()).expect("column");
    round_trip(&ChunkStream::Column(payload));
}

#[test]
fn column_payload_round_trips_blocks() {
    let record = small_record();
    let payload = ColumnPayload::encode(&record).expect("encode");
    let back = payload.decode().expect("decode");
    assert_eq!(back.pos, record.pos);
    assert_eq!(back.bounds, record.bounds);
    for (a, b) in back.sections.iter().zip(&record.sections) {
        for i in 0..4096 {
            assert_eq!(a.get(i), b.get(i));
        }
    }
}

#[test]
fn column_payload_with_wrong_position_is_rejected() {
    let mut payload = ColumnPayload::encode(&small_record()).expect("encode");
    payload.pos.x += 1;
    assert!(matches!(
        payload.decode(),
        Err(ProtocolError::Invalid("column position"))
    ));
}

#[test]
fn handshake_header_detects_versions_and_strangers() {
    let ours = HandshakeHeader::current();
    let back = HandshakeHeader::decode(&ours.encode()).expect("decode");
    assert_eq!(back, ours);
    assert_eq!(back.check(), Ok(()));

    let newer = HandshakeHeader {
        protocol: PROTOCOL_VERSION + 1,
    };
    let err = HandshakeHeader::decode(&newer.encode())
        .expect("decode")
        .check()
        .expect_err("mismatch");
    assert!(err.to_string().contains("Update the game"), "{err}");

    assert_eq!(
        HandshakeHeader::decode(b"GET / HTTP/1.1"),
        Err(ProtocolError::BadMagic)
    );
    assert_eq!(
        HandshakeHeader::decode(b"VXE"),
        Err(ProtocolError::Truncated)
    );
}

#[test]
fn invalid_values_are_refused_on_both_sides() {
    // The sender refuses what the receiver would reject.
    let mut gap = input(2);
    gap.intents[1].tick += 5;
    assert!(gap.encode().is_err());
    let none = InputDatagram {
        ack_snapshot: 0,
        intents: vec![],
    };
    assert!(none.encode().is_err());
    let mut bits = input(1);
    bits.intents[0].buttons = Buttons(1 << 15);
    assert!(bits.encode().is_err());

    let mut e = entities(1);
    e.baseline = e.snapshot;
    assert!(e.encode().is_err());
    let mut c = corrected();
    if let Some(k) = c.correction.as_mut() {
        k.pos[1] = f64::NAN;
    }
    assert!(c.encode().is_err());
    assert!(entities(MAX_ENTITY_UPDATES + 1).encode().is_err());

    let long = ClientEvent::Chat {
        text: "x".repeat(MAX_CHAT_LEN + 1),
    };
    assert!(long.encode().is_err());
    let control = ClientEvent::Chat {
        text: "a\u{7}b".into(),
    };
    assert!(control.encode().is_err());

    // The receiver refuses it too: encode by hand with bitcode.
    let raw = bitcode::encode(&gap);
    assert!(matches!(
        InputDatagram::decode(&raw),
        Err(ProtocolError::Invalid(_))
    ));
}

fn golden(name: &str, bytes: &[u8]) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, bytes).expect("write golden");
        return;
    }
    let expected = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("{}: {e} (run with UPDATE_GOLDEN=1)", path.display()));
    assert_eq!(
        bytes,
        expected.as_slice(),
        "{name}: the wire format changed. If deliberate, raise PROTOCOL_VERSION and run with UPDATE_GOLDEN=1"
    );
}

#[test]
fn handshake_bytes_match_golden_files() {
    assert_eq!(PROTOCOL_VERSION, 5, "a version bump needs new golden files");
    golden("header.bin", &HandshakeHeader::current().encode());
    golden("hello.bin", &hello().encode().expect("hello"));
    golden("welcome.bin", &welcome().encode().expect("welcome"));
    golden("registry.bin", &registry().encode().expect("registry"));
    golden("spawn.bin", &spawn().encode().expect("spawn"));
    golden("kick.bin", &kick().encode().expect("kick"));
}
