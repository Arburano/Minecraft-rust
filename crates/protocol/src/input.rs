//! The input channel: player intents with tick numbers, sent as datagrams.
//!
//! Each datagram repeats the last few intents (newest last), so a lost
//! datagram is covered by the next one without retransmission
//! (`networking-protocol.md`: "duplicated in 2–3 datagrams against loss").

use bitcode::{Decode, Encode};

use crate::{Angle16, MAX_DATAGRAM, Message, ProtocolError, check};

/// Most intents in one datagram.
pub const MAX_INTENTS: usize = 8;

/// Held buttons and one-tick events of an intent, as bits.
#[derive(Encode, Decode, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Buttons(pub u16);

impl Buttons {
    /// Move forward.
    pub const FORWARD: u16 = 1 << 0;
    /// Move back.
    pub const BACK: u16 = 1 << 1;
    /// Strafe left.
    pub const LEFT: u16 = 1 << 2;
    /// Strafe right.
    pub const RIGHT: u16 = 1 << 3;
    /// Jump / fly up / swim up.
    pub const JUMP: u16 = 1 << 4;
    /// Sneak / fly down.
    pub const SNEAK: u16 = 1 << 5;
    /// Sprint.
    pub const SPRINT: u16 = 1 << 6;
    /// One-tick event: toggle flight (the double-tapped jump, `simulation-and-ai.md`).
    pub const FLY_TOGGLE: u16 = 1 << 7;
    /// Bits defined in this protocol version; others must be zero.
    pub const ALL: u16 = (1 << 8) - 1;

    /// Whether every bit of `mask` is set.
    #[must_use]
    pub fn has(self, mask: u16) -> bool {
        self.0 & mask == mask
    }
}

/// What the player did in one client tick.
#[derive(Encode, Decode, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Intent {
    /// The client tick this intent belongs to; consecutive intents differ by 1.
    pub tick: u32,
    /// Buttons and events.
    pub buttons: Buttons,
    /// View yaw.
    pub yaw: Angle16,
    /// View pitch (read with `to_signed_radians`).
    pub pitch: Angle16,
    /// Mouse-wheel notches while flying: the fly speed multiplier steps
    /// (`simulation-and-ai.md`, flight).
    pub fly_speed_steps: i8,
}

/// One input datagram.
#[derive(Encode, Decode, Clone, Debug, Default, PartialEq, Eq)]
pub struct InputDatagram {
    /// The newest entity snapshot the client received complete (0 = none):
    /// the server sends deltas relative to it (`EntityDatagram::baseline`).
    pub ack_snapshot: u32,
    /// The last intents, oldest first, consecutive ticks.
    pub intents: Vec<Intent>,
}

impl Message for InputDatagram {
    const MAX_LEN: usize = MAX_DATAGRAM;

    fn validate(&self) -> Result<(), ProtocolError> {
        check(
            (1..=MAX_INTENTS).contains(&self.intents.len()),
            "intent count",
        )?;
        check(
            self.intents
                .windows(2)
                .all(|w| w[0].tick.checked_add(1) == Some(w[1].tick)),
            "intent ticks must be consecutive",
        )?;
        check(
            self.intents
                .iter()
                .all(|i| i.buttons.0 & !Buttons::ALL == 0),
            "unknown button bits",
        )
    }
}
