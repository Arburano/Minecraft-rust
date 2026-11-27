//! Column records on the chunks channel: palette sections + zstd level 1
//! (networking-protocol.md). Sections use the on-disk section encoding of
//! T1-02 (`world::Section::encode`), so the server can reuse it and the
//! decoder validates every palette index. Light is not sent: the client
//! computes it (ADR-0011).

use std::fmt;

use world::{ColumnPos, Section, SectionDecodeError, WorldBounds};

use crate::codec::{self, DecodeError, EncodeError};

/// zstd level for columns on the wire (fast; the disk format uses 3).
pub const ZSTD_LEVEL: i32 = 1;

/// Most sections in a column (a 1,024-block-tall world). Bounds the
/// decompression buffer.
pub const MAX_COLUMN_SECTIONS: u32 = 64;

/// Largest encoding of one section: tag + 4,096 `u16` states (direct).
pub const MAX_SECTION_BYTES: usize = 1 + 2 * 4096;

/// One column, sent on its own unidirectional stream.
#[derive(Clone, Debug, PartialEq, Eq, bitcode::Encode, bitcode::Decode)]
pub struct ColumnData {
    /// Column X.
    pub x: i32,
    /// Column Z.
    pub z: i32,
    /// Lowest section Y.
    pub min_section_y: i32,
    /// Number of sections in `payload`.
    pub section_count: u32,
    /// zstd-compressed concatenation of `Section::encode`, bottom to top, in
    /// the server's block IDs (`Registry::id_table`).
    pub payload: Vec<u8>,
}

/// A column record is unusable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ColumnError {
    /// The frame does not decode.
    Decode(DecodeError),
    /// More than [`MAX_COLUMN_SECTIONS`] sections, or a count that does not
    /// match the payload.
    SectionCount,
    /// The payload is not valid zstd or is larger than its section count allows.
    Compression,
    /// A section is corrupt.
    Section(SectionDecodeError),
}

impl fmt::Display for ColumnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode(e) => write!(f, "column record: {e}"),
            Self::SectionCount => f.write_str("column record: bad section count"),
            Self::Compression => f.write_str("column record: bad compressed payload"),
            Self::Section(e) => write!(f, "column record: {e}"),
        }
    }
}

impl std::error::Error for ColumnError {}

impl ColumnData {
    /// Packs a column's sections (bottom to top).
    pub fn from_sections(
        pos: ColumnPos,
        bounds: WorldBounds,
        sections: &[Section],
    ) -> Result<Self, ColumnError> {
        let count = u32::try_from(sections.len()).map_err(|_| ColumnError::SectionCount)?;
        if count > MAX_COLUMN_SECTIONS || count != bounds.section_count {
            return Err(ColumnError::SectionCount);
        }
        let mut raw = Vec::with_capacity(sections.len() * 512);
        for s in sections {
            s.encode(&mut raw);
        }
        let payload =
            zstd::bulk::compress(&raw, ZSTD_LEVEL).map_err(|_| ColumnError::Compression)?;
        Ok(Self {
            x: pos.x,
            z: pos.z,
            min_section_y: bounds.min_section_y,
            section_count: count,
            payload,
        })
    }

    /// Column position.
    #[must_use]
    pub fn pos(&self) -> ColumnPos {
        ColumnPos {
            x: self.x,
            z: self.z,
        }
    }

    /// Vertical bounds.
    #[must_use]
    pub fn bounds(&self) -> WorldBounds {
        WorldBounds {
            min_section_y: self.min_section_y,
            section_count: self.section_count,
        }
    }

    /// Unpacks the sections, validating everything. Never panics.
    pub fn sections(&self) -> Result<Vec<Section>, ColumnError> {
        if self.section_count > MAX_COLUMN_SECTIONS {
            return Err(ColumnError::SectionCount);
        }
        let limit = self.section_count as usize * MAX_SECTION_BYTES;
        let raw =
            zstd::bulk::decompress(&self.payload, limit).map_err(|_| ColumnError::Compression)?;
        let mut sections = Vec::with_capacity(self.section_count as usize);
        let mut at = 0usize;
        for _ in 0..self.section_count {
            let rest = raw.get(at..).ok_or(ColumnError::SectionCount)?;
            let (s, used) = Section::decode(rest).map_err(ColumnError::Section)?;
            at += used;
            sections.push(s);
        }
        if at != raw.len() {
            return Err(ColumnError::SectionCount);
        }
        Ok(sections)
    }

    /// Encodes the stream frame.
    pub fn encode_frame(&self) -> Result<Vec<u8>, EncodeError> {
        codec::encode_frame(self)
    }

    /// Decodes a stream frame body (the payload is unpacked by
    /// [`Self::sections`], on a worker thread).
    pub fn decode(bytes: &[u8]) -> Result<Self, ColumnError> {
        codec::decode(bytes).map_err(ColumnError::Decode)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use world::BlockStateId;

    fn sample() -> (ColumnPos, WorldBounds, Vec<Section>) {
        let bounds = WorldBounds {
            min_section_y: -4,
            section_count: 4,
        };
        let mut blocks = [BlockStateId::AIR; 4096];
        for (i, b) in blocks.iter_mut().enumerate() {
            *b = BlockStateId((i % 7) as u16);
        }
        let mut direct = [BlockStateId::AIR; 4096];
        for (i, b) in direct.iter_mut().enumerate() {
            *b = BlockStateId((i % 300) as u16);
        }
        let sections = vec![
            Section::uniform(BlockStateId(2)),
            Section::from_blocks(&blocks),
            Section::from_blocks(&direct),
            Section::uniform(BlockStateId::AIR),
        ];
        (ColumnPos { x: -3, z: 9 }, bounds, sections)
    }

    #[test]
    fn roundtrip() {
        let (pos, bounds, sections) = sample();
        let c = ColumnData::from_sections(pos, bounds, &sections).expect("packs");
        let frame = c.encode_frame().expect("fits");
        let back = ColumnData::decode(&frame[4..]).expect("decodes");
        assert_eq!(back, c);
        assert_eq!(back.pos(), pos);
        assert_eq!(back.bounds(), bounds);
        let got = back.sections().expect("unpacks");
        assert_eq!(got.len(), sections.len());
        let (mut a, mut b) = ([BlockStateId::AIR; 4096], [BlockStateId::AIR; 4096]);
        for (x, y) in got.iter().zip(&sections) {
            x.unpack_into(&mut a);
            y.unpack_into(&mut b);
            assert_eq!(a, b);
        }
    }

    #[test]
    fn wrong_counts_are_errors() {
        let (pos, bounds, sections) = sample();
        assert_eq!(
            ColumnData::from_sections(pos, bounds, &sections[..3]),
            Err(ColumnError::SectionCount)
        );
        let mut c = ColumnData::from_sections(pos, bounds, &sections).expect("packs");
        c.section_count = 3;
        assert_eq!(c.sections().err(), Some(ColumnError::SectionCount));
        c.section_count = 5;
        assert!(c.sections().is_err());
        c.section_count = u32::MAX;
        assert_eq!(c.sections().err(), Some(ColumnError::SectionCount));
        c.section_count = 4;
        c.payload.truncate(c.payload.len() / 2);
        assert_eq!(c.sections().err(), Some(ColumnError::Compression));
    }
}
