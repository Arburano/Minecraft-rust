//! Versioned network message types shared by client, server and gateway.
//!
//! Spec: `docs/architecture/networking-protocol.md`, ADR-0007 (QUIC + own
//! protocol). The transport (`quinn`) lives in `net`; this crate only defines
//! what goes over it and how it is encoded:
//!
//! | Channel | QUIC type | Messages |
//! | --- | --- | --- |
//! | control | bidirectional stream | [`handshake`] (`Hello` / `Welcome` / `Registry` / `Spawn`), then [`ClientControl`] / [`ServerControl`] |
//! | input | datagrams | [`InputDatagram`] |
//! | entities | datagrams | [`EntityDelta`] |
//! | chunks | unidirectional stream per column | [`ColumnData`] |
//! | events | bidirectional stream | [`ClientEvent`] / [`ServerEvent`] |
//!
//! Every message is a Rust type serialized with `bitcode` ([`codec`]). Stream
//! messages are length-prefixed frames; datagrams are at most
//! [`MAX_DATAGRAM_BYTES`]. Decoding never panics on garbage input: every
//! decoder returns an error instead (fuzz test `tests/fuzz.rs`).
//!
//! Only the first frame of each direction on the control stream has a fixed
//! layout (magic + [`PROTOCOL_VERSION`]), so a peer of any other version is
//! told about the mismatch instead of seeing garbage
//! ([`handshake::HandshakeError::VersionMismatch`]).

pub mod codec;
pub mod column;
pub mod handshake;
pub mod messages;
pub mod quant;

pub use codec::{
    DecodeError, EncodeError, FrameReader, MAX_DATAGRAM_BYTES, MAX_FRAME_BYTES, decode,
    decode_datagram, encode, encode_datagram, encode_frame,
};
pub use column::{ColumnData, ColumnError};
pub use handshake::{
    ClaimedIdentity, HandshakeError, Hello, Registry, Rejection, ServerHello, ServerInfo, Spawn,
    Welcome,
};
pub use messages::{
    ClientControl, ClientEvent, EntityDelta, EntitySpawn, EntityUpdate, InputDatagram, InputFrame,
    NetEntityId, PosUpdate, ServerControl, ServerEvent,
};
pub use quant::{Angle8, Angle16, FixedPos, PosDelta};

/// Protocol version exchanged in the handshake. Bump it on any change to a
/// message layout; the golden handshake files (`tests/golden/`) make such a
/// change deliberate.
pub const PROTOCOL_VERSION: u32 = 1;
