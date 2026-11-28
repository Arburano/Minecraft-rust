//! Length-prefixed frames for streams that carry several messages (control,
//! events): `u32` little-endian body length, then the `bitcode` body.

use crate::{Message, ProtocolError};

/// Bytes of the length prefix.
pub const FRAME_HEADER_LEN: usize = 4;

/// The largest frame body on any stream. The largest message is
/// [`crate::RegistrySync`] (the ID table); see its limit.
pub const MAX_FRAME: usize = 4 << 20;

/// Appends `msg` as one frame to `out`.
pub fn encode_frame<M: Message>(msg: &M, out: &mut Vec<u8>) -> Result<(), ProtocolError> {
    let body = msg.encode()?;
    // `Message::encode` bounds the body by `M::MAX_LEN`, which is ≤ MAX_FRAME.
    let len = u32::try_from(body.len()).map_err(|_| ProtocolError::TooLarge {
        len: body.len(),
        max: MAX_FRAME,
    })?;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&body);
    Ok(())
}

/// Splits the first complete frame off `buf`: returns its body and the total
/// bytes it used, or `Ok(None)` if more bytes are needed. A length above
/// [`MAX_FRAME`] is an error at once, before any of the body arrives, so a peer
/// cannot make us buffer an unbounded frame.
pub fn split_frame(buf: &[u8]) -> Result<Option<(&[u8], usize)>, ProtocolError> {
    let Some(head) = buf.get(..FRAME_HEADER_LEN) else {
        return Ok(None);
    };
    let len = u32::from_le_bytes([head[0], head[1], head[2], head[3]]) as usize;
    if len > MAX_FRAME {
        return Err(ProtocolError::TooLarge {
            len,
            max: MAX_FRAME,
        });
    }
    let end = FRAME_HEADER_LEN + len;
    Ok(buf.get(FRAME_HEADER_LEN..end).map(|body| (body, end)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ClientEvent, ServerEvent};

    #[test]
    fn frames_split_back_in_order_and_wait_for_more_bytes() {
        let mut stream = Vec::new();
        let a = ClientEvent::Chat {
            text: "hello".into(),
        };
        let b = ClientEvent::Chat {
            text: "world".into(),
        };
        encode_frame(&a, &mut stream).expect("encode");
        encode_frame(&b, &mut stream).expect("encode");

        for cut in 0..FRAME_HEADER_LEN + 1 {
            assert_eq!(split_frame(&stream[..cut]).expect("ok"), None);
        }
        let (body, used) = split_frame(&stream).expect("ok").expect("frame");
        assert_eq!(ClientEvent::decode(body).expect("decode"), a);
        let (body, used2) = split_frame(&stream[used..]).expect("ok").expect("frame");
        assert_eq!(ClientEvent::decode(body).expect("decode"), b);
        assert_eq!(used + used2, stream.len());
    }

    #[test]
    fn oversized_length_is_rejected_before_the_body() {
        let len = (MAX_FRAME as u32 + 1).to_le_bytes();
        assert!(matches!(
            split_frame(&len),
            Err(ProtocolError::TooLarge { .. })
        ));
        // A frame of the right type but with a body that is not that type.
        let mut out = Vec::new();
        encode_frame(
            &ServerEvent::ForgetColumn {
                dimension: 0,
                column: crate::ColumnXz { x: 1, z: 2 },
            },
            &mut out,
        )
        .expect("encode");
        let (body, _) = split_frame(&out).expect("ok").expect("frame");
        assert!(crate::ClientControl::decode(body).is_err());
    }
}
