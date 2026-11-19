//! QUIC transport (quinn): reliable prioritised streams for chunks, datagrams for entity deltas.
//!
//! Spec:
//! - `docs/architecture/networking-protocol.md`
//! - ADR-0007 (QUIC + own protocol)
//!
//! T0-11 spike scope: the channel layout from the spec (datagrams for intents
//! and entity deltas, one prioritised unidirectional stream per column) with
//! random bytes instead of real column data, plus a loss/delay relay for
//! transport-level loss emulation. Real message types (`protocol`, `bitcode`)
//! and the real column format are stage 1 work.

pub mod endpoint;
pub mod lossy;
pub mod spike;

pub use endpoint::{ALPN, Error, client_endpoint, server_endpoint, server_endpoint_with_transport};
