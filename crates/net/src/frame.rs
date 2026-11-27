//! Length-prefixed frames on QUIC streams (`protocol::codec` framing).

use quinn::{RecvStream, SendStream};

/// Reading or writing a frame failed.
#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    /// The stream ended or the connection failed.
    #[error("stream read: {0}")]
    Read(#[from] quinn::ReadExactError),
    /// Writing failed.
    #[error("stream write: {0}")]
    Write(#[from] quinn::WriteError),
    /// The frame length is over the limit of the caller or of the protocol.
    #[error("frame of {size} bytes exceeds the {limit}-byte limit")]
    TooLarge {
        /// Announced length.
        size: usize,
        /// Accepted length.
        limit: usize,
    },
}

/// Reads one frame body of at most `limit` bytes (and at most
/// `protocol::MAX_FRAME_BYTES`). The length is checked before allocating.
pub async fn read_frame(recv: &mut RecvStream, limit: usize) -> Result<Vec<u8>, FrameError> {
    let mut header = [0u8; protocol::codec::FRAME_HEADER_BYTES];
    recv.read_exact(&mut header).await?;
    let size = u32::from_le_bytes(header) as usize;
    let limit = limit.min(protocol::MAX_FRAME_BYTES);
    if size > limit {
        return Err(FrameError::TooLarge { size, limit });
    }
    let mut body = vec![0u8; size];
    recv.read_exact(&mut body).await?;
    Ok(body)
}

/// Writes an already framed message (`encode_frame` output).
pub async fn write_frame(send: &mut SendStream, frame: &[u8]) -> Result<(), FrameError> {
    send.write_all(frame).await?;
    Ok(())
}

/// Frames a raw body (for the handshake, whose bodies carry their own
/// version prefix). Fails only above `protocol::MAX_FRAME_BYTES`.
pub fn frame_body(body: &[u8]) -> Result<Vec<u8>, protocol::EncodeError> {
    protocol::codec::frame(body)
}
