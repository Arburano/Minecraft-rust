//! Per-section 4-bit light storage (`docs/architecture/lighting.md`, "Data").

use world::SECTION_VOLUME;

/// Bytes of a packed section: two 4-bit levels per byte.
pub const PACKED_BYTES: usize = SECTION_VOLUME / 2;

/// Maximum light level.
pub const MAX_LIGHT: u8 = 15;

/// One light channel of a 16^3 section.
///
/// Uniform sections (all sky 15, all dark 0) are a single byte; everything else
/// is 2,048 bytes, two levels per byte, low nibble = even index. Cell indices
/// are YZX, the same as [`world::Section`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Nibbles {
    /// Every cell has this level.
    Single(u8),
    /// Two levels per byte.
    Packed(Box<[u8; PACKED_BYTES]>),
}

impl Default for Nibbles {
    fn default() -> Self {
        Self::Single(0)
    }
}

impl Nibbles {
    /// Builds the most compact representation of 4,096 levels (each `<= 15`).
    #[must_use]
    pub fn from_levels(levels: &[u8; SECTION_VOLUME]) -> Self {
        let first = levels[0] & MAX_LIGHT;
        if levels.iter().all(|&l| l & MAX_LIGHT == first) {
            return Self::Single(first);
        }
        let mut packed = Box::new([0u8; PACKED_BYTES]);
        for (byte, pair) in packed.iter_mut().zip(levels.chunks_exact(2)) {
            *byte = (pair[0] & MAX_LIGHT) | ((pair[1] & MAX_LIGHT) << 4);
        }
        Self::Packed(packed)
    }

    /// Level of cell `index` (YZX). Panics if `index >= 4096`.
    #[must_use]
    pub fn get(&self, index: usize) -> u8 {
        assert!(index < SECTION_VOLUME, "light index out of range");
        match self {
            Self::Single(level) => *level,
            Self::Packed(packed) => (packed[index / 2] >> ((index & 1) * 4)) & MAX_LIGHT,
        }
    }

    /// Sets cell `index` (YZX) to `level` (clamped to 15), expanding a uniform
    /// section when needed. Panics if `index >= 4096`.
    pub fn set(&mut self, index: usize, level: u8) {
        assert!(index < SECTION_VOLUME, "light index out of range");
        let level = level.min(MAX_LIGHT);
        if let Self::Single(current) = *self {
            if current == level {
                return;
            }
            *self = Self::Packed(Box::new([current | (current << 4); PACKED_BYTES]));
        }
        if let Self::Packed(packed) = self {
            let shift = (index & 1) * 4;
            let byte = &mut packed[index / 2];
            *byte = (*byte & !(MAX_LIGHT << shift)) | (level << shift);
        }
    }

    /// Collapses a packed section whose cells are all equal into [`Nibbles::Single`].
    pub fn compact(&mut self) {
        if let Self::Packed(packed) = self {
            let first = packed[0] & MAX_LIGHT;
            let byte = first | (first << 4);
            if packed.iter().all(|&b| b == byte) {
                *self = Self::Single(first);
            }
        }
    }

    /// Heap + inline size of this channel's data in bytes (1 or 2,048).
    #[must_use]
    pub fn storage_bytes(&self) -> usize {
        match self {
            Self::Single(_) => 1,
            Self::Packed(_) => PACKED_BYTES,
        }
    }
}

/// Block and sky light of one section, stored apart from the blocks under the
/// same `SectionPos` key.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SectionLight {
    /// Light emitted by blocks.
    pub block: Nibbles,
    /// Light from the sky.
    pub sky: Nibbles,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_levels_are_single() {
        assert_eq!(
            Nibbles::from_levels(&[15; SECTION_VOLUME]),
            Nibbles::Single(15)
        );
        assert_eq!(
            Nibbles::from_levels(&[0; SECTION_VOLUME]).storage_bytes(),
            1
        );
    }

    #[test]
    fn packed_round_trip_and_compact() {
        let mut levels = [0u8; SECTION_VOLUME];
        for (i, l) in levels.iter_mut().enumerate() {
            *l = (i * 7 % 16) as u8;
        }
        let mut n = Nibbles::from_levels(&levels);
        assert_eq!(n.storage_bytes(), PACKED_BYTES);
        for (i, &l) in levels.iter().enumerate() {
            assert_eq!(n.get(i), l);
        }
        for i in 0..SECTION_VOLUME {
            n.set(i, 9);
        }
        n.compact();
        assert_eq!(n, Nibbles::Single(9));
    }

    #[test]
    fn set_expands_single_and_keeps_neighbors() {
        let mut n = Nibbles::Single(15);
        n.set(4095, 15);
        assert_eq!(n, Nibbles::Single(15));
        n.set(1, 3);
        assert_eq!((n.get(0), n.get(1), n.get(2)), (15, 3, 15));
        n.set(0, 200);
        assert_eq!(n.get(0), 15, "levels clamp to 15");
    }
}
