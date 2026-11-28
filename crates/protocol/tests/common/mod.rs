//! Sample messages shared by the integration tests.

#![allow(dead_code)]

use protocol::handshake::ComponentSchema;
use protocol::messages::buttons;
use protocol::*;

pub fn hello() -> Hello {
    Hello {
        client_version: "0.0.1".into(),
        identity: ClaimedIdentity::Offline {
            name: "Alex_42".into(),
        },
        session_jwt: None,
    }
}

pub fn welcome() -> Welcome {
    Welcome {
        server_info: ServerInfo {
            name: "Test server".into(),
            motd: "Hello, world".into(),
            tick_rate: 20,
            view_distance: 12,
        },
        player_id: [
            0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0x8d, 0xef, 0x80, 0x11, 0x22, 0x33, 0x44, 0x55,
            0x66, 0x77,
        ],
        mod_manifest_hash: [0xab; 32],
    }
}

pub fn registry() -> Registry {
    Registry {
        id_table: vec![
            "core:air".into(),
            "core:missing".into(),
            "core:stone".into(),
            "core:log[axis=y]".into(),
        ],
        entity_kinds: vec!["core:player".into(), "core:pig".into()],
        component_schemas: vec![ComponentSchema {
            name: "example:mana".into(),
            schema_hash: [7; 32],
        }],
    }
}

pub fn spawn() -> Spawn {
    Spawn {
        pos: FixedPos::from_blocks([8.5, 72.0, -3.25]),
        yaw: Angle16::from_radians(1.0),
        pitch: Angle16::from_radians(-0.25),
        tick: 1234,
        entity: 1,
        min_section_y: -4,
        section_count: 24,
    }
}

/// Deterministic pseudo-random numbers (splitmix64), so sample messages are
/// not unrealistically regular (bitcode packs regular data very well).
pub fn noise(i: u64) -> u64 {
    let mut z = i.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

pub fn input(frames: usize) -> InputDatagram {
    InputDatagram {
        ack_snapshot: 77_000,
        frames: (0..frames as u32)
            .map(|i| InputFrame {
                tick: 1_000_000 + i,
                buttons: buttons::JUMP | buttons::SPRINT,
                forward: 127,
                strafe: -64,
                fly_speed: 16,
                yaw: Angle16(noise(u64::from(i)) as u16),
                pitch: Angle16(noise(u64::from(i) + 100) as u16),
            })
            .collect(),
    }
}

/// A steady-state delta: `moving` entities walking (up to ~0.45 block, i.e.
/// two 20 Hz snapshots at sprint speed, per horizontal axis), each with a new
/// yaw, scattered IDs, plus one spawn and one despawn.
pub fn entity_delta(moving: u32) -> EntityDelta {
    EntityDelta {
        snapshot: 5021,
        baseline: Some(5019),
        tick: 100_400,
        ack_input: 1_000_003,
        spawns: vec![EntitySpawn {
            id: NetEntityId(900),
            kind: 1,
            pos: FixedPos::from_blocks([120.25, 64.0, -40.5]),
            yaw: Angle8(17),
            pitch: Angle8(0),
        }],
        updates: (0..u64::from(moving))
            .map(|i| {
                let r = noise(i + 1000);
                let axis = |bits: u64| ((bits % 29) as i16) - 14;
                EntityUpdate {
                    id: NetEntityId((noise(i) % 5000) as u32),
                    pos: Some(PosUpdate::Delta(PosDelta {
                        dx: axis(r),
                        dy: if r & (1 << 40) == 0 {
                            0
                        } else {
                            axis(r >> 20) / 2
                        },
                        dz: axis(r >> 8),
                    })),
                    yaw: Some(Angle8((r >> 48) as u8)),
                    pitch: None,
                    on_ground: r & (1 << 41) == 0,
                }
            })
            .collect(),
        despawns: vec![NetEntityId(4)],
    }
}
