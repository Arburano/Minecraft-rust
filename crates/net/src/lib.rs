//! QUIC transport (quinn): reliable prioritised streams for chunks, datagrams for entity deltas.
//!
//! Spec:
//! - `docs/architecture/networking-protocol.md`
//! - ADR-0007 (QUIC + own protocol)
//!
//! - `endpoint`: QUIC endpoints (BBR on the server; an offline-mode client that
//!   accepts the server's self-signed certificate).
//! - `messages`: the handshake header and framed `protocol` messages over
//!   streams (T2-02).
//! - `lossy`: a loss/delay relay for tests.
//! - `spike`: the T0-11 transport spike (random payloads), kept for its
//!   measurements.

pub mod endpoint;
pub mod lossy;
pub mod messages;
pub mod spike;

pub use endpoint::{
    ALPN, Error, client_endpoint, client_endpoint_offline, server_endpoint,
    server_endpoint_with_transport,
};
