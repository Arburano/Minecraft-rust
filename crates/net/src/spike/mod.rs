//! T0-11 QUIC transport spike: server and client on `quinn` with the
//! channels from networking-protocol.md.
//!
//! - control: one bidirectional stream, a minimal version handshake;
//! - input: datagrams, client → server, each intent duplicated 2×;
//! - entities: datagrams, server → client at 20 Hz;
//! - chunks: one unidirectional stream per column, ordered by distance,
//!   `SendStream::set_priority` by distance rank, at most
//!   [`ServerParams::max_chunk_streams`] concurrent streams.
//!
//! Payloads are deterministic random bytes; the real column format and the
//! real `protocol` messages are out of scope (task T0-11).

pub mod client;
pub mod server;
pub mod wire;

pub use client::{ClientParams, ClientReport, run_client};
pub use server::{ServerParams, ServerStats, serve_connection};
pub use wire::{COLUMN_BYTES, GRID_SIDE, PROTOCOL_VERSION};

/// Errors of the spike client/server.
#[derive(Debug, thiserror::Error)]
pub enum SpikeError {
    /// The QUIC connection failed.
    #[error("connection: {0}")]
    Connection(#[from] quinn::ConnectionError),
    /// Connecting to the server failed.
    #[error("connect: {0}")]
    Connect(#[from] quinn::ConnectError),
    /// Writing to a stream failed.
    #[error("stream write: {0}")]
    Write(#[from] quinn::WriteError),
    /// Reading a stream to end failed.
    #[error("stream read: {0}")]
    ReadToEnd(#[from] quinn::ReadToEndError),
    /// Reading an exact count of bytes failed.
    #[error("stream read exact: {0}")]
    ReadExact(#[from] quinn::ReadExactError),
    /// The stream was already closed.
    #[error("stream closed")]
    ClosedStream(#[from] quinn::ClosedStream),
    /// Waiting for the peer to acknowledge a stream failed.
    #[error("stream stopped: {0}")]
    Stopped(#[from] quinn::StoppedError),
    /// The peer speaks an incompatible protocol version.
    #[error("protocol version mismatch: peer {peer}, ours {ours}")]
    VersionMismatch {
        /// Version the peer sent.
        peer: u32,
        /// Our version.
        ours: u32,
    },
    /// A column stream carried a malformed header.
    #[error("malformed column header")]
    MalformedColumn,
    /// A spawned column task panicked; this is a bug, not peer input.
    #[error("column task panicked: {0}")]
    TaskPanic(#[from] tokio::task::JoinError),
}

impl SpikeError {
    /// Whether the error just means the peer went away (the spike client
    /// closes the connection when its observation window ends).
    pub fn is_peer_closed(&self) -> bool {
        fn closed(e: &quinn::ConnectionError) -> bool {
            matches!(
                e,
                quinn::ConnectionError::ApplicationClosed(_)
                    | quinn::ConnectionError::LocallyClosed
                    | quinn::ConnectionError::TimedOut
            )
        }
        match self {
            Self::Connection(e) => closed(e),
            Self::ClosedStream(_) => true,
            Self::Stopped(quinn::StoppedError::ConnectionLost(e)) => closed(e),
            Self::Write(quinn::WriteError::ConnectionLost(e)) => closed(e),
            Self::ReadExact(quinn::ReadExactError::ReadError(
                quinn::ReadError::ConnectionLost(e),
            )) => closed(e),
            Self::ReadToEnd(quinn::ReadToEndError::Read(quinn::ReadError::ConnectionLost(e))) => {
                closed(e)
            }
            _ => false,
        }
    }
}
