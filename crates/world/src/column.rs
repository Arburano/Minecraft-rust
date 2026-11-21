//! Chunk columns: a stack of sections, the sky heightmap and the loading state
//! (`world-storage.md`, "Column and loading"). Light is stored apart from blocks
//! under the same `SectionPos` key by the `light` crate (`lighting.md`), so the
//! dependency direction `light → world` is kept.

use crate::coords::{BlockPos, SectionPos};
use crate::registry::Registry;
use crate::{BlockStateId, SECTION_SIZE, Section};

/// Columns per axis inside a column: 16 x 16 block columns.
const COLUMN_AREA: usize = (SECTION_SIZE * SECTION_SIZE) as usize;

/// Vertical extent of a dimension, in whole sections.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WorldBounds {
    /// Lowest section y.
    pub min_section_y: i32,
    /// Number of sections in a column.
    pub section_count: u32,
}

impl Default for WorldBounds {
    /// y in `-64..320`: 24 sections, `DEFAULT_WORLD_HEIGHT` blocks.
    fn default() -> Self {
        Self {
            min_section_y: -4,
            section_count: (crate::DEFAULT_WORLD_HEIGHT / SECTION_SIZE) as u32,
        }
    }
}

impl WorldBounds {
    /// Lowest block y.
    #[must_use]
    pub fn min_y(self) -> i32 {
        self.min_section_y * SECTION_SIZE
    }

    /// One past the highest block y.
    #[must_use]
    pub fn max_y(self) -> i32 {
        (self.min_section_y + self.section_count as i32) * SECTION_SIZE
    }

    /// Index of section y `sy` inside a column, if in range.
    #[must_use]
    pub fn section_index(self, sy: i32) -> Option<usize> {
        let i = sy.checked_sub(self.min_section_y)?;
        (0..self.section_count as i32)
            .contains(&i)
            .then_some(i as usize)
    }
}

/// Position of a 16 x height x 16 column.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ColumnPos {
    /// Column x = block x div 16.
    pub x: i32,
    /// Column z = block z div 16.
    pub z: i32,
}

impl ColumnPos {
    /// The column that contains a block.
    #[must_use]
    pub fn of_block(pos: BlockPos) -> Self {
        Self {
            x: pos.x.div_euclid(SECTION_SIZE),
            z: pos.z.div_euclid(SECTION_SIZE),
        }
    }

    /// The column that contains a section.
    #[must_use]
    pub fn of_section(pos: SectionPos) -> Self {
        Self { x: pos.x, z: pos.z }
    }
}

/// Loading state of a column. Meshing and simulation run only on `Ready`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ColumnState {
    /// Waiting in the loading queue.
    #[default]
    Requested,
    /// Being generated or read from disk.
    Generating,
    /// Blocks present and initial light computed.
    Lit,
    /// Fully available.
    Ready,
    /// Being saved and removed.
    Unloading,
}

/// A column of sections with its sky heightmap.
#[derive(Clone)]
pub struct Column {
    pos: ColumnPos,
    bounds: WorldBounds,
    sections: Box<[Section]>,
    /// Per `(z * 16 + x)`: world y of the lowest block from which the column is
    /// open to the sky (topmost block with `light_opacity > 0`, plus one).
    heightmap: Box<[i32; COLUMN_AREA]>,
    /// Loading state, managed by the loader.
    pub state: ColumnState,
}

impl Column {
    /// An all-air column.
    #[must_use]
    pub fn new(pos: ColumnPos, bounds: WorldBounds) -> Self {
        Self {
            pos,
            bounds,
            sections: (0..bounds.section_count).map(|_| Section::new()).collect(),
            heightmap: Box::new([bounds.min_y(); COLUMN_AREA]),
            state: ColumnState::default(),
        }
    }

    /// A column from generated or loaded sections (bottom to top). The heightmap
    /// is computed from `registry`. Returns `None` if the section count does not
    /// match `bounds`.
    #[must_use]
    pub fn from_sections(
        pos: ColumnPos,
        bounds: WorldBounds,
        sections: Vec<Section>,
        registry: &Registry,
    ) -> Option<Self> {
        if sections.len() != bounds.section_count as usize {
            return None;
        }
        let mut column = Self {
            pos,
            bounds,
            sections: sections.into_boxed_slice(),
            heightmap: Box::new([bounds.min_y(); COLUMN_AREA]),
            state: ColumnState::default(),
        };
        column.recompute_heightmap(registry);
        Some(column)
    }

    /// Column position.
    #[must_use]
    pub fn pos(&self) -> ColumnPos {
        self.pos
    }

    /// Vertical bounds.
    #[must_use]
    pub fn bounds(&self) -> WorldBounds {
        self.bounds
    }

    /// Sections bottom to top.
    #[must_use]
    pub fn sections(&self) -> &[Section] {
        &self.sections
    }

    /// The section at section y `sy`, if inside the bounds.
    #[must_use]
    pub fn section(&self, sy: i32) -> Option<&Section> {
        self.bounds.section_index(sy).map(|i| &self.sections[i])
    }

    /// Sky heightmap value at local `(x, z)` (`0..16`): world y from which the
    /// column is open to the sky.
    #[must_use]
    pub fn height(&self, x: usize, z: usize) -> i32 {
        self.heightmap[(z & 15) * 16 + (x & 15)]
    }

    /// Block at local `x, z` (`0..16`) and world `y`; `None` outside the bounds.
    #[inline]
    #[must_use]
    pub fn get(&self, x: usize, y: i32, z: usize) -> Option<BlockStateId> {
        let s = self.bounds.section_index(y.div_euclid(SECTION_SIZE))?;
        let ly = y.rem_euclid(SECTION_SIZE) as usize;
        Some(self.sections[s].get((ly * 16 + (z & 15)) * 16 + (x & 15)))
    }

    /// Sets a block and keeps the heightmap current. Returns the previous
    /// state, or `None` (and changes nothing) outside the bounds.
    pub fn set(
        &mut self,
        x: usize,
        y: i32,
        z: usize,
        id: BlockStateId,
        registry: &Registry,
    ) -> Option<BlockStateId> {
        let (x, z) = (x & 15, z & 15);
        let s = self.bounds.section_index(y.div_euclid(SECTION_SIZE))?;
        let ly = y.rem_euclid(SECTION_SIZE) as usize;
        let index = (ly * 16 + z) * 16 + x;
        let old = self.sections[s].get(index);
        if old == id {
            return Some(old);
        }
        self.sections[s].set(index, id);
        let h = &mut self.heightmap[z * 16 + x];
        if registry.props(id).light_opacity > 0 {
            if y + 1 > *h {
                *h = y + 1;
            }
        } else if y + 1 == *h {
            let mut top = y;
            while top > self.bounds.min_y() {
                let below = top - 1;
                let blocked = self
                    .get(x, below, z)
                    .is_some_and(|b| registry.props(b).light_opacity > 0);
                if blocked {
                    break;
                }
                top = below;
            }
            self.heightmap[z * 16 + x] = top;
        }
        Some(old)
    }

    /// Recomputes the heightmap from the blocks (after generation or loading).
    pub fn recompute_heightmap(&mut self, registry: &Registry) {
        let (min_y, max_y) = (self.bounds.min_y(), self.bounds.max_y());
        for z in 0..16 {
            for x in 0..16 {
                let mut h = min_y;
                let mut y = max_y;
                while y > min_y {
                    y -= 1;
                    if self
                        .get(x, y, z)
                        .is_some_and(|b| registry.props(b).light_opacity > 0)
                    {
                        h = y + 1;
                        break;
                    }
                }
                self.heightmap[z * 16 + x] = h;
            }
        }
    }
}
