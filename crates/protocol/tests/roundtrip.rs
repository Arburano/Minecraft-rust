//! Every message type survives encode → frame/datagram → decode unchanged.

mod common;

use proptest::prelude::*;
use protocol::*;

/// Encodes as a stream frame, then reads it back through `FrameReader`.
fn through_frame(frame: Vec<u8>) -> Vec<u8> {
    let mut r = FrameReader::new();
    r.push(&frame);
    let body = r.next_frame().expect("valid").expect("complete").to_vec();
    assert_eq!(r.pending(), 0);
    body
}

#[test]
fn handshake_messages() {
    let h = common::hello();
    let body = through_frame(codec::frame(&h.encode()).expect("fits"));
    assert_eq!(Hello::decode(&body), Ok(h));

    let w = common::welcome();
    let body =
        through_frame(codec::frame(&ServerHello::Accepted(w.clone()).encode()).expect("fits"));
    assert_eq!(ServerHello::decode(&body), Ok(w));

    for r in [
        Rejection::OfflineNotAllowed,
        Rejection::InvalidName,
        Rejection::SessionNotSupported,
        Rejection::AlreadyConnected,
        Rejection::ServerFull,
        Rejection::Other("maintenance".into()),
    ] {
        let body = ServerHello::Rejected(r.clone()).encode();
        assert_eq!(ServerHello::decode(&body), Err(HandshakeError::Rejected(r)));
    }
}

#[test]
fn control_messages() {
    for m in [ClientControl::Ready, ClientControl::Disconnect] {
        let body = through_frame(m.encode_frame().expect("fits"));
        assert_eq!(ClientControl::decode(&body), Ok(m));
    }
    for m in [
        ServerControl::Registry(common::registry()),
        ServerControl::Spawn(common::spawn()),
        ServerControl::Kick("bye".into()),
    ] {
        let body = through_frame(m.encode_frame().expect("fits"));
        assert_eq!(ServerControl::decode(&body), Ok(m));
    }
}

#[test]
fn event_messages() {
    let m = ClientEvent::Chat("hi all".into());
    let body = through_frame(m.encode_frame().expect("fits"));
    assert_eq!(ClientEvent::decode(&body), Ok(m));
    for m in [
        ServerEvent::Chat {
            from: "Alex".into(),
            text: "hi".into(),
        },
        ServerEvent::System("Alex joined".into()),
    ] {
        let body = through_frame(m.encode_frame().expect("fits"));
        assert_eq!(ServerEvent::decode(&body), Ok(m));
    }
    let long = ClientEvent::Chat("x".repeat(messages::MAX_CHAT_BYTES + 1));
    let body = through_frame(long.encode_frame().expect("fits"));
    assert!(matches!(
        ClientEvent::decode(&body),
        Err(DecodeError::Invalid(_))
    ));
}

#[test]
fn datagrams() {
    let i = common::input(3);
    assert_eq!(InputDatagram::decode(&i.encode().expect("fits")), Ok(i));
    let too_many = common::input(messages::MAX_INPUT_FRAMES + 1);
    assert_eq!(
        InputDatagram::decode(&too_many.encode().expect("fits")),
        Err(DecodeError::Invalid("too many input frames"))
    );
    let d = common::entity_delta(20);
    assert_eq!(EntityDelta::decode(&d.encode().expect("fits")), Ok(d));
    let full = EntityDelta {
        baseline: None,
        updates: vec![EntityUpdate {
            id: NetEntityId(1),
            pos: Some(PosUpdate::Absolute(FixedPos {
                x: i32::MIN,
                y: 0,
                z: i32::MAX,
            })),
            yaw: None,
            pitch: Some(Angle8(255)),
            on_ground: false,
        }],
        ..EntityDelta::default()
    };
    assert_eq!(EntityDelta::decode(&full.encode().expect("fits")), Ok(full));
}

fn arb_frame() -> impl Strategy<Value = InputFrame> {
    (
        any::<u32>(),
        any::<u8>(),
        any::<i8>(),
        any::<i8>(),
        any::<u8>(),
        any::<u16>(),
        any::<u16>(),
    )
        .prop_map(
            |(tick, buttons, forward, strafe, fly_speed, yaw, pitch)| InputFrame {
                tick,
                buttons,
                forward,
                strafe,
                fly_speed,
                yaw: Angle16(yaw),
                pitch: Angle16(pitch),
            },
        )
}

fn arb_pos() -> impl Strategy<Value = FixedPos> {
    (any::<i32>(), any::<i32>(), any::<i32>()).prop_map(|(x, y, z)| FixedPos { x, y, z })
}

fn arb_update() -> impl Strategy<Value = EntityUpdate> {
    (
        any::<u32>(),
        prop_oneof![
            Just(None),
            (any::<i16>(), any::<i16>(), any::<i16>())
                .prop_map(|(dx, dy, dz)| Some(PosUpdate::Delta(PosDelta { dx, dy, dz }))),
            arb_pos().prop_map(|p| Some(PosUpdate::Absolute(p))),
        ],
        proptest::option::of(any::<u8>()),
        proptest::option::of(any::<u8>()),
        any::<bool>(),
    )
        .prop_map(|(id, pos, yaw, pitch, on_ground)| EntityUpdate {
            id: NetEntityId(id),
            pos,
            yaw: yaw.map(Angle8),
            pitch: pitch.map(Angle8),
            on_ground,
        })
}

proptest! {
    #[test]
    fn any_input_datagram(ack in any::<u32>(), frames in proptest::collection::vec(arb_frame(), 0..=messages::MAX_INPUT_FRAMES)) {
        let d = InputDatagram { ack_snapshot: ack, frames };
        prop_assert_eq!(InputDatagram::decode(&d.encode().expect("8 frames fit")), Ok(d));
    }

    #[test]
    fn any_entity_delta(
        snapshot in any::<u32>(),
        baseline in proptest::option::of(any::<u32>()),
        tick in any::<u32>(),
        updates in proptest::collection::vec(arb_update(), 0..40),
        despawns in proptest::collection::vec(any::<u32>(), 0..20),
        spawn_pos in arb_pos(),
    ) {
        let d = EntityDelta {
            snapshot,
            baseline,
            tick,
            ack_input: tick ^ 0x55,
            spawns: vec![EntitySpawn { id: NetEntityId(1), kind: 3, pos: spawn_pos, yaw: Angle8(1), pitch: Angle8(2) }],
            updates,
            despawns: despawns.into_iter().map(NetEntityId).collect(),
        };
        match d.encode() {
            Ok(bytes) => prop_assert_eq!(EntityDelta::decode(&bytes), Ok(d)),
            // Large random deltas may exceed one datagram; the body must still round-trip.
            Err(EncodeError::TooLarge { .. }) => prop_assert_eq!(codec::decode::<EntityDelta>(&codec::encode(&d)), Ok(d)),
        }
    }

    #[test]
    fn any_chat(text in "\\PC{0,200}") {
        let m = ClientEvent::Chat(text);
        let frame = m.encode_frame().expect("fits");
        prop_assert_eq!(ClientEvent::decode(&frame[4..]), Ok(m));
    }
}
