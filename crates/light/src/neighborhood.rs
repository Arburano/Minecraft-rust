//! Light on a 3x3x3 section snapshot: full BFS and incremental add/remove.
//!
//! Model (identical for the block and the sky channel, see `lighting.md`):
//! every cell has a source level `src` (block emission, or 15 for cells in the
//! column's exposed sky run) and an opacity `op` in `0..=15`. Light is the least
//! fixed point of
//!
//! ```text
//! L(c) = max(src(c), max over the 6 neighbors n of L(n) - 1 - op(c)), clamped at 0
//! ```
//!
//! inside the snapshot; cells outside it count as dark. Light travels at most 15
//! blocks, so the **center section** is exact (no source outside the snapshot
//! can reach it); the outer ring is a boundary approximation and is recomputed
//! by the neighboring snapshot. Full and incremental relighting compute the
//! same fixed point bit for bit.
//!
//! Storage is a padded 50^3 grid (one sentinel cell of opacity 15 on every
//! side), so the BFS steps by constant index offsets without bounds checks on
//! coordinates. Queues are reused between edits: after warm-up an edit does
//! not allocate.

use std::fmt;

use world::coords::{BlockPos, SectionPos};
use world::{BlockStateId, SECTION_SIZE, SECTION_VOLUME, Section};

use crate::nibbles::{MAX_LIGHT, Nibbles, SectionLight};

/// Edge of the snapshot in blocks (3 sections).
pub const EDGE: usize = 3 * SECTION_SIZE as usize;
/// Cells in the snapshot.
pub const VOLUME: usize = EDGE * EDGE * EDGE;
/// Columns in the snapshot (`EDGE * EDGE`), indexed `z * EDGE + x`.
pub const COLUMNS: usize = EDGE * EDGE;

const P: usize = EDGE + 2;
const P2: usize = P * P;
const P3: usize = P2 * P;
const NEIGHBORS: [usize; 6] = [
    1,
    1usize.wrapping_neg(),
    P,
    P.wrapping_neg(),
    P2,
    P2.wrapping_neg(),
];
const LEVEL_SHIFT: u32 = 24;
const INDEX_MASK: u32 = (1 << LEVEL_SHIFT) - 1;

/// What the engine needs to know about a block for lighting. Supplied by the
/// caller (registry data from mods); the engine does not know specific blocks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct LightProps {
    /// Emitted block light, `0..=15`.
    pub emission: u8,
    /// Extra attenuation when light enters this block: 0 for air and glass,
    /// 15 for fully opaque blocks. Anything above 0 also stops direct sky.
    pub opacity: u8,
}

impl LightProps {
    /// Air: no emission, no attenuation.
    pub const AIR: Self = Self::new(0, 0);
    /// A fully opaque, non-emitting block.
    pub const OPAQUE: Self = Self::new(0, MAX_LIGHT);

    /// Creates properties, clamping both values to `0..=15`.
    #[must_use]
    pub const fn new(emission: u8, opacity: u8) -> Self {
        Self {
            emission: if emission > MAX_LIGHT {
                MAX_LIGHT
            } else {
                emission
            },
            opacity: if opacity > MAX_LIGHT {
                MAX_LIGHT
            } else {
                opacity
            },
        }
    }
}

/// Lighting errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightError {
    /// The position is outside the 3x3x3 snapshot.
    OutOfBounds(BlockPos),
    /// [`LightNeighborhood::relight_add`] was given a change that can darken
    /// cells (less emission or more opacity); use `relight_remove`.
    WouldDecrease,
    /// An input slice has the wrong length.
    BadLength {
        /// Expected number of elements.
        expected: usize,
        /// Supplied number of elements.
        actual: usize,
    },
}

impl fmt::Display for LightError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfBounds(p) => write!(f, "block {p:?} is outside the light snapshot"),
            Self::WouldDecrease => write!(f, "relight_add cannot apply a darkening change"),
            Self::BadLength { expected, actual } => {
                write!(f, "expected {expected} elements, got {actual}")
            }
        }
    }
}

impl std::error::Error for LightError {}

struct Channel {
    light: Box<[u8]>,
    src: Box<[u8]>,
}

impl Channel {
    fn new() -> Self {
        Self {
            light: vec![0; P3].into_boxed_slice(),
            src: vec![0; P3].into_boxed_slice(),
        }
    }
}

/// Light of a 3x3x3 section snapshot around a center section.
pub struct LightNeighborhood {
    origin: BlockPos,
    opacity: Box<[u8]>,
    block: Channel,
    sky: Channel,
    open_top: Box<[bool]>,
    /// Lowest local y of the run exposed to direct sky (`EDGE` = none in the snapshot).
    sky_floor: Box<[u8]>,
    inc: Vec<u32>,
    dec: Vec<u32>,
}

#[inline]
fn padded(x: usize, y: usize, z: usize) -> usize {
    ((y + 1) * P + (z + 1)) * P + (x + 1)
}

impl LightNeighborhood {
    /// Builds a snapshot from the 27 sections around `center`, all light zero.
    /// Call [`Self::full_relight`] before reading light.
    ///
    /// - `sections[(dy * 3 + dz) * 3 + dx]` is the section at `center + (dx-1, dy-1, dz-1)`.
    /// - `heightmap[z * 48 + x]` is, per column of the snapshot, the world y from
    ///   which the column is open to the sky (topmost block with opacity > 0,
    ///   plus one). Only "is the column open above the snapshot" is taken from
    ///   it; the exposed run inside the snapshot is derived from the blocks, so
    ///   edits keep it consistent.
    /// - `props` maps a block state to its light properties.
    pub fn new(
        center: SectionPos,
        sections: [&Section; 27],
        heightmap: &[i32],
        props: impl Fn(BlockStateId) -> LightProps,
    ) -> Result<Self, LightError> {
        if heightmap.len() != COLUMNS {
            return Err(LightError::BadLength {
                expected: COLUMNS,
                actual: heightmap.len(),
            });
        }
        let origin = BlockPos::new(
            (center.x - 1) * SECTION_SIZE,
            (center.y - 1) * SECTION_SIZE,
            (center.z - 1) * SECTION_SIZE,
        );
        let top = origin.y + EDGE as i32;
        let open_top: Vec<bool> = heightmap.iter().map(|&h| h <= top).collect();
        let mut cells = vec![LightProps::OPAQUE; VOLUME];
        let mut ids = [BlockStateId::AIR; SECTION_VOLUME];
        let s = SECTION_SIZE as usize;
        for (i, section) in sections.iter().enumerate() {
            let (dx, dz, dy) = (i % 3, i / 3 % 3, i / 9);
            section.unpack_into(&mut ids);
            for (j, &id) in ids.iter().enumerate() {
                let (x, z, y) = (dx * s + j % s, dz * s + j / s % s, dy * s + j / (s * s));
                cells[(y * EDGE + z) * EDGE + x] = props(id);
            }
        }
        Self::from_props(origin, &cells, &open_top)
    }

    /// Builds a snapshot directly from per-cell properties (`cells[(y*48+z)*48+x]`)
    /// and per-column "open to the sky above the snapshot" flags
    /// (`open_top[z*48+x]`). `origin` is the world position of local `(0, 0, 0)`.
    pub fn from_props(
        origin: BlockPos,
        cells: &[LightProps],
        open_top: &[bool],
    ) -> Result<Self, LightError> {
        if cells.len() != VOLUME {
            return Err(LightError::BadLength {
                expected: VOLUME,
                actual: cells.len(),
            });
        }
        if open_top.len() != COLUMNS {
            return Err(LightError::BadLength {
                expected: COLUMNS,
                actual: open_top.len(),
            });
        }
        let mut n = Self {
            origin,
            opacity: vec![MAX_LIGHT; P3].into_boxed_slice(),
            block: Channel::new(),
            sky: Channel::new(),
            open_top: open_top.to_vec().into_boxed_slice(),
            sky_floor: vec![EDGE as u8; COLUMNS].into_boxed_slice(),
            inc: Vec::with_capacity(2 * VOLUME),
            dec: Vec::with_capacity(VOLUME),
        };
        for y in 0..EDGE {
            for z in 0..EDGE {
                for x in 0..EDGE {
                    let c = cells[(y * EDGE + z) * EDGE + x];
                    let i = padded(x, y, z);
                    n.opacity[i] = c.opacity;
                    n.block.src[i] = c.emission;
                }
            }
        }
        for z in 0..EDGE {
            for x in 0..EDGE {
                let floor = n.scan_floor(x, z, EDGE);
                n.sky_floor[z * EDGE + x] = floor as u8;
                for y in floor..EDGE {
                    n.sky.src[padded(x, y, z)] = MAX_LIGHT;
                }
            }
        }
        Ok(n)
    }

    /// World position of local `(0, 0, 0)`.
    #[must_use]
    pub fn origin(&self) -> BlockPos {
        self.origin
    }

    /// Recomputes both channels from scratch (BFS from every source).
    pub fn full_relight(&mut self) {
        for channel in [&mut self.block, &mut self.sky] {
            channel.light.fill(0);
            self.inc.clear();
            for y in 0..EDGE {
                for z in 0..EDGE {
                    for x in 0..EDGE {
                        let i = padded(x, y, z);
                        let s = channel.src[i];
                        if s > 0 {
                            channel.light[i] = s;
                            self.inc.push(i as u32);
                        }
                    }
                }
            }
            propagate_increase(channel, &self.opacity, &mut self.inc);
        }
    }

    /// Applies a block change that cannot darken anything (emission does not
    /// drop and opacity does not rise): a single BFS from the changed cell.
    ///
    /// Returns [`LightError::WouldDecrease`] without changing anything
    /// otherwise; such changes need [`Self::relight_remove`].
    pub fn relight_add(&mut self, pos: BlockPos, props: LightProps) -> Result<(), LightError> {
        let i = self.index(pos)?;
        let props = LightProps::new(props.emission, props.opacity);
        if props.emission < self.block.src[i] || props.opacity > self.opacity[i] {
            return Err(LightError::WouldDecrease);
        }
        self.apply(pos, i, props);
        Ok(())
    }

    /// Applies any block change, including darkening ones: reverse BFS clears
    /// everything that may have been lit through the changed cell, collects the
    /// boundary with other sources and refills from it.
    pub fn relight_remove(&mut self, pos: BlockPos, props: LightProps) -> Result<(), LightError> {
        let i = self.index(pos)?;
        self.apply(pos, i, LightProps::new(props.emission, props.opacity));
        Ok(())
    }

    /// Applies a block change, choosing the cheapest correct update.
    /// Equivalent to [`Self::relight_remove`]; the decrease pass is skipped
    /// automatically when nothing can get darker.
    pub fn set_block(&mut self, pos: BlockPos, props: LightProps) -> Result<(), LightError> {
        self.relight_remove(pos, props)
    }

    /// Block light at `pos`, or `None` outside the snapshot.
    #[must_use]
    pub fn block_light(&self, pos: BlockPos) -> Option<u8> {
        self.index(pos).ok().map(|i| self.block.light[i])
    }

    /// Sky light at `pos`, or `None` outside the snapshot.
    #[must_use]
    pub fn sky_light(&self, pos: BlockPos) -> Option<u8> {
        self.index(pos).ok().map(|i| self.sky.light[i])
    }

    /// Exports one section of the snapshot (`d* in 0..3`, 1 = center) as
    /// compact [`SectionLight`]. Only the center section is exact.
    #[must_use]
    pub fn section_light(&self, dx: usize, dy: usize, dz: usize) -> SectionLight {
        let s = SECTION_SIZE as usize;
        let mut block = [0u8; SECTION_VOLUME];
        let mut sky = [0u8; SECTION_VOLUME];
        for j in 0..SECTION_VOLUME {
            let i = padded(
                dx.min(2) * s + j % s,
                dy.min(2) * s + j / (s * s),
                dz.min(2) * s + j / s % s,
            );
            block[j] = self.block.light[i];
            sky[j] = self.sky.light[i];
        }
        SectionLight {
            block: Nibbles::from_levels(&block),
            sky: Nibbles::from_levels(&sky),
        }
    }

    /// Block light of every cell, YZX over 48^3. Allocates; for tests and tools.
    #[must_use]
    pub fn block_levels(&self) -> Vec<u8> {
        self.levels(&self.block)
    }

    /// Sky light of every cell, YZX over 48^3. Allocates; for tests and tools.
    #[must_use]
    pub fn sky_levels(&self) -> Vec<u8> {
        self.levels(&self.sky)
    }

    fn levels(&self, channel: &Channel) -> Vec<u8> {
        let mut out = Vec::with_capacity(VOLUME);
        for y in 0..EDGE {
            for z in 0..EDGE {
                for x in 0..EDGE {
                    out.push(channel.light[padded(x, y, z)]);
                }
            }
        }
        out
    }

    fn index(&self, pos: BlockPos) -> Result<usize, LightError> {
        let (x, y, z) = (
            pos.x - self.origin.x,
            pos.y - self.origin.y,
            pos.z - self.origin.z,
        );
        let edge = EDGE as i32;
        if (0..edge).contains(&x) && (0..edge).contains(&y) && (0..edge).contains(&z) {
            Ok(padded(x as usize, y as usize, z as usize))
        } else {
            Err(LightError::OutOfBounds(pos))
        }
    }

    /// Lowest y such that every cell in `y..top` of the column has opacity 0.
    fn scan_floor(&self, x: usize, z: usize, top: usize) -> usize {
        if !self.open_top[z * EDGE + x] {
            return EDGE;
        }
        let mut floor = top;
        while floor > 0 && self.opacity[padded(x, floor - 1, z)] == 0 {
            floor -= 1;
        }
        floor
    }

    fn apply(&mut self, pos: BlockPos, i: usize, props: LightProps) {
        let old_emission = self.block.src[i];
        let old_opacity = self.opacity[i];
        self.opacity[i] = props.opacity;
        self.block.src[i] = props.emission;
        let opacity_up = props.opacity > old_opacity;

        // Block channel.
        self.inc.clear();
        self.dec.clear();
        if props.emission < old_emission || opacity_up {
            seed_decrease(&mut self.block, i, &mut self.inc, &mut self.dec);
        }
        self.finish(true, i);

        // Sky channel: the changed column's exposed run may move.
        let (x, y, z) = (
            (pos.x - self.origin.x) as usize,
            (pos.y - self.origin.y) as usize,
            (pos.z - self.origin.z) as usize,
        );
        let col = z * EDGE + x;
        let old_floor = self.sky_floor[col] as usize;
        let new_floor = if !self.open_top[col] {
            EDGE
        } else if props.opacity > 0 && y >= old_floor {
            y + 1
        } else if props.opacity == 0 && old_opacity > 0 && y + 1 == old_floor {
            self.scan_floor(x, z, old_floor)
        } else {
            old_floor
        };
        self.sky_floor[col] = new_floor as u8;
        self.inc.clear();
        self.dec.clear();
        for cy in old_floor..new_floor {
            let c = padded(x, cy, z);
            self.sky.src[c] = 0;
            seed_decrease(&mut self.sky, c, &mut self.inc, &mut self.dec);
        }
        for cy in new_floor..old_floor {
            let c = padded(x, cy, z);
            self.sky.src[c] = MAX_LIGHT;
            self.inc.push(c as u32);
        }
        if opacity_up && y < old_floor {
            seed_decrease(&mut self.sky, i, &mut self.inc, &mut self.dec);
        }
        self.finish(false, i);
    }

    /// Runs the decrease pass (if seeded), re-seeds sources and refills.
    fn finish(&mut self, block: bool, changed: usize) {
        let channel = if block {
            &mut self.block
        } else {
            &mut self.sky
        };
        propagate_decrease(channel, &mut self.inc, &mut self.dec);
        // Light may now flow into the changed cell (lower opacity) or out of it.
        self.inc.push(changed as u32);
        for off in NEIGHBORS {
            self.inc.push(changed.wrapping_add(off) as u32);
        }
        for &c in &self.inc {
            let c = c as usize;
            if channel.light[c] < channel.src[c] {
                channel.light[c] = channel.src[c];
            }
        }
        propagate_increase(channel, &self.opacity, &mut self.inc);
    }
}

/// Clears cell `c` and queues it for the reverse BFS (and for re-seeding).
fn seed_decrease(channel: &mut Channel, c: usize, inc: &mut Vec<u32>, dec: &mut Vec<u32>) {
    let level = channel.light[c];
    if level > 0 {
        channel.light[c] = 0;
        dec.push(c as u32 | (u32::from(level) << LEVEL_SHIFT));
    }
    inc.push(c as u32);
}

/// Reverse BFS: zero every cell that may have been lit through a cleared
/// cell; brighter neighbors (independent light) become the refill boundary.
fn propagate_decrease(channel: &mut Channel, inc: &mut Vec<u32>, dec: &mut Vec<u32>) {
    let mut head = 0;
    while head < dec.len() {
        let entry = dec[head];
        head += 1;
        let c = (entry & INDEX_MASK) as usize;
        let level = (entry >> LEVEL_SHIFT) as u8;
        for off in NEIGHBORS {
            let n = c.wrapping_add(off);
            let ln = channel.light[n];
            if ln == 0 {
                continue;
            }
            if ln < level {
                channel.light[n] = 0;
                dec.push(n as u32 | (u32::from(ln) << LEVEL_SHIFT));
                if channel.src[n] > 0 {
                    inc.push(n as u32);
                }
            } else {
                inc.push(n as u32);
            }
        }
    }
    dec.clear();
}

/// Forward BFS from every queued cell; each step costs 1 + the target's opacity.
fn propagate_increase(channel: &mut Channel, opacity: &[u8], inc: &mut Vec<u32>) {
    let mut head = 0;
    while head < inc.len() {
        let c = inc[head] as usize;
        head += 1;
        let level = channel.light[c];
        if level <= 1 {
            continue;
        }
        for off in NEIGHBORS {
            let n = c.wrapping_add(off);
            let candidate = level.saturating_sub(1 + opacity[n]);
            if candidate > channel.light[n] {
                channel.light[n] = candidate;
                inc.push(n as u32);
            }
        }
    }
    inc.clear();
}
