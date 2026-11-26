//! The chunks channel: one unidirectional stream per column, carrying one
//! [`ChunkStream`] message (`networking-protocol.md`: palette sections +
//! zstd level 1).
//!
//! The column body reuses the on-disk record encoding of T1-02
//! (`world::storage::encode_column`), so the server can stream what it loads
//! without re-encoding sections. Changing `world::storage::FORMAT_VERSION`
//! therefore also requires a [`crate::PROTOCOL_VERSION`] bump (the golden
//! files catch it). Light is computed on the client (ADR-0011), so the light
//! blob is normally empty.

use std::io::Read;

use bitcode::{Decode, Encode};
use world::storage::{ColumnRecord, decode_column, encode_column};

use crate::{ColumnXz, Message, ProtocolError, check};

/// zstd level for columns on the wire (fast; saves use level 3).
pub const ZSTD_LEVEL: i32 = 1;
/// Largest compressed column payload.
pub const MAX_COLUMN_COMPRESSED: usize = 1 << 20;
/// Largest decompressed column record: a guard against decompression bombs.
/// 8 MiB holds ≈ 1,000 sections even in the uncompressed (direct) form;
/// a 24-section column is far below.
pub const MAX_COLUMN_RAW: usize = 8 << 20;

/// One column as sent: position and the compressed record.
#[derive(Encode, Decode, Clone, Debug, PartialEq, Eq)]
pub struct ColumnPayload {
    /// The column; must match the position inside the record.
    pub pos: ColumnXz,
    /// zstd-compressed `world::storage` column record. Block states are the
    /// numbers of the connection's [`crate::RegistrySync`].
    pub zstd: Vec<u8>,
}

impl ColumnPayload {
    /// Encodes and compresses a column record.
    pub fn encode(record: &ColumnRecord) -> Result<Self, ProtocolError> {
        let raw = encode_column(record, 0);
        if raw.len() > MAX_COLUMN_RAW {
            return Err(ProtocolError::TooLarge {
                len: raw.len(),
                max: MAX_COLUMN_RAW,
            });
        }
        let zstd = zstd::bulk::compress(&raw, ZSTD_LEVEL)
            .map_err(|e| ProtocolError::Malformed(e.to_string()))?;
        let payload = Self {
            pos: record.pos.into(),
            zstd,
        };
        payload.validate()?;
        Ok(payload)
    }

    /// Decompresses and decodes the record. Never panics on garbage, and
    /// never allocates more than [`MAX_COLUMN_RAW`] for the record. The
    /// receiver maps the state numbers to its registry with `Section::remap`
    /// and the table from `Registry::remap`.
    pub fn decode(&self) -> Result<ColumnRecord, ProtocolError> {
        self.validate()?;
        let mut raw = Vec::new();
        zstd::stream::read::Decoder::new(self.zstd.as_slice())
            .map_err(|e| ProtocolError::Malformed(e.to_string()))?
            .take(MAX_COLUMN_RAW as u64 + 1)
            .read_to_end(&mut raw)
            .map_err(|e| ProtocolError::Malformed(format!("column zstd: {e}")))?;
        if raw.len() > MAX_COLUMN_RAW {
            return Err(ProtocolError::TooLarge {
                len: raw.len(),
                max: MAX_COLUMN_RAW,
            });
        }
        let (record, _table) =
            decode_column(&raw).map_err(|e| ProtocolError::Malformed(e.to_string()))?;
        check(ColumnXz::from(record.pos) == self.pos, "column position")?;
        Ok(record)
    }

    fn validate(&self) -> Result<(), ProtocolError> {
        if self.zstd.len() > MAX_COLUMN_COMPRESSED {
            return Err(ProtocolError::TooLarge {
                len: self.zstd.len(),
                max: MAX_COLUMN_COMPRESSED,
            });
        }
        Ok(())
    }
}

/// The message of one chunk stream. LOD sections (stage 5) will be a variant
/// of their own on the lod channel.
#[derive(Encode, Decode, Clone, Debug, PartialEq, Eq)]
pub enum ChunkStream {
    /// A full column.
    Column(ColumnPayload),
}

impl Message for ChunkStream {
    const MAX_LEN: usize = MAX_COLUMN_COMPRESSED + 64;

    fn validate(&self) -> Result<(), ProtocolError> {
        match self {
            Self::Column(c) => c.validate(),
        }
    }
}
