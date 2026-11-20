//! Spike-only wire framing (T0-11).
//!
//! Fixed-size little-endian headers, random payloads. This is deliberately
//! not the real protocol: real messages are enums in `protocol` serialized
//! with `bitcode`, and real columns are palette sections + zstd 1
//! (networking-protocol.md). Decoders must never panic on garbage input.

/// Handshake protocol version of the spike. The real `PROTOCOL_VERSION`
/// will live in `protocol` together with the message enums (stage 1).
pub const PROTOCOL_VERSION: u32 = 1;

/// Default column payload size for the spike: 8 KB (task T0-11).
pub const COLUMN_BYTES: usize = 8 * 1024;

/// Upper bound accepted by [`ColumnHeader::decode`]; keeps a garbage header
/// from triggering a huge allocation.
pub const MAX_COLUMN_BYTES: usize = 64 * 1024;

/// Default grid side: 25 × 25 = 625 columns (task T0-11).
pub const GRID_SIDE: i32 = 25;

/// Encoded size of [`ColumnHeader`].
pub const COLUMN_HEADER_LEN: usize = 12;

/// Encoded size of [`DeltaHeader`].
pub const DELTA_HEADER_LEN: usize = 16;

/// Encoded size of [`Intent`].
pub const INTENT_LEN: usize = 8;

/// Header of one column stream: coordinates and payload length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColumnHeader {
    /// Column X in chunks.
    pub x: i32,
    /// Column Z in chunks.
    pub z: i32,
    /// Payload bytes that follow the header.
    pub len: u32,
}

impl ColumnHeader {
    /// Encode into a fixed-size buffer.
    pub fn encode(self) -> [u8; COLUMN_HEADER_LEN] {
        let mut buf = [0u8; COLUMN_HEADER_LEN];
        buf[0..4].copy_from_slice(&self.x.to_le_bytes());
        buf[4..8].copy_from_slice(&self.z.to_le_bytes());
        buf[8..12].copy_from_slice(&self.len.to_le_bytes());
        buf
    }

    /// Decode; `None` on a short buffer or an implausible length.
    pub fn decode(buf: &[u8]) -> Option<Self> {
        if buf.len() < COLUMN_HEADER_LEN {
            return None;
        }
        let x = i32::from_le_bytes(buf[0..4].try_into().ok()?);
        let z = i32::from_le_bytes(buf[4..8].try_into().ok()?);
        let len = u32::from_le_bytes(buf[8..12].try_into().ok()?);
        if len as usize > MAX_COLUMN_BYTES {
            return None;
        }
        Some(Self { x, z, len })
    }
}

/// Header of an entity-delta datagram: sequence number and send time in
/// microseconds since the shared spike epoch (client and server run in one
/// process, so one `Instant` epoch is enough to measure latency).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeltaHeader {
    /// Monotonic delta sequence, starts at 1.
    pub seq: u64,
    /// Send timestamp, µs since the shared epoch.
    pub sent_us: u64,
}

impl DeltaHeader {
    /// Encode into a fixed-size buffer.
    pub fn encode(self) -> [u8; DELTA_HEADER_LEN] {
        let mut buf = [0u8; DELTA_HEADER_LEN];
        buf[0..8].copy_from_slice(&self.seq.to_le_bytes());
        buf[8..16].copy_from_slice(&self.sent_us.to_le_bytes());
        buf
    }

    /// Decode; `None` on a short buffer.
    pub fn decode(buf: &[u8]) -> Option<Self> {
        if buf.len() < DELTA_HEADER_LEN {
            return None;
        }
        let seq = u64::from_le_bytes(buf[0..8].try_into().ok()?);
        let sent_us = u64::from_le_bytes(buf[8..16].try_into().ok()?);
        Some(Self { seq, sent_us })
    }
}

/// A player intent datagram: just a tick number in the spike. Sent
/// duplicated 2–3× against loss (networking-protocol.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Intent {
    /// Client tick number.
    pub tick: u64,
}

impl Intent {
    /// Encode into a fixed-size buffer.
    pub fn encode(self) -> [u8; INTENT_LEN] {
        self.tick.to_le_bytes()
    }

    /// Decode; `None` on a short buffer.
    pub fn decode(buf: &[u8]) -> Option<Self> {
        if buf.len() < INTENT_LEN {
            return None;
        }
        Some(Self {
            tick: u64::from_le_bytes(buf[0..INTENT_LEN].try_into().ok()?),
        })
    }
}

/// Grid coordinates ordered by Euclidean distance from the origin, ties
/// broken by `(x, z)` for determinism. The first 9 entries of a grid with
/// side ≥ 3 are exactly the nearest 3×3 columns.
pub fn columns_by_distance(side: i32) -> Vec<(i32, i32)> {
    let half = side / 2;
    let mut cells: Vec<(i32, i32)> = (-half..=half)
        .flat_map(|x| (-half..=half).map(move |z| (x, z)))
        .collect();
    cells.sort_by_key(|&(x, z)| (x * x + z * z, x, z));
    cells
}

/// Deterministic payload seed of a column, shared by server and client so
/// the client can verify received bytes.
pub fn column_seed(x: i32, z: i32) -> u64 {
    ((x as u32 as u64) << 32) | (z as u32 as u64)
}

/// Deterministic pseudo-random payload bytes (no `rand` dependency; the
/// task explicitly asks for random bytes, not a real column format).
pub fn fill_payload(buf: &mut [u8], seed: u64) {
    let mut state = seed;
    for chunk in buf.chunks_mut(8) {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        let bytes = z.to_le_bytes();
        chunk.copy_from_slice(&bytes[..chunk.len()]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_header_roundtrip() {
        let h = ColumnHeader {
            x: -7,
            z: 12,
            len: COLUMN_BYTES as u32,
        };
        assert_eq!(ColumnHeader::decode(&h.encode()), Some(h));
    }

    #[test]
    fn delta_header_roundtrip() {
        let h = DeltaHeader {
            seq: u64::MAX,
            sent_us: 123_456_789,
        };
        assert_eq!(DeltaHeader::decode(&h.encode()), Some(h));
    }

    #[test]
    fn intent_roundtrip() {
        let i = Intent { tick: 42 };
        assert_eq!(Intent::decode(&i.encode()), Some(i));
    }

    #[test]
    fn decoders_reject_garbage_without_panicking() {
        // Every truncation of a valid encoding, plus arbitrary bytes.
        let enc = ColumnHeader {
            x: 1,
            z: 2,
            len: 8192,
        }
        .encode();
        for cut in 0..COLUMN_HEADER_LEN {
            assert_eq!(ColumnHeader::decode(&enc[..cut]), None);
        }
        let mut state = 0x1234_5678u64;
        for len in 0..64 {
            let mut buf = vec![0u8; len];
            fill_payload(&mut buf, state);
            state = state.wrapping_add(1);
            let _ = ColumnHeader::decode(&buf);
            let _ = DeltaHeader::decode(&buf);
            let _ = Intent::decode(&buf);
        }
        let huge = ColumnHeader {
            x: 0,
            z: 0,
            len: u32::MAX,
        };
        assert_eq!(ColumnHeader::decode(&huge.encode()), None);
    }

    #[test]
    fn distance_order_starts_with_3x3() {
        let cells = columns_by_distance(GRID_SIDE);
        assert_eq!(cells.len(), 625);
        assert_eq!(cells[0], (0, 0));
        let mut first9 = cells[..9].to_vec();
        first9.sort();
        let mut expected: Vec<(i32, i32)> = (-1..=1)
            .flat_map(|x| (-1..=1).map(move |z| (x, z)))
            .collect();
        expected.sort();
        assert_eq!(first9, expected);
        let &(lx, lz) = cells.last().expect("625 cells");
        assert_eq!(lx * lx + lz * lz, 2 * 12 * 12); // a far corner comes last
        // Non-decreasing distance.
        assert!(
            cells
                .windows(2)
                .all(|w| w[0].0 * w[0].0 + w[0].1 * w[0].1 <= w[1].0 * w[1].0 + w[1].1 * w[1].1)
        );
    }

    #[test]
    fn payload_is_deterministic_and_varied() {
        let mut a = vec![0u8; 8192];
        let mut b = vec![0u8; 8192];
        fill_payload(&mut a, 99);
        fill_payload(&mut b, 99);
        assert_eq!(a, b);
        fill_payload(&mut b, 100);
        assert_ne!(a, b);
        // Odd length must not panic.
        let mut odd = vec![0u8; 13];
        fill_payload(&mut odd, 1);
    }
}
