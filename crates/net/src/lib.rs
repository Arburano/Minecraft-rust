//! QUIC transport (quinn): reliable prioritised streams for chunks, datagrams for entity deltas.
//!
//! Spec:
//! - `docs/architecture/networking-protocol.md`
//! - ADR-0007 (QUIC + own protocol)
//!
//! Game connection (T2-02): [`client`] (the client side of the handshake and
//! column streams), [`frame`] (length-prefixed frames on streams); the server
//! side lives in the `server` crate on top of [`endpoint`] and [`frame`].
//!
//! T0-11 spike scope: the channel layout from the spec (datagrams for intents
//! and entity deltas, one prioritised unidirectional stream per column) with
//! random bytes instead of real column data, plus a loss/delay relay for
//! transport-level loss emulation. Real message types (`protocol`, `bitcode`)
//! and the real column format are stage 1 work.

pub mod client;
pub mod endpoint;
pub mod frame;
pub mod lossy;
pub mod spike;

pub use endpoint::{
    ALPN, Error, client_endpoint, client_endpoint_offline, game_server_transport, server_endpoint,
    server_endpoint_with_transport,
};
