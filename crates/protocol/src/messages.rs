//! Messages after the handshake, by channel (networking-protocol.md,
//! "Transport").

use crate::codec::{self, DecodeError, EncodeError};
use crate::handshake::{Registry, Spawn};
use crate::quant::{Angle8, Angle16, FixedPos, PosDelta};

/// Longest chat or kick text, in bytes.
pub const MAX_CHAT_BYTES: usize = 512;

/// Most input frames in one datagram. The client sends the last 2–3 ticks
/// in every datagram, so a lost datagram costs nothing.
pub const MAX_INPUT_FRAMES: usize = 8;

// ---------------------------------------------------------------- control --

/// Client → server on the control stream, after `Hello`.
#[derive(Clone, Debug, PartialEq, Eq, bitcode::Encode, bitcode::Decode)]
#[non_exhaustive]
pub enum ClientControl {
    /// The client applied `Registry` (and downloaded mods if needed) and is
    /// ready for `Spawn` and columns.
    Ready,
    /// The client leaves.
    Disconnect,
}

/// Server → client on the control stream, after `ServerHello`.
#[derive(Clone, Debug, PartialEq, Eq, bitcode::Encode, bitcode::Decode)]
#[non_exhaustive]
pub enum ServerControl {
    /// ID tables; sent once, right after `Welcome`.
    Registry(Registry),
    /// The player is placed; sent once the client is `Ready`.
    Spawn(Spawn),
    /// The server drops the client, with a reason shown to the player.
    Kick(String),
}

impl ClientControl {
    /// Encodes one control-stream frame.
    pub fn encode_frame(&self) -> Result<Vec<u8>, EncodeError> {
        codec::encode_frame(self)
    }

    /// Decodes a control-stream frame body.
    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        codec::decode(bytes)
    }
}

impl ServerControl {
    /// Encodes one control-stream frame.
    pub fn encode_frame(&self) -> Result<Vec<u8>, EncodeError> {
        codec::encode_frame(self)
    }

    /// Decodes a control-stream frame body.
    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let msg: Self = codec::decode(bytes)?;
        if let Self::Kick(text) = &msg
            && text.len() > MAX_CHAT_BYTES
        {
            return Err(DecodeError::Invalid("kick text too long"));
        }
        Ok(msg)
    }
}

// ------------------------------------------------------------------ input --

/// Button bits of [`InputFrame::buttons`].
pub mod buttons {
    /// Jump held.
    pub const JUMP: u8 = 1 << 0;
    /// Sprint held.
    pub const SPRINT: u8 = 1 << 1;
    /// Descend held (flying: move down; sneak later).
    pub const DESCEND: u8 = 1 << 2;
    /// Toggle flight this tick (double-tapped jump).
    pub const TOGGLE_FLY: u8 = 1 << 3;
}

/// One tick of player intent, quantized. Both the client's prediction and
/// the server simulate the dequantized values, so they agree exactly.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, bitcode::Encode, bitcode::Decode)]
pub struct InputFrame {
    /// Client tick this input is for.
    pub tick: u32,
    /// [`buttons`] bits.
    pub buttons: u8,
    /// Forward (+127) / backward (−127), see `quant::axis_to_i8`.
    pub forward: i8,
    /// Right (+127) / left (−127).
    pub strafe: i8,
    /// Fly speed multiplier in 1/16 (16 = ×1); 0 means ×1.
    pub fly_speed: u8,
    /// View yaw.
    pub yaw: Angle16,
    /// View pitch.
    pub pitch: Angle16,
}

/// Client → server datagram on the input channel.
#[derive(Clone, Debug, Default, PartialEq, Eq, bitcode::Encode, bitcode::Decode)]
pub struct InputDatagram {
    /// The newest entity snapshot the client has applied; the server sends
    /// deltas against it.
    pub ack_snapshot: u32,
    /// The last few ticks of input, oldest first (duplicates across
    /// datagrams against loss; the server ignores ticks it has seen).
    pub frames: Vec<InputFrame>,
}

impl InputDatagram {
    /// Encodes the datagram (≤ `MAX_DATAGRAM_BYTES`).
    pub fn encode(&self) -> Result<Vec<u8>, EncodeError> {
        codec::encode_datagram(self)
    }

    /// Decodes a datagram. Never panics.
    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let d: Self = codec::decode_datagram(bytes)?;
        if d.frames.len() > MAX_INPUT_FRAMES {
            return Err(DecodeError::Invalid("too many input frames"));
        }
        Ok(d)
    }
}

// --------------------------------------------------------------- entities --

/// A replicated entity's network ID (per connection, not persistent).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, bitcode::Encode, bitcode::Decode)]
pub struct NetEntityId(pub u32);

/// An entity appears for this client.
#[derive(Clone, Copy, Debug, PartialEq, Eq, bitcode::Encode, bitcode::Decode)]
pub struct EntitySpawn {
    /// Network ID.
    pub id: NetEntityId,
    /// Entity kind, an index into `Registry::entity_kinds`.
    pub kind: u16,
    /// Position.
    pub pos: FixedPos,
    /// Body yaw.
    pub yaw: Angle8,
    /// Head pitch.
    pub pitch: Angle8,
}

/// How a position changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, bitcode::Encode, bitcode::Decode)]
pub enum PosUpdate {
    /// Moved relative to the baseline snapshot.
    Delta(PosDelta),
    /// Moved too far for a delta (teleport).
    Absolute(FixedPos),
}

/// Changed fields of an entity relative to the baseline snapshot; `None`
/// means unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, bitcode::Encode, bitcode::Decode)]
pub struct EntityUpdate {
    /// Network ID.
    pub id: NetEntityId,
    /// New position.
    pub pos: Option<PosUpdate>,
    /// New body yaw.
    pub yaw: Option<Angle8>,
    /// New head pitch.
    pub pitch: Option<Angle8>,
    /// Standing on the ground (for animation).
    pub on_ground: bool,
}

/// Server → client datagram on the entities channel: one snapshot as a delta
/// against the snapshot the client acknowledged.
#[derive(Clone, Debug, Default, PartialEq, Eq, bitcode::Encode, bitcode::Decode)]
pub struct EntityDelta {
    /// This snapshot's number (increasing).
    pub snapshot: u32,
    /// The snapshot this is a delta against; `None` = relative to nothing
    /// (all visible entities are in `spawns`).
    pub baseline: Option<u32>,
    /// Server tick of the snapshot.
    pub tick: u32,
    /// The newest client input tick the server has applied (reconciliation).
    pub ack_input: u32,
    /// Entities that became visible.
    pub spawns: Vec<EntitySpawn>,
    /// Entities that changed.
    pub updates: Vec<EntityUpdate>,
    /// Entities that are gone.
    pub despawns: Vec<NetEntityId>,
}

impl EntityDelta {
    /// Encodes the datagram; fails if it is over `MAX_DATAGRAM_BYTES` (the
    /// server then splits the snapshot over several datagrams).
    pub fn encode(&self) -> Result<Vec<u8>, EncodeError> {
        codec::encode_datagram(self)
    }

    /// Decodes a datagram. Never panics.
    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        codec::decode_datagram(bytes)
    }
}

// ----------------------------------------------------------------- events --

/// Client → server on the events stream.
#[derive(Clone, Debug, PartialEq, Eq, bitcode::Encode, bitcode::Decode)]
#[non_exhaustive]
pub enum ClientEvent {
    /// A chat line typed by the player.
    Chat(String),
}

/// Server → client on the events stream.
#[derive(Clone, Debug, PartialEq, Eq, bitcode::Encode, bitcode::Decode)]
#[non_exhaustive]
pub enum ServerEvent {
    /// A chat line from a player.
    Chat {
        /// Sender's name.
        from: String,
        /// Text.
        text: String,
    },
    /// A message from the server itself.
    System(String),
}

impl ClientEvent {
    /// Encodes one events-stream frame.
    pub fn encode_frame(&self) -> Result<Vec<u8>, EncodeError> {
        codec::encode_frame(self)
    }

    /// Decodes an events-stream frame body.
    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let msg: Self = codec::decode(bytes)?;
        let Self::Chat(text) = &msg;
        if text.len() > MAX_CHAT_BYTES {
            return Err(DecodeError::Invalid("chat text too long"));
        }
        Ok(msg)
    }
}

impl ServerEvent {
    /// Encodes one events-stream frame.
    pub fn encode_frame(&self) -> Result<Vec<u8>, EncodeError> {
        codec::encode_frame(self)
    }

    /// Decodes an events-stream frame body.
    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let msg: Self = codec::decode(bytes)?;
        let too_long = match &msg {
            Self::Chat { from, text } => from.len() > 64 || text.len() > MAX_CHAT_BYTES,
            Self::System(text) => text.len() > MAX_CHAT_BYTES,
        };
        if too_long {
            return Err(DecodeError::Invalid("event text too long"));
        }
        Ok(msg)
    }
}
