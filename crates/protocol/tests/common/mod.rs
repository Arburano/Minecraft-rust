//! Sample messages shared by the protocol tests.
#![allow(dead_code)]

use protocol::*;
use world::storage::ColumnRecord;
use world::{BlockStateId, ColumnPos, Section, WorldBounds};

pub fn hello() -> ClientControl {
    ClientControl::Hello(Hello {
        client_version: "0.0.1".into(),
        player_name: "Alex_42".into(),
        session_jwt: None,
    })
}

pub fn welcome() -> ServerControl {
    ServerControl::Welcome(Welcome {
        server_name: "Test server".into(),
        motd: "Hello there".into(),
        server_version: "0.0.1".into(),
        player_id: [7; 16],
        tick_rate: 20,
        mod_manifest_hash: [0; 32],
    })
}

pub fn registry() -> ServerControl {
    ServerControl::Registry(RegistrySync {
        block_states: vec![
            "core:air".into(),
            "core:missing".into(),
            "core:stone".into(),
            "core:log[axis=x]".into(),
            "core:log[axis=y]".into(),
        ],
        component_schema_hash: [0; 32],
    })
}

pub fn spawn() -> ServerControl {
    ServerControl::Spawn(Spawn {
        tick: 1234,
        position: [8.5, 72.0, -3.25],
        yaw: 1.5,
        pitch: -0.25,
        min_section_y: -4,
        section_count: 24,
        flying: false,
        fly_speed: 1.0,
        view_distance: 12,
    })
}

pub fn kick() -> ServerControl {
    ServerControl::Kick {
        reason: KickReason::ServerFull,
        message: "The server is full".into(),
    }
}

pub fn input(n: usize) -> InputDatagram {
    InputDatagram {
        ack_snapshot: 4000,
        intents: (0..n as u32)
            .map(|i| Intent {
                tick: 10_000 + i,
                buttons: Buttons(
                    Buttons::FORWARD | Buttons::SPRINT | ((i as u16 & 1) * Buttons::JUMP),
                ),
                yaw: Angle16(12_345u16.wrapping_add(i as u16 * 77)),
                pitch: Angle16::from_radians(-0.3),
                fly_speed_steps: 0,
                state_hash: 0x9e37_79b9u32.wrapping_mul(i + 1),
            })
            .collect(),
    }
}

/// `n` entities appearing (kind, absolute position, look, flags): the
/// largest form, sent once per entity.
pub fn entities(n: usize) -> EntityDatagram {
    let origin = BlockXyz {
        x: 1000,
        y: 64,
        z: -2000,
    };
    EntityDatagram {
        snapshot: 4001,
        baseline: 4000,
        ack_tick: Some(9_990),
        correction: None,
        origin,
        updates: (0..n as u64)
            .map(|i| EntityUpdate {
                id: NetId(100_000 + i * 3),
                kind: Some(0),
                pos: RelPos::quantize(
                    [
                        1000.0 + (i as f64 * 7.31) % 90.0 - 45.0,
                        64.0 + (i as f64 * 1.7) % 10.0,
                        -2000.0 + (i as f64 * 13.9) % 90.0 - 45.0,
                    ],
                    origin,
                ),
                moved: None,
                look: Some(Look {
                    yaw: Angle8((i * 37) as u8),
                    pitch: Angle8((i * 11) as u8),
                    head_yaw: Angle8((i * 41) as u8),
                }),
                flags: Some(1),
            })
            .collect(),
        removed: vec![NetId(5)],
    }
}

/// `n` known entities moving and turning (the steady state at 20 Hz:
/// ≈ 0.2–0.3 blocks per tick, i.e. a step of up to ±10 in 1/32 block).
pub fn moving(n: usize) -> EntityDatagram {
    EntityDatagram {
        snapshot: 4001,
        baseline: 4000,
        ack_tick: Some(9_990),
        correction: None,
        origin: BlockXyz {
            x: 1000,
            y: 64,
            z: -2000,
        },
        updates: (0..n as u64)
            .map(|i| EntityUpdate {
                id: NetId(100_000 + i * 3),
                kind: None,
                pos: None,
                moved: Some(Step8 {
                    x: ((i * 7) % 21) as i8 - 10,
                    y: 0,
                    z: ((i * 5) % 21) as i8 - 10,
                }),
                look: Some(Look {
                    yaw: Angle8((i * 37) as u8),
                    pitch: Angle8((i * 11) as u8),
                    head_yaw: Angle8((i * 41) as u8),
                }),
                flags: None,
            })
            .collect(),
        removed: Vec::new(),
    }
}

/// A datagram with a correction (the rare case).
pub fn corrected() -> EntityDatagram {
    EntityDatagram {
        correction: Some(Correction {
            tick: 9_990,
            pos: [1000.123, 70.0, -1999.5],
            vel: [0.1, -0.0784, 0.0],
            flags: EntityUpdate::ON_GROUND,
            fly_speed: 1.0,
        }),
        ..moving(2)
    }
}

pub fn client_events() -> Vec<ClientEvent> {
    vec![
        ClientEvent::Chat {
            text: "hi all".into(),
        },
        ClientEvent::Dig {
            seq: 1,
            pos: BlockXyz { x: 1, y: -5, z: 3 },
        },
        ClientEvent::Place {
            seq: 2,
            pos: BlockXyz { x: 1, y: 70, z: 3 },
            state: 2,
        },
    ]
}

pub fn server_events() -> Vec<ServerEvent> {
    vec![
        ServerEvent::Chat {
            from: "Alex_42".into(),
            text: "hi all".into(),
        },
        ServerEvent::BlockChanges(vec![
            BlockChange {
                pos: BlockXyz { x: 1, y: -5, z: 3 },
                state: 0,
            },
            BlockChange {
                pos: BlockXyz { x: 2, y: 70, z: 3 },
                state: 4,
            },
        ]),
        ServerEvent::BlockAck {
            seq: 2,
            accepted: false,
        },
        ServerEvent::ForgetColumn(ColumnXz { x: -7, z: 9 }),
    ]
}

/// A small hand-made column: stone below 0, a mixed section, air above.
pub fn small_record() -> ColumnRecord {
    let bounds = WorldBounds {
        min_section_y: -4,
        section_count: 8,
    };
    let mut sections = Vec::new();
    for sy in -4..4 {
        let s = match sy {
            ..0 => Section::uniform(BlockStateId(2)),
            0 => {
                let mut s = Section::uniform(BlockStateId::AIR);
                for i in 0..4096 {
                    if i % 7 == 0 || i < 256 {
                        s.set(i, BlockStateId(2 + (i % 3) as u16));
                    }
                }
                s
            }
            _ => Section::new(),
        };
        sections.push(s);
    }
    ColumnRecord {
        pos: ColumnPos { x: -3, z: 12 },
        bounds,
        sections,
        light: Vec::new(),
        entities: Vec::new(),
    }
}
