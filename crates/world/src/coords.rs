//! World coordinate types. All conversions use Euclidean division so negative
//! coordinates map to the correct section (block -1 is in section -1, not 0).

use crate::{MESH_CHUNK_SIZE, SECTION_SIZE};

/// Absolute block position in a dimension.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockPos {
    /// East-west.
    pub x: i32,
    /// Up-down.
    pub y: i32,
    /// North-south.
    pub z: i32,
}

/// Position of a 16^3 storage section.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SectionPos {
    /// Section x = block x div 16.
    pub x: i32,
    /// Section y = block y div 16.
    pub y: i32,
    /// Section z = block z div 16.
    pub z: i32,
}

/// Position of a 32^3 mesh chunk (2x2x2 sections).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MeshChunkPos {
    /// Mesh chunk x = block x div 32.
    pub x: i32,
    /// Mesh chunk y = block y div 32.
    pub y: i32,
    /// Mesh chunk z = block z div 32.
    pub z: i32,
}

impl BlockPos {
    /// Creates a block position.
    #[must_use]
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    /// The storage section that contains this block.
    #[must_use]
    pub fn section(self) -> SectionPos {
        SectionPos {
            x: self.x.div_euclid(SECTION_SIZE),
            y: self.y.div_euclid(SECTION_SIZE),
            z: self.z.div_euclid(SECTION_SIZE),
        }
    }

    /// Index of this block inside its section, in YZX order (y major).
    #[must_use]
    pub fn index_in_section(self) -> usize {
        let lx = self.x.rem_euclid(SECTION_SIZE) as usize;
        let ly = self.y.rem_euclid(SECTION_SIZE) as usize;
        let lz = self.z.rem_euclid(SECTION_SIZE) as usize;
        (ly * 16 + lz) * 16 + lx
    }

    /// The mesh chunk that contains this block.
    #[must_use]
    pub fn mesh_chunk(self) -> MeshChunkPos {
        MeshChunkPos {
            x: self.x.div_euclid(MESH_CHUNK_SIZE),
            y: self.y.div_euclid(MESH_CHUNK_SIZE),
            z: self.z.div_euclid(MESH_CHUNK_SIZE),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negative_blocks_map_to_negative_sections() {
        assert_eq!(
            BlockPos::new(-1, -1, -1).section(),
            SectionPos {
                x: -1,
                y: -1,
                z: -1
            }
        );
        assert_eq!(
            BlockPos::new(-16, 0, 15).section(),
            SectionPos { x: -1, y: 0, z: 0 }
        );
        assert_eq!(
            BlockPos::new(-17, 0, 16).section(),
            SectionPos { x: -2, y: 0, z: 1 }
        );
    }

    #[test]
    fn index_in_section_covers_0_to_4095() {
        assert_eq!(BlockPos::new(0, 0, 0).index_in_section(), 0);
        assert_eq!(BlockPos::new(15, 15, 15).index_in_section(), 4095);
        assert_eq!(BlockPos::new(-1, -1, -1).index_in_section(), 4095);
    }

    #[test]
    fn mesh_chunk_is_two_sections_wide() {
        assert_eq!(BlockPos::new(31, 0, 0).mesh_chunk().x, 0);
        assert_eq!(BlockPos::new(32, 0, 0).mesh_chunk().x, 1);
        assert_eq!(BlockPos::new(-1, 0, 0).mesh_chunk().x, -1);
    }
}
