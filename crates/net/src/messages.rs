//! Protocol messages over QUIC streams (`networking-protocol.md`): the fixed
//! handshake header and length-prefixed frames on the control and events
//! streams. Every read is bounded by the expected message's size limit before
//! the body is read, so a peer cannot make us buffer more than that.

use protocol::{FRAME_HEADER_LEN, HandshakeHeader, Message, ProtocolError, encode_frame};
use quinn::{ReadExactError, RecvStream, SendStream};

/// Errors on a message stream.
#[derive(Debug, thiserror::Error)]
pub enum StreamError {
    /// The peer sent something that is not valid protocol data.
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
    /// Reading failed (connection lost, stream reset).
    #[error("read: {0}")]
    Read(#[from] quinn::ReadError),
    /// Writing failed.
    #[error("write: {0}")]
    Write(#[from] quinn::WriteError),
    /// The stream ended in the middle of a message.
    #[error("stream ended inside a message")]
    Truncated,
}

impl From<ReadExactError> for StreamError {
    fn from(e: ReadExactError) -> Self {
        match e {
            ReadExactError::FinishedEarly(_) => Self::Truncated,
            ReadExactError::ReadError(e) => Self::Read(e),
        }
    }
}

/// Sends this build's handshake header.
pub async fn write_header(send: &mut SendStream) -> Result<(), StreamError> {
    send.write_all(&HandshakeHeader::current().encode()).await?;
    Ok(())
}

/// Reads the peer's handshake header (not yet version-checked: call
/// [`HandshakeHeader::check`]).
pub async fn read_header(recv: &mut RecvStream) -> Result<HandshakeHeader, StreamError> {
    let mut buf = [0u8; HandshakeHeader::LEN];
    recv.read_exact(&mut buf).await?;
    Ok(HandshakeHeader::decode(&buf)?)
}

/// Sends one framed message.
pub async fn write_message<M: Message>(send: &mut SendStream, msg: &M) -> Result<(), StreamError> {
    let mut out = Vec::new();
    encode_frame(msg, &mut out)?;
    send.write_all(&out).await?;
    Ok(())
}

/// Reads one framed message; `Ok(None)` if the stream ended cleanly between
/// messages. A length above `M::MAX_LEN` is refused before the body is read.
pub async fn read_message<M: Message>(recv: &mut RecvStream) -> Result<Option<M>, StreamError> {
    let mut head = [0u8; FRAME_HEADER_LEN];
    match recv.read_exact(&mut head).await {
        Ok(()) => {}
        Err(ReadExactError::FinishedEarly(0)) => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let len = u32::from_le_bytes(head) as usize;
    if len > M::MAX_LEN {
        return Err(ProtocolError::TooLarge {
            len,
            max: M::MAX_LEN,
        }
        .into());
    }
    let mut body = vec![0u8; len];
    recv.read_exact(&mut body).await?;
    Ok(Some(M::decode(&body)?))
}
