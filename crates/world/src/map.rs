//! The map of loaded columns. Owned by one thread (the client's main thread, a
//! server region); there is no lock around the world (AGENTS.md rule 3).
//! Readers on worker threads get section snapshots, never the map.

use std::collections::HashMap;
use std::fmt;
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::Arc;

use crate::column::{Column, ColumnPos, WorldBounds};
use crate::coords::{BlockPos, SectionPos};
use crate::registry::Registry;
use crate::{BlockStateId, SECTION_SIZE};

/// Which consumer a dirty mark is for. Each consumer drains its own list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DirtyKind {
    /// Re-mesh (the consumer also re-meshes neighbors when the edit is on a border).
    Mesh = 0,
    /// Re-light.
    Light = 1,
    /// Save to disk.
    Save = 2,
}

const DIRTY_KINDS: usize = 3;

/// Errors of world edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorldError {
    /// The column is not loaded.
    NotLoaded(ColumnPos),
    /// The y coordinate is outside the dimension's bounds.
    OutOfHeight(BlockPos),
    /// The state is not in the registry.
    UnknownState(BlockStateId),
    /// The column was built with different bounds than the map.
    BoundsMismatch(ColumnPos),
}

impl fmt::Display for WorldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotLoaded(c) => write!(f, "column {c:?} is not loaded"),
            Self::OutOfHeight(p) => write!(f, "block {p:?} is outside the world height"),
            Self::UnknownState(id) => write!(f, "block state {id:?} is not registered"),
            Self::BoundsMismatch(c) => write!(f, "column {c:?} has different world bounds"),
        }
    }
}

impl std::error::Error for WorldError {}

/// A small multiplicative hasher for integer keys (positions). The std SipHash
/// is DoS-resistant but several times slower; keys here are not attacker
/// controlled per lookup.
#[derive(Default)]
pub struct PosHasher(u64);

impl Hasher for PosHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.write_u64(u64::from(b));
        }
    }

    fn write_i32(&mut self, i: i32) {
        self.write_u64(u64::from(i as u32));
    }

    fn write_u64(&mut self, i: u64) {
        self.0 = (self.0.rotate_left(5) ^ i).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
}

type PosMap<K, V> = HashMap<K, V, BuildHasherDefault<PosHasher>>;

/// Edge of the direct-mapped slot grid in columns (power of two). A column
/// lives in slot `(x mod 64, z mod 64)`, so a loaded area up to 64 columns
/// wide (1,024 blocks) never collides; colliding columns go to an overflow map.
const GRID: usize = 64;
const GRID_MASK: i32 = GRID as i32 - 1;

#[inline]
fn slot(pos: ColumnPos) -> usize {
    ((pos.z & GRID_MASK) as usize) * GRID + (pos.x & GRID_MASK) as usize
}

struct Loaded {
    column: Column,
    /// Per section: bit `k` set = already queued for `DirtyKind` `k`.
    dirty: Box<[u8]>,
}

/// Loaded columns of one dimension, with dirty-section tracking.
pub struct WorldMap {
    registry: Arc<Registry>,
    bounds: WorldBounds,
    slots: Box<[Option<Loaded>]>,
    overflow: PosMap<ColumnPos, Box<Loaded>>,
    count: usize,
    dirty: [Vec<SectionPos>; DIRTY_KINDS],
}

/// Finds a loaded column: the direct slot first, the overflow map only when
/// the slot holds another column.
#[inline]
fn find_mut<'a>(
    slots: &'a mut [Option<Loaded>],
    overflow: &'a mut PosMap<ColumnPos, Box<Loaded>>,
    pos: ColumnPos,
) -> Option<&'a mut Loaded> {
    let i = slot(pos);
    if slots[i].as_ref().is_some_and(|l| l.column.pos() == pos) {
        return slots[i].as_mut();
    }
    if overflow.is_empty() {
        return None;
    }
    overflow.get_mut(&pos).map(|l| &mut **l)
}

impl WorldMap {
    /// An empty map for a dimension.
    #[must_use]
    pub fn new(registry: Arc<Registry>, bounds: WorldBounds) -> Self {
        Self {
            registry,
            bounds,
            slots: (0..GRID * GRID).map(|_| None).collect(),
            overflow: PosMap::default(),
            count: 0,
            dirty: [Vec::new(), Vec::new(), Vec::new()],
        }
    }

    /// The registry.
    #[must_use]
    pub fn registry(&self) -> &Arc<Registry> {
        &self.registry
    }

    /// Vertical bounds.
    #[must_use]
    pub fn bounds(&self) -> WorldBounds {
        self.bounds
    }

    /// Number of loaded columns.
    #[must_use]
    pub fn len(&self) -> usize {
        self.count
    }

    /// Whether no column is loaded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    #[inline]
    fn loaded(&self, pos: ColumnPos) -> Option<&Loaded> {
        match &self.slots[slot(pos)] {
            Some(l) if l.column.pos() == pos => Some(l),
            _ if self.overflow.is_empty() => None,
            _ => self.overflow.get(&pos).map(|l| &**l),
        }
    }

    /// Inserts a column, returning the one it replaces. New columns are not
    /// marked dirty: the loading pipeline lights and meshes them.
    pub fn insert_column(&mut self, column: Column) -> Result<Option<Column>, WorldError> {
        if column.bounds() != self.bounds {
            return Err(WorldError::BoundsMismatch(column.pos()));
        }
        let pos = column.pos();
        let dirty = vec![0u8; self.bounds.section_count as usize].into_boxed_slice();
        let new = Loaded { column, dirty };
        let old = match &mut self.slots[slot(pos)] {
            Some(l) if l.column.pos() == pos => Some(std::mem::replace(l, new)),
            Some(_) => self.overflow.insert(pos, Box::new(new)).map(|b| *b),
            empty @ None => {
                *empty = Some(new);
                None
            }
        };
        if old.is_none() {
            self.count += 1;
        }
        Ok(old.map(|l| l.column))
    }

    /// Removes a column. Its pending dirty marks are dropped when drained.
    pub fn remove_column(&mut self, pos: ColumnPos) -> Option<Column> {
        let i = slot(pos);
        let removed = if self.slots[i]
            .as_ref()
            .is_some_and(|l| l.column.pos() == pos)
        {
            // Move a column waiting in the overflow map into the freed slot.
            let waiting = self.overflow.keys().find(|p| slot(**p) == i).copied();
            let promoted = waiting.and_then(|p| self.overflow.remove(&p)).map(|b| *b);
            std::mem::replace(&mut self.slots[i], promoted)
        } else {
            self.overflow.remove(&pos).map(|b| *b)
        };
        if removed.is_some() {
            self.count -= 1;
        }
        removed.map(|l| l.column)
    }

    /// Positions of all loaded columns (any order).
    pub fn column_positions(&self) -> impl Iterator<Item = ColumnPos> + '_ {
        self.slots
            .iter()
            .filter_map(|s| s.as_ref().map(|l| l.column.pos()))
            .chain(self.overflow.keys().copied())
    }

    /// A loaded column.
    #[must_use]
    pub fn column(&self, pos: ColumnPos) -> Option<&Column> {
        self.loaded(pos).map(|l| &l.column)
    }

    /// Mutable access to a column's non-block state (its loading state).
    /// Block edits go through [`Self::set_block`] so dirty tracking stays exact.
    pub fn column_state_mut(&mut self, pos: ColumnPos) -> Option<&mut crate::column::ColumnState> {
        find_mut(&mut self.slots, &mut self.overflow, pos).map(|l| &mut l.column.state)
    }

    /// Block at `pos`; `None` if the column is not loaded or `y` is out of bounds.
    #[inline]
    #[must_use]
    pub fn get_block(&self, pos: BlockPos) -> Option<BlockStateId> {
        let loaded = self.loaded(ColumnPos::of_block(pos))?;
        loaded.column.get(
            pos.x.rem_euclid(SECTION_SIZE) as usize,
            pos.y,
            pos.z.rem_euclid(SECTION_SIZE) as usize,
        )
    }

    /// Sky heightmap at a block column, if loaded.
    #[must_use]
    pub fn height(&self, x: i32, z: i32) -> Option<i32> {
        let loaded = self.loaded(ColumnPos::of_block(BlockPos::new(x, 0, z)))?;
        Some(loaded.column.height(
            x.rem_euclid(SECTION_SIZE) as usize,
            z.rem_euclid(SECTION_SIZE) as usize,
        ))
    }

    /// Sets a block, updates the heightmap and marks the section dirty for every
    /// consumer when the block actually changed. Returns the previous state.
    pub fn set_block(
        &mut self,
        pos: BlockPos,
        id: BlockStateId,
    ) -> Result<BlockStateId, WorldError> {
        if !self.registry.contains(id) {
            return Err(WorldError::UnknownState(id));
        }
        let cpos = ColumnPos::of_block(pos);
        let Self {
            registry,
            bounds,
            slots,
            overflow,
            dirty,
            ..
        } = self;
        let loaded = find_mut(slots, overflow, cpos).ok_or(WorldError::NotLoaded(cpos))?;
        let old = loaded
            .column
            .set(
                pos.x.rem_euclid(SECTION_SIZE) as usize,
                pos.y,
                pos.z.rem_euclid(SECTION_SIZE) as usize,
                id,
                registry,
            )
            .ok_or(WorldError::OutOfHeight(pos))?;
        if old != id {
            if let Some(i) = bounds.section_index(pos.y.div_euclid(SECTION_SIZE)) {
                let flags = &mut loaded.dirty[i];
                for (k, list) in dirty.iter_mut().enumerate() {
                    if *flags & (1 << k) == 0 {
                        *flags |= 1 << k;
                        list.push(pos.section());
                    }
                }
            }
        }
        Ok(old)
    }

    /// Moves the sections dirty for `kind` into `out` (each once, in first-edit
    /// order) and clears their marks. Sections of removed columns are skipped.
    pub fn take_dirty(&mut self, kind: DirtyKind, out: &mut Vec<SectionPos>) {
        let k = kind as usize;
        let bounds = self.bounds;
        let Self {
            slots,
            overflow,
            dirty,
            ..
        } = self;
        for pos in dirty[k].drain(..) {
            let Some(loaded) = find_mut(slots, overflow, ColumnPos::of_section(pos)) else {
                continue;
            };
            let Some(i) = bounds.section_index(pos.y) else {
                continue;
            };
            if loaded.dirty[i] & (1 << k) != 0 {
                loaded.dirty[i] &= !(1 << k);
                out.push(pos);
            }
        }
    }
}
