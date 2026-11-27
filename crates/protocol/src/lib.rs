//! Versioned network message types shared by client, server and gateway.
//!
//! Spec: `docs/architecture/networking-protocol.md`, ADR-0007.
//!
//! The crate is transport-agnostic: it turns messages into bytes and back.
//! `net` moves the bytes over QUIC. One QUIC connection carries these channels:
//!
//! | Channel | QUIC type | Messages |
//! | --- | --- | --- |
//! | control | bidirectional stream | [`HandshakeHeader`], then framed [`ClientControl`] / [`ServerControl`] |
//! | input | datagrams | [`InputDatagram`] |
//! | entities | datagrams | [`EntityDatagram`] |
//! | chunks | one unidirectional stream per column | [`ChunkStream`] |
//! | events | bidirectional stream | framed [`ClientEvent`] / [`ServerEvent`] |
//!
//! Every message is a Rust type serialized with `bitcode`. Decoding is safe on
//! arbitrary input: a size limit is checked before decoding, `bitcode` rejects
//! malformed data, and [`Message::validate`] rejects values outside the
//! protocol's limits. Nothing here panics on garbage (fuzz-tested).
//!
//! Versioning: [`PROTOCOL_VERSION`] is sent in the fixed 8-byte
//! [`HandshakeHeader`], which never changes layout, so two builds with
//! different message layouts still recognize each other and report a clear
//! [`ProtocolError::VersionMismatch`] instead of a decode error.

mod chunks;
mod control;
mod entities;
mod events;
mod frame;
mod input;
pub mod lan;
mod wire;

pub use chunks::{ChunkStream, ColumnPayload, MAX_COLUMN_COMPRESSED, MAX_COLUMN_RAW};
pub use control::{
    ClientControl, HandshakeHeader, Hello, KickReason, MAX_NAME_LEN, MAX_TEXT_LEN, RegistrySync,
    ServerControl, Spawn, Welcome,
};
pub use entities::{
    ABILITY_STATE_BYTES, Correction, EntityDatagram, EntityUpdate, Look, MAX_ENTITY_UPDATES, Step8,
};
pub use events::{BlockChange, ClientEvent, MAX_BLOCK_CHANGES, MAX_CHAT_LEN, ServerEvent};
pub use frame::{FRAME_HEADER_LEN, MAX_FRAME, encode_frame, split_frame};
pub use input::{Buttons, InputDatagram, Intent, MAX_INTENTS};
pub use wire::{Angle8, Angle16, BlockXyz, ColumnXz, NetId, RelPos};

/// Version of the message layouts. Any change to a message type that an older
/// build cannot decode raises it (and the golden files in `tests/golden/`).
pub const PROTOCOL_VERSION: u32 = 4;

/// QUIC ALPN of this protocol family (`networking-protocol.md`).
pub const ALPN: &[u8] = b"vxl/1";

/// Largest datagram payload we send: anything larger goes into a stream.
pub const MAX_DATAGRAM: usize = 1200;

/// Errors from encoding or decoding protocol data.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProtocolError {
    /// The peer is not speaking this protocol at all.
    #[error("not a voxel-engine connection (bad handshake magic)")]
    BadMagic,
    /// Both sides speak this protocol, but different versions of it.
    #[error(
        "protocol version mismatch: this build speaks version {local}, the other side speaks {remote}. {}",
        version_hint(*.local, *.remote)
    )]
    VersionMismatch {
        /// [`PROTOCOL_VERSION`] of this build.
        local: u32,
        /// The version announced by the peer.
        remote: u32,
    },
    /// The data ends before the message does.
    #[error("truncated message")]
    Truncated,
    /// A message is larger than its channel allows.
    #[error("message of {len} bytes exceeds the limit of {max}")]
    TooLarge {
        /// Size of the offending message.
        len: usize,
        /// Limit for its channel.
        max: usize,
    },
    /// The bytes are not a valid encoding of the expected message.
    #[error("malformed message: {0}")]
    Malformed(String),
    /// The message decoded, but a value is outside the protocol's limits.
    #[error("invalid message: {0}")]
    Invalid(&'static str),
}

fn version_hint(local: u32, remote: u32) -> &'static str {
    if remote > local {
        "Update the game to join this server."
    } else {
        "The server runs an older version; ask its owner to update it, or use a matching game version."
    }
}

/// A protocol message: encoded with `bitcode`, size-limited and validated.
pub trait Message: bitcode::Encode + bitcode::DecodeOwned + Sized {
    /// The largest encoded size accepted for this message.
    const MAX_LEN: usize;

    /// Checks values that the type system does not bound (lengths, ranges).
    fn validate(&self) -> Result<(), ProtocolError>;

    /// Encodes the message. Fails if it is invalid or larger than [`Self::MAX_LEN`],
    /// so a sender never produces what a receiver would reject.
    fn encode(&self) -> Result<Vec<u8>, ProtocolError> {
        self.validate()?;
        let bytes = bitcode::encode(self);
        if bytes.len() > Self::MAX_LEN {
            return Err(ProtocolError::TooLarge {
                len: bytes.len(),
                max: Self::MAX_LEN,
            });
        }
        Ok(bytes)
    }

    /// Decodes and validates a message from untrusted bytes. Never panics.
    fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        if bytes.len() > Self::MAX_LEN {
            return Err(ProtocolError::TooLarge {
                len: bytes.len(),
                max: Self::MAX_LEN,
            });
        }
        let msg: Self =
            bitcode::decode(bytes).map_err(|e| ProtocolError::Malformed(e.to_string()))?;
        msg.validate()?;
        Ok(msg)
    }
}

pub(crate) fn check(ok: bool, what: &'static str) -> Result<(), ProtocolError> {
    if ok {
        Ok(())
    } else {
        Err(ProtocolError::Invalid(what))
    }
}

pub(crate) fn check_text(s: &str, max: usize, what: &'static str) -> Result<(), ProtocolError> {
    check(s.len() <= max && !s.chars().any(char::is_control), what)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_mismatch_message_tells_what_to_do() {
        let newer = ProtocolError::VersionMismatch {
            local: 1,
            remote: 2,
        };
        assert!(newer.to_string().contains("Update the game"));
        let older = ProtocolError::VersionMismatch {
            local: 2,
            remote: 1,
        };
        assert!(older.to_string().contains("older version"));
    }
}
