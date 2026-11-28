//! Encoding of messages: `bitcode` bodies, length-prefixed stream frames and
//! size-checked datagrams.
//!
//! A stream frame is `u32` little-endian body length + body. Frames longer than
//! [`MAX_FRAME_BYTES`] are rejected before any allocation, so a garbage length
//! cannot make the reader reserve gigabytes.

use std::fmt;

/// Largest datagram payload (networking-protocol.md: "a datagram is ≤ ~1,200
/// bytes; anything larger goes into a stream"). Stays under the QUIC minimum
/// path MTU of 1,200 bytes minus nothing extra on our side; `quinn` reports
/// the exact per-path limit through `max_datagram_size`.
pub const MAX_DATAGRAM_BYTES: usize = 1200;

/// Largest stream frame body. The biggest frame is the ID table in
/// `Registry` or one column; 4 MiB covers ~100k state names.
pub const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;

/// Bytes of the frame length prefix.
pub const FRAME_HEADER_BYTES: usize = 4;

/// Encoding failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EncodeError {
    /// The encoded message is larger than the channel allows.
    TooLarge {
        /// Encoded size in bytes.
        size: usize,
        /// The channel's limit.
        limit: usize,
    },
}

impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { size, limit } => {
                write!(f, "message of {size} bytes exceeds the {limit}-byte limit")
            }
        }
    }
}

impl std::error::Error for EncodeError {}

/// Decoding failed: the input is not a valid message. The peer is broken or
/// malicious; the caller drops the datagram or closes the connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// The input is longer than the channel allows.
    TooLarge {
        /// Input size in bytes.
        size: usize,
        /// The channel's limit.
        limit: usize,
    },
    /// The bytes are not a valid encoding of the expected type.
    Malformed,
    /// The encoding is valid but a value is out of range (e.g. too many
    /// input frames, too many sections).
    Invalid(&'static str),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { size, limit } => {
                write!(f, "message of {size} bytes exceeds the {limit}-byte limit")
            }
            Self::Malformed => f.write_str("malformed message"),
            Self::Invalid(what) => write!(f, "invalid message: {what}"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Encodes a message body (no frame header).
#[must_use]
pub fn encode<T: bitcode::Encode + ?Sized>(msg: &T) -> Vec<u8> {
    bitcode::encode(msg)
}

/// Decodes a message body (no frame header). Never panics.
pub fn decode<T: bitcode::DecodeOwned>(bytes: &[u8]) -> Result<T, DecodeError> {
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(DecodeError::TooLarge {
            size: bytes.len(),
            limit: MAX_FRAME_BYTES,
        });
    }
    bitcode::decode(bytes).map_err(|_| DecodeError::Malformed)
}

/// Encodes a datagram, failing if it exceeds [`MAX_DATAGRAM_BYTES`].
pub fn encode_datagram<T: bitcode::Encode + ?Sized>(msg: &T) -> Result<Vec<u8>, EncodeError> {
    let bytes = bitcode::encode(msg);
    if bytes.len() > MAX_DATAGRAM_BYTES {
        return Err(EncodeError::TooLarge {
            size: bytes.len(),
            limit: MAX_DATAGRAM_BYTES,
        });
    }
    Ok(bytes)
}

/// Decodes a datagram. Never panics.
pub fn decode_datagram<T: bitcode::DecodeOwned>(bytes: &[u8]) -> Result<T, DecodeError> {
    if bytes.len() > MAX_DATAGRAM_BYTES {
        return Err(DecodeError::TooLarge {
            size: bytes.len(),
            limit: MAX_DATAGRAM_BYTES,
        });
    }
    bitcode::decode(bytes).map_err(|_| DecodeError::Malformed)
}

/// Wraps an already encoded body into a frame.
pub fn frame(body: &[u8]) -> Result<Vec<u8>, EncodeError> {
    if body.len() > MAX_FRAME_BYTES {
        return Err(EncodeError::TooLarge {
            size: body.len(),
            limit: MAX_FRAME_BYTES,
        });
    }
    let mut out = Vec::with_capacity(FRAME_HEADER_BYTES + body.len());
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(body);
    Ok(out)
}

/// Encodes a message as one stream frame.
pub fn encode_frame<T: bitcode::Encode + ?Sized>(msg: &T) -> Result<Vec<u8>, EncodeError> {
    frame(&bitcode::encode(msg))
}

/// Parses a frame length prefix, rejecting lengths over [`MAX_FRAME_BYTES`].
/// For transports that read the header and the body separately
/// (`RecvStream::read_exact`).
pub fn frame_len(header: [u8; FRAME_HEADER_BYTES]) -> Result<usize, DecodeError> {
    let len = u32::from_le_bytes(header) as usize;
    if len > MAX_FRAME_BYTES {
        return Err(DecodeError::TooLarge {
            size: len,
            limit: MAX_FRAME_BYTES,
        });
    }
    Ok(len)
}

/// Splits a byte stream into frames. Feed it whatever the stream delivered,
/// then take complete frame bodies out.
#[derive(Debug, Default)]
pub struct FrameReader {
    buf: Vec<u8>,
    start: usize,
}

impl FrameReader {
    /// An empty reader.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends received bytes.
    pub fn push(&mut self, bytes: &[u8]) {
        if self.start > 0 && self.start == self.buf.len() {
            self.buf.clear();
            self.start = 0;
        }
        self.buf.extend_from_slice(bytes);
    }

    /// Bytes received but not yet returned as frames.
    #[must_use]
    pub fn pending(&self) -> usize {
        self.buf.len() - self.start
    }

    /// The next complete frame body, `Ok(None)` if more bytes are needed, or
    /// an error on an oversized length (the stream is unusable after that).
    pub fn next_frame(&mut self) -> Result<Option<&[u8]>, DecodeError> {
        let rest = &self.buf[self.start..];
        let Some(header) = rest.first_chunk::<FRAME_HEADER_BYTES>() else {
            return Ok(None);
        };
        let len = frame_len(*header)?;
        let end = FRAME_HEADER_BYTES + len;
        if rest.len() < end {
            return Ok(None);
        }
        let from = self.start + FRAME_HEADER_BYTES;
        self.start += end;
        Ok(Some(&self.buf[from..from + len]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_split_and_join() {
        let a = encode_frame(&(1u32, String::from("hi"))).expect("small");
        let b = encode_frame(&vec![7u8; 300]).expect("small");
        let mut all = a.clone();
        all.extend_from_slice(&b);
        // Deliver one byte at a time.
        let mut r = FrameReader::new();
        let mut got: Vec<Vec<u8>> = Vec::new();
        for byte in &all {
            r.push(std::slice::from_ref(byte));
            while let Some(f) = r.next_frame().expect("valid") {
                got.push(f.to_vec());
            }
        }
        assert_eq!(got.len(), 2);
        assert_eq!(
            decode::<(u32, String)>(&got[0]).expect("decodes"),
            (1, "hi".into())
        );
        assert_eq!(decode::<Vec<u8>>(&got[1]).expect("decodes"), vec![7u8; 300]);
        assert_eq!(r.pending(), 0);
    }

    #[test]
    fn oversized_frame_length_is_rejected() {
        let mut r = FrameReader::new();
        r.push(&u32::MAX.to_le_bytes());
        assert!(matches!(r.next_frame(), Err(DecodeError::TooLarge { .. })));
        assert!(frame_len(((MAX_FRAME_BYTES + 1) as u32).to_le_bytes()).is_err());
        assert_eq!(
            frame_len((MAX_FRAME_BYTES as u32).to_le_bytes()),
            Ok(MAX_FRAME_BYTES)
        );
    }

    #[test]
    fn datagram_limit() {
        // bitcode packs repetitive data, so use bytes that do not compress.
        let varied = |n: usize| -> Vec<u8> { (0..n).map(|i| (i * 131 % 251) as u8).collect() };
        assert!(encode_datagram(&varied(1100)).is_ok());
        assert!(matches!(
            encode_datagram(&varied(1300)),
            Err(EncodeError::TooLarge { .. })
        ));
        assert!(matches!(
            decode_datagram::<Vec<u8>>(&[0u8; 1201]),
            Err(DecodeError::TooLarge { .. })
        ));
    }
}
