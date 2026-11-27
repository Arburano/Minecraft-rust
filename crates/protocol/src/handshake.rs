//! Handshake on the control stream (networking-protocol.md, "Handshake").
//!
//! ```text
//! client ─ Hello { client_version, identity, session_jwt } ─► server
//! server ─ ServerHello::Accepted(Welcome) | Rejected(..) ──► client
//! server ─ ServerControl::Registry { id_table, .. } ───────► client
//! server ─ ServerControl::Spawn { pos, tick } ─────────────► client
//! ```
//!
//! The first frame in each direction starts with a fixed 8-byte prefix,
//! [`HANDSHAKE_MAGIC`] + [`PROTOCOL_VERSION`] (`u32` LE), followed by the
//! `bitcode` body. The prefix layout never changes, so peers of different
//! versions always detect the mismatch and report it with an update hint
//! instead of failing to decode. On a mismatch the server answers with the
//! prefix alone (empty body): the body layout of another version is unknown.
//!
//! Identity is offline only until T0-10 is answered (open question 4);
//! `session_jwt` is reserved and must be `None`.

use std::fmt;

use crate::PROTOCOL_VERSION;
use crate::codec::DecodeError;
use crate::quant::{Angle16, FixedPos};

/// The first four bytes of a handshake frame body.
pub const HANDSHAKE_MAGIC: [u8; 4] = *b"VXLH";

/// Bytes of the version prefix: magic + `u32` version.
pub const PREFIX_BYTES: usize = 8;

/// Longest accepted `client_version` / server name / MOTD string, in bytes.
pub const MAX_TEXT_BYTES: usize = 256;

/// Who the client says it is. The server checks it with `auth`
/// (`AdmissionPolicy`); the protocol only carries it.
#[derive(Clone, Debug, PartialEq, Eq, bitcode::Encode, bitcode::Decode)]
#[non_exhaustive]
pub enum ClaimedIdentity {
    /// Offline (development) identity: a name, no ownership check. The server
    /// derives the player ID from it (`auth::PlayerId::offline`).
    Offline {
        /// Player name as typed.
        name: String,
    },
}

/// The client's first message.
#[derive(Clone, Debug, PartialEq, Eq, bitcode::Encode, bitcode::Decode)]
pub struct Hello {
    /// Client build version, e.g. `0.0.1`; shown in server logs.
    pub client_version: String,
    /// Claimed identity.
    pub identity: ClaimedIdentity,
    /// Session token from the login service. Reserved: always `None` until
    /// T0-10 (open question 4); a server rejects `Some` for now.
    // OPEN-QUESTION: 4
    pub session_jwt: Option<String>,
}

/// Server description shown in the client and used to configure it.
#[derive(Clone, Debug, PartialEq, Eq, bitcode::Encode, bitcode::Decode)]
pub struct ServerInfo {
    /// Server name.
    pub name: String,
    /// Message of the day.
    pub motd: String,
    /// Simulation ticks per second.
    pub tick_rate: u16,
    /// View distance the server streams, in columns.
    pub view_distance: u8,
}

/// The server accepts the client.
#[derive(Clone, Debug, PartialEq, Eq, bitcode::Encode, bitcode::Decode)]
pub struct Welcome {
    /// Server description.
    pub server_info: ServerInfo,
    /// The player's ID as the server sees it (offline: derived from the name).
    pub player_id: [u8; 16],
    /// SHA-256 of the server's mod manifest. If it differs from the client's,
    /// the client downloads the mods through the menu
    /// (client-menu-and-updater.md) before continuing.
    pub mod_manifest_hash: [u8; 32],
}

/// Why the server refuses a client (other than a version mismatch, which is
/// detected from the prefix).
#[derive(Clone, Debug, PartialEq, Eq, bitcode::Encode, bitcode::Decode)]
#[non_exhaustive]
pub enum Rejection {
    /// The server does not admit offline identities (legal.md rule 7).
    OfflineNotAllowed,
    /// The name is not 3..=16 characters of `A-Z a-z 0-9 _`.
    InvalidName,
    /// A session token was sent but sessions are not supported yet.
    SessionNotSupported,
    /// A player with this ID is already connected.
    AlreadyConnected,
    /// The server is full.
    ServerFull,
    /// Another reason, as text.
    Other(String),
}

impl fmt::Display for Rejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OfflineNotAllowed => f.write_str("this server does not allow offline players"),
            Self::InvalidName => {
                f.write_str("invalid player name: use 3–16 characters A–Z, a–z, 0–9 or _")
            }
            Self::SessionNotSupported => f.write_str("online sessions are not supported yet"),
            Self::AlreadyConnected => f.write_str("this player is already connected"),
            Self::ServerFull => f.write_str("the server is full"),
            Self::Other(text) => f.write_str(text),
        }
    }
}

/// The server's first message.
#[derive(Clone, Debug, PartialEq, Eq, bitcode::Encode, bitcode::Decode)]
pub enum ServerHello {
    /// Joined; `Registry` and `Spawn` follow on the control stream.
    Accepted(Welcome),
    /// Refused; the server closes the connection.
    Rejected(Rejection),
}

/// Replicated component schema of a mod (stage 3; empty until then).
#[derive(Clone, Debug, PartialEq, Eq, bitcode::Encode, bitcode::Decode)]
pub struct ComponentSchema {
    /// Component name, `mod:component`.
    pub name: String,
    /// Hash of the schema from the mod manifest.
    pub schema_hash: [u8; 32],
}

/// ID tables the client needs to read the server's numbers.
#[derive(Clone, Debug, PartialEq, Eq, bitcode::Encode, bitcode::Decode)]
pub struct Registry {
    /// Block state names by numeric ID (`world::IdTable`); column sections are
    /// in these IDs. The client remaps them with `world::Registry::remap`.
    pub id_table: Vec<String>,
    /// Entity kind names by numeric ID (`EntitySpawn::kind`).
    pub entity_kinds: Vec<String>,
    /// Replicated mod component schemas (stage 3; empty until then).
    pub component_schemas: Vec<ComponentSchema>,
}

/// The player is placed in the world; the client takes control once the
/// nearest columns arrive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, bitcode::Encode, bitcode::Decode)]
pub struct Spawn {
    /// Feet position.
    pub pos: FixedPos,
    /// View yaw.
    pub yaw: Angle16,
    /// View pitch.
    pub pitch: Angle16,
    /// Server tick at spawn; the client's input ticks continue from it.
    pub tick: u32,
    /// The player's own entity ID in entity deltas.
    pub entity: u32,
    /// Lowest section Y of the world (`world::WorldBounds`).
    pub min_section_y: i32,
    /// Sections per column.
    pub section_count: u32,
}

/// Handshake failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HandshakeError {
    /// The peer speaks another protocol version.
    VersionMismatch {
        /// Our [`PROTOCOL_VERSION`].
        ours: u32,
        /// The peer's version.
        theirs: u32,
    },
    /// The frame does not start with [`HANDSHAKE_MAGIC`]: not our protocol.
    NotOurProtocol,
    /// The body does not decode.
    Decode(DecodeError),
    /// The server refused the client.
    Rejected(Rejection),
}

impl fmt::Display for HandshakeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::VersionMismatch { ours, theirs } if theirs > ours => write!(
                f,
                "protocol version mismatch: the other side speaks version {theirs}, this game \
                 speaks {ours}; update the game (main menu → Update)"
            ),
            Self::VersionMismatch { ours, theirs } => write!(
                f,
                "protocol version mismatch: the other side speaks older version {theirs}, this \
                 game speaks {ours}; the other side needs an update"
            ),
            Self::NotOurProtocol => f.write_str("the peer does not speak this game's protocol"),
            Self::Decode(e) => write!(f, "bad handshake: {e}"),
            Self::Rejected(r) => write!(f, "the server refused to join: {r}"),
        }
    }
}

impl std::error::Error for HandshakeError {}

fn prefixed(version: u32, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(PREFIX_BYTES + body.len());
    out.extend_from_slice(&HANDSHAKE_MAGIC);
    out.extend_from_slice(&version.to_le_bytes());
    out.extend_from_slice(body);
    out
}

/// Checks the prefix and returns the body.
fn check_prefix(bytes: &[u8]) -> Result<&[u8], HandshakeError> {
    let (Some(magic), Some(version)) = (
        bytes.first_chunk::<4>(),
        bytes
            .get(4..PREFIX_BYTES)
            .and_then(|v| v.first_chunk::<4>()),
    ) else {
        return Err(HandshakeError::NotOurProtocol);
    };
    if *magic != HANDSHAKE_MAGIC {
        return Err(HandshakeError::NotOurProtocol);
    }
    let theirs = u32::from_le_bytes(*version);
    if theirs != PROTOCOL_VERSION {
        return Err(HandshakeError::VersionMismatch {
            ours: PROTOCOL_VERSION,
            theirs,
        });
    }
    Ok(&bytes[PREFIX_BYTES..])
}

fn text_ok(s: &str) -> bool {
    s.len() <= MAX_TEXT_BYTES
}

impl Hello {
    /// Encodes the frame body: prefix + `bitcode`.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        prefixed(PROTOCOL_VERSION, &bitcode::encode(self))
    }

    /// Decodes a frame body (server side). A version mismatch is reported
    /// before the body is looked at.
    pub fn decode(bytes: &[u8]) -> Result<Self, HandshakeError> {
        let body = check_prefix(bytes)?;
        let hello: Self = crate::codec::decode(body).map_err(HandshakeError::Decode)?;
        let ClaimedIdentity::Offline { name } = &hello.identity;
        let jwt_ok = hello.session_jwt.as_deref().is_none_or(|j| j.len() <= 8192);
        if !text_ok(&hello.client_version) || !text_ok(name) || !jwt_ok {
            return Err(HandshakeError::Decode(DecodeError::Invalid(
                "handshake text too long",
            )));
        }
        Ok(hello)
    }
}

impl ServerHello {
    /// Encodes the frame body: prefix + `bitcode`.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        prefixed(PROTOCOL_VERSION, &bitcode::encode(self))
    }

    /// The server's answer to a client of another version: the prefix alone.
    #[must_use]
    pub fn encode_version_mismatch() -> Vec<u8> {
        prefixed(PROTOCOL_VERSION, &[])
    }

    /// Decodes a frame body (client side). Returns the `Welcome`, or the
    /// reason the client cannot join.
    pub fn decode(bytes: &[u8]) -> Result<Welcome, HandshakeError> {
        let body = check_prefix(bytes)?;
        match crate::codec::decode::<Self>(body).map_err(HandshakeError::Decode)? {
            Self::Accepted(w) => {
                if !text_ok(&w.server_info.name) || !text_ok(&w.server_info.motd) {
                    return Err(HandshakeError::Decode(DecodeError::Invalid(
                        "handshake text too long",
                    )));
                }
                Ok(w)
            }
            Self::Rejected(r) => Err(HandshakeError::Rejected(r)),
        }
    }
}

/// Builds a handshake frame body with an arbitrary version prefix; for tests
/// of version mismatches.
#[doc(hidden)]
#[must_use]
pub fn encode_with_version(version: u32, body: &[u8]) -> Vec<u8> {
    prefixed(version, body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hello() -> Hello {
        Hello {
            client_version: "0.0.1".into(),
            identity: ClaimedIdentity::Offline {
                name: "Alex".into(),
            },
            session_jwt: None,
        }
    }

    #[test]
    fn hello_roundtrip() {
        assert_eq!(Hello::decode(&hello().encode()), Ok(hello()));
    }

    #[test]
    fn version_mismatch_is_reported_with_update_hint() {
        let newer = encode_with_version(PROTOCOL_VERSION + 1, b"whatever the future holds");
        let err = Hello::decode(&newer).expect_err("mismatch");
        assert_eq!(
            err,
            HandshakeError::VersionMismatch {
                ours: PROTOCOL_VERSION,
                theirs: PROTOCOL_VERSION + 1
            }
        );
        assert!(err.to_string().contains("update the game"));
        let older = encode_with_version(PROTOCOL_VERSION - 1, &[]);
        let err = ServerHello::decode(&older).expect_err("mismatch");
        assert!(err.to_string().contains("needs an update"));
        // The server's mismatch answer decodes as a mismatch on the other version.
        assert_eq!(
            ServerHello::decode(&ServerHello::encode_version_mismatch()),
            Err(HandshakeError::Decode(DecodeError::Malformed))
        );
    }

    #[test]
    fn not_our_protocol() {
        assert_eq!(
            Hello::decode(b"GET / HTTP/1.1\r\n"),
            Err(HandshakeError::NotOurProtocol)
        );
        assert_eq!(Hello::decode(b"VXL"), Err(HandshakeError::NotOurProtocol));
    }

    #[test]
    fn rejection_is_an_error() {
        let bytes = ServerHello::Rejected(Rejection::OfflineNotAllowed).encode();
        let err = ServerHello::decode(&bytes).expect_err("rejected");
        assert_eq!(err, HandshakeError::Rejected(Rejection::OfflineNotAllowed));
        assert!(err.to_string().contains("offline"));
    }

    #[test]
    fn long_text_is_rejected() {
        let mut h = hello();
        h.client_version = "x".repeat(MAX_TEXT_BYTES + 1);
        assert!(matches!(
            Hello::decode(&h.encode()),
            Err(HandshakeError::Decode(DecodeError::Invalid(_)))
        ));
    }
}
