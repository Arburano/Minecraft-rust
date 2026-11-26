//! The control channel: the fixed handshake header, then login, ID table,
//! spawn and kick (`networking-protocol.md`, "Handshake").
//!
//! ```text
//! client ─ HandshakeHeader ─► server ─ HandshakeHeader ─► client   (fixed 8 bytes each)
//! client ─ Hello ──────────────────────────────────────► server
//! server ─ Welcome ────────────────────────────────────► client
//! server ─ RegistrySync ───────────────────────────────► client
//! server ─ Spawn ──────────────────────────────────────► client   (player control)
//! ```
//!
//! On a version mismatch the server still answers with its header and closes
//! the connection; both sides report [`ProtocolError::VersionMismatch`].

use bitcode::{Decode, Encode};
use world::WorldBounds;

use crate::frame::MAX_FRAME;
use crate::{MAX_DATAGRAM, Message, PROTOCOL_VERSION, ProtocolError, check, check_text};

/// Longest player, server or version name, in bytes.
pub const MAX_NAME_LEN: usize = 64;
/// Longest free text (MOTD, kick message), in bytes.
pub const MAX_TEXT_LEN: usize = 1024;
/// Longest session token, in bytes (a JWT, reserved until T0-10).
const MAX_JWT_LEN: usize = 4096;
/// Longest block-state name in the ID table, in bytes.
const MAX_STATE_NAME_LEN: usize = 256;
/// Most block states a registry can hold (`world-storage.md`).
const MAX_STATES: usize = 1 << 16;

/// The first 8 bytes each side sends on the control stream: a magic and the
/// protocol version, little-endian. This layout never changes, so builds of
/// any version can tell each other apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HandshakeHeader {
    /// The sender's protocol version.
    pub protocol: u32,
}

impl HandshakeHeader {
    /// Magic bytes of the protocol.
    pub const MAGIC: [u8; 4] = *b"VXEN";
    /// Encoded length.
    pub const LEN: usize = 8;

    /// The header of this build.
    #[must_use]
    pub fn current() -> Self {
        Self {
            protocol: PROTOCOL_VERSION,
        }
    }

    /// Encodes the header.
    #[must_use]
    pub fn encode(self) -> [u8; Self::LEN] {
        let mut out = [0u8; Self::LEN];
        out[..4].copy_from_slice(&Self::MAGIC);
        out[4..].copy_from_slice(&self.protocol.to_le_bytes());
        out
    }

    /// Decodes a header from the first [`Self::LEN`] bytes of `bytes`.
    pub fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        let b = bytes.get(..Self::LEN).ok_or(ProtocolError::Truncated)?;
        if b[..4] != Self::MAGIC {
            return Err(ProtocolError::BadMagic);
        }
        Ok(Self {
            protocol: u32::from_le_bytes([b[4], b[5], b[6], b[7]]),
        })
    }

    /// `Ok` if the peer speaks this build's version.
    pub fn check(self) -> Result<(), ProtocolError> {
        if self.protocol == PROTOCOL_VERSION {
            Ok(())
        } else {
            Err(ProtocolError::VersionMismatch {
                local: PROTOCOL_VERSION,
                remote: self.protocol,
            })
        }
    }
}

/// The client's login request.
#[derive(Encode, Decode, Clone, Debug, PartialEq, Eq)]
pub struct Hello {
    /// The client build, e.g. `0.0.1`, for logs and the server list.
    pub client_version: String,
    /// The player name. Offline mode only until T0-10; the server validates it
    /// (`auth::PlayerName`) and derives the player ID.
    pub player_name: String,
    /// The session token from the login service. Reserved: always `None`
    /// until Microsoft login exists (T0-10, `legal.md` rule 7).
    pub session_jwt: Option<String>,
}

/// The server's answer to an accepted [`Hello`].
#[derive(Encode, Decode, Clone, Debug, PartialEq, Eq)]
pub struct Welcome {
    /// Server display name.
    pub server_name: String,
    /// Message of the day.
    pub motd: String,
    /// The server build.
    pub server_version: String,
    /// The player's stable ID (UUID layout, `auth::PlayerId`).
    pub player_id: [u8; 16],
    /// Simulation ticks per second (20).
    pub tick_rate: u8,
    /// SHA-256 of the server's mod manifest; the client downloads mods if its
    /// own differs (stage 3). All zeros until mod sync exists.
    pub mod_manifest_hash: [u8; 32],
}

/// The server's block ID table: index = numeric state ID on this connection.
/// The client maps it to its own registry with `Registry::remap`; states it
/// does not know become `core:missing`.
#[derive(Encode, Decode, Clone, Debug, PartialEq, Eq)]
pub struct RegistrySync {
    /// Canonical state names, `ns:path[a=x,b=y]`.
    pub block_states: Vec<String>,
    /// Hash of the mod component schemas (stage 3); all zeros until then.
    pub component_schema_hash: [u8; 32],
}

/// Puts the player into the world.
#[derive(Encode, Decode, Clone, Debug, PartialEq)]
pub struct Spawn {
    /// The server tick at which the position is valid.
    pub tick: u64,
    /// Feet position (exact; later movement is quantized).
    pub position: [f64; 3],
    /// View yaw, radians.
    pub yaw: f32,
    /// View pitch, radians.
    pub pitch: f32,
    /// Lowest section y of the dimension.
    pub min_section_y: i32,
    /// Sections per column.
    pub section_count: u32,
    /// Whether the player starts flying.
    pub flying: bool,
    /// Column radius the server streams (≥ 8, PRD §14).
    pub view_distance: u8,
}

impl Spawn {
    /// The world's vertical bounds.
    #[must_use]
    pub fn bounds(&self) -> WorldBounds {
        WorldBounds {
            min_section_y: self.min_section_y,
            section_count: self.section_count,
        }
    }
}

/// Why the server ends the session.
#[derive(Encode, Decode, Clone, Copy, Debug, PartialEq, Eq)]
pub enum KickReason {
    /// The server is shutting down.
    ShuttingDown,
    /// The server is full.
    ServerFull,
    /// The name is invalid or already in use.
    BadName,
    /// The player was removed by an operator.
    Kicked,
    /// The client broke the protocol.
    ProtocolViolation,
    /// Login is required but offline mode was used.
    LoginRequired,
}

/// Client → server on the control stream.
#[derive(Encode, Decode, Clone, Debug, PartialEq, Eq)]
pub enum ClientControl {
    /// Login request.
    Hello(Hello),
    /// The client leaves on purpose (closing the connection also works).
    Leave,
}

/// Server → client on the control stream.
#[derive(Encode, Decode, Clone, Debug, PartialEq)]
pub enum ServerControl {
    /// Login accepted.
    Welcome(Welcome),
    /// The block ID table.
    Registry(RegistrySync),
    /// Player control starts.
    Spawn(Spawn),
    /// The session ends; `message` is shown to the player.
    Kick {
        /// Machine-readable reason.
        reason: KickReason,
        /// Text for the player.
        message: String,
    },
}

impl Message for ClientControl {
    const MAX_LEN: usize = 8 * 1024;

    fn validate(&self) -> Result<(), ProtocolError> {
        match self {
            Self::Hello(h) => {
                check_text(&h.client_version, MAX_NAME_LEN, "client version")?;
                check_text(&h.player_name, MAX_NAME_LEN, "player name")?;
                if let Some(jwt) = &h.session_jwt {
                    check_text(jwt, MAX_JWT_LEN, "session token")?;
                }
                Ok(())
            }
            Self::Leave => Ok(()),
        }
    }
}

impl Message for ServerControl {
    const MAX_LEN: usize = MAX_FRAME;

    fn validate(&self) -> Result<(), ProtocolError> {
        match self {
            Self::Welcome(w) => {
                check_text(&w.server_name, MAX_NAME_LEN, "server name")?;
                check_text(&w.server_version, MAX_NAME_LEN, "server version")?;
                check(w.motd.len() <= MAX_TEXT_LEN, "motd")?;
                check(w.tick_rate > 0, "tick rate")
            }
            Self::Registry(r) => {
                check(r.block_states.len() <= MAX_STATES, "too many block states")?;
                for s in &r.block_states {
                    check_text(s, MAX_STATE_NAME_LEN, "block state name")?;
                }
                Ok(())
            }
            Self::Spawn(s) => {
                check(s.position.iter().all(|v| v.is_finite()), "spawn position")?;
                check(s.yaw.is_finite() && s.pitch.is_finite(), "spawn angles")?;
                check(
                    (1..=4096).contains(&s.section_count)
                        && s.min_section_y
                            .checked_add(s.section_count as i32)
                            .is_some_and(|top| top <= (1 << 20)),
                    "world bounds",
                )?;
                check(s.view_distance >= 2, "view distance")
            }
            Self::Kick { message, .. } => check(message.len() <= MAX_TEXT_LEN, "kick message"),
        }
    }
}

// The input datagram must fit a datagram; a compile-time reminder for whoever
// changes the constants.
const _: () = assert!(MAX_DATAGRAM <= 1200);
