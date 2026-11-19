//! Deterministic synthetic test terrain for benchmarks and tests (task T0-12).
//!
//! Spec: `docs/architecture/worldgen.md` ("Test corpus"),
//! `docs/process/testing-and-benchmarks.md` ("fixed data corpus").
//! Starting point: the generator in `research/bench/meshing`, as adapted by the
//! T0-04 benchmark corpus.
//!
//! This is **not** the game's final generator. It produces a heightmap + caves +
//! ores + trees terrain with ~8% transparent blocks (water, leaves) from a seed,
//! column by column. Every column is a pure function of `(seed, x, z)` and the
//! requested section y range: generation order and thread count never affect the
//! result ([`TestColumn::hash`] is the determinism check). Trees are confined to
//! their 32x32 placement cell (trunks at cell offset 3..=28, leaves reach at most
//! 2 blocks sideways), so no column ever needs a neighbor's data.
//!
//! The block IDs below are corpus-local test IDs, not registry semantics; the
//! engine's opacity still comes from the caller's predicate. For this corpus use
//! [`is_transparent`].

use std::ops::Range;

use noise::{NoiseFn, Perlin};
use world::{BlockStateId, SECTION_VOLUME, Section};

/// Corpus block IDs. Test-only; the real registry is assigned by mods.
pub mod ids {
    /// Air (always 0).
    pub const AIR: u16 = 0;
    /// Stone.
    pub const STONE: u16 = 1;
    /// Dirt (also tree trunks, as in the research corpus).
    pub const DIRT: u16 = 2;
    /// Grass surface.
    pub const GRASS: u16 = 3;
    /// Sand near and under the sea level.
    pub const SAND: u16 = 4;
    /// Water (transparent).
    pub const WATER: u16 = 5;
    /// Tree leaves (transparent).
    pub const LEAVES: u16 = 6;
    /// Glass (transparent); never generated, reserved for edit benchmarks.
    pub const GLASS: u16 = 7;
    /// Coal ore (opaque).
    pub const COAL_ORE: u16 = 8;
    /// Iron ore (opaque), only deep underground.
    pub const IRON_ORE: u16 = 9;
}

/// The corpus-local transparency predicate: water, leaves and glass.
#[must_use]
pub fn is_transparent(b: BlockStateId) -> bool {
    matches!(b.0, ids::WATER..=ids::GLASS)
}

/// Sea level in blocks; water fills air at `y <= SEA_LEVEL`.
pub const SEA_LEVEL: i32 = 44;
/// Mean terrain height in blocks.
const HEIGHT_BASE: f64 = 36.0;
/// Terrain height amplitude in blocks.
const HEIGHT_AMP: f64 = 16.0;
/// Tree placement cell edge in blocks (2x2 columns).
const TREE_CELL: i32 = 32;
/// Tree candidates per placement cell.
const TREES_PER_CELL: u32 = 24;
/// Iron ore only appears below this height.
const IRON_MAX_Y: i32 = 24;

// Domain separators so height, cave, tree and ore randomness never correlate.
const HEIGHT_DOMAIN: u64 = 1;
const CAVE_DOMAIN: u64 = 2;
const TREE_DOMAIN: u64 = 3;
const ORE_DOMAIN: u64 = 4;

/// A rectangular area in **section coordinates** (16-block units), end-exclusive.
///
/// `x` and `z` select the columns; `y` selects the section stack inside every
/// column. Negative coordinates are fine (Euclidean section mapping).
#[derive(Clone, Debug)]
pub struct CorpusArea {
    /// Column x range in section coordinates.
    pub x: Range<i32>,
    /// Section y range inside each column.
    pub y: Range<i32>,
    /// Column z range in section coordinates.
    pub z: Range<i32>,
}

impl CorpusArea {
    /// Number of columns in the area.
    #[must_use]
    pub fn column_count(&self) -> usize {
        let nx = (self.x.end - self.x.start).max(0) as usize;
        let nz = (self.z.end - self.z.start).max(0) as usize;
        nx * nz
    }
}

/// One generated 16x16 column: a stack of [`Section`]s over the requested y range.
#[derive(Clone)]
pub struct TestColumn {
    /// Column x in section coordinates.
    pub x: i32,
    /// Column z in section coordinates.
    pub z: i32,
    y_min: i32,
    sections: Vec<Section>,
}

impl TestColumn {
    /// The sections, bottom to top, starting at `y_range().start`.
    #[must_use]
    pub fn sections(&self) -> &[Section] {
        &self.sections
    }

    /// The section y range this column covers.
    #[must_use]
    pub fn y_range(&self) -> Range<i32> {
        self.y_min..self.y_min + self.sections.len() as i32
    }

    /// The section at section y `sy`. Panics when outside [`Self::y_range`].
    #[must_use]
    pub fn section(&self, sy: i32) -> &Section {
        &self.sections[(sy - self.y_min) as usize]
    }

    /// Content hash (FNV-1a 64) over the column coordinates and every block ID
    /// in a fixed order (sections bottom to top, YZX inside a section).
    ///
    /// Independent of the palette representation and of generation order; this
    /// is the determinism check required by `docs/architecture/worldgen.md`.
    #[must_use]
    pub fn hash(&self) -> u64 {
        let mut h = FNV_OFFSET;
        for v in [self.x, self.z, self.y_min] {
            for b in v.to_le_bytes() {
                h = fnv_byte(h, b);
            }
        }
        let mut arr = [BlockStateId::AIR; SECTION_VOLUME];
        for section in &self.sections {
            section.unpack_into(&mut arr);
            for id in &arr {
                for b in id.0.to_le_bytes() {
                    h = fnv_byte(h, b);
                }
            }
        }
        h
    }
}

/// Block composition of a set of sections, under [`is_transparent`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Composition {
    /// Air blocks.
    pub air: usize,
    /// Non-air opaque blocks.
    pub opaque: usize,
    /// Transparent blocks (water, leaves, glass).
    pub transparent: usize,
}

impl Composition {
    /// Total number of blocks counted.
    #[must_use]
    pub fn total(&self) -> usize {
        self.air + self.opaque + self.transparent
    }

    /// Transparent blocks as a fraction of all blocks.
    #[must_use]
    pub fn transparent_share(&self) -> f64 {
        self.transparent as f64 / self.total() as f64
    }

    /// Transparent blocks as a fraction of non-air blocks.
    #[must_use]
    pub fn transparent_share_of_non_air(&self) -> f64 {
        self.transparent as f64 / (self.opaque + self.transparent) as f64
    }
}

/// Counts the composition of the given sections under [`is_transparent`].
pub fn composition<'a>(sections: impl IntoIterator<Item = &'a Section>) -> Composition {
    let mut c = Composition::default();
    let mut arr = [BlockStateId::AIR; SECTION_VOLUME];
    for section in sections {
        section.unpack_into(&mut arr);
        for &b in &arr {
            if b == BlockStateId::AIR {
                c.air += 1;
            } else if is_transparent(b) {
                c.transparent += 1;
            } else {
                c.opaque += 1;
            }
        }
    }
    c
}

/// Generates the deterministic test corpus for `area` from `seed`.
///
/// Columns come in a fixed order (x major, z minor), but each column is
/// independent: generating them in any order or on any number of threads gives
/// byte-identical sections and the same [`TestColumn::hash`].
#[must_use]
pub fn test_corpus(seed: u64, area: &CorpusArea) -> Vec<TestColumn> {
    let mut columns = Vec::with_capacity(area.column_count());
    for x in area.x.clone() {
        for z in area.z.clone() {
            columns.push(test_column(seed, x, z, area.y.clone()));
        }
    }
    columns
}

/// Generates one column of the test corpus: a pure function of
/// `(seed, x, z, y)` in section coordinates.
#[must_use]
pub fn test_column(seed: u64, x: i32, z: i32, y: Range<i32>) -> TestColumn {
    let height_noise = Perlin::new(splitmix64(seed ^ HEIGHT_DOMAIN) as u32);
    let cave_noise = Perlin::new(splitmix64(seed ^ CAVE_DOMAIN) as u32);

    let height_blocks = (y.end - y.start).max(0) as usize * 16;
    // Flat column buffer, index (by * 16 + lz) * 16 + lx, by relative to y.start.
    let mut blocks = vec![BlockStateId::AIR; height_blocks * 256];
    let base_y = y.start * 16;

    for lx in 0..16i32 {
        for lz in 0..16i32 {
            let (wx, wz) = (x * 16 + lx, z * 16 + lz);
            let h = surface_height(&height_noise, wx, wz);
            for (by, slot) in blocks
                .iter_mut()
                .skip((lz * 16 + lx) as usize)
                .step_by(256)
                .enumerate()
            {
                let wy = base_y + by as i32;
                *slot = BlockStateId(terrain_block(&cave_noise, seed, wx, wy, wz, h));
            }
        }
    }

    place_trees(seed, &height_noise, x, z, base_y, &mut blocks);

    let mut sections = Vec::with_capacity(height_blocks / 16);
    for sy in 0..height_blocks / 16 {
        let mut arr = [BlockStateId::AIR; SECTION_VOLUME];
        for (i, slot) in arr.iter_mut().enumerate() {
            *slot = blocks[sy * SECTION_VOLUME + i];
        }
        sections.push(Section::from_blocks(&arr));
    }
    TestColumn {
        x,
        z,
        y_min: y.start,
        sections,
    }
}

/// Terrain surface height at a block position: 4 octaves of Perlin, base
/// frequency 1/128, gain 0.5, lacunarity 2 (the research corpus parameters).
fn surface_height(noise: &Perlin, wx: i32, wz: i32) -> i32 {
    let (wx, wz) = (f64::from(wx), f64::from(wz));
    let (mut a, mut f, mut s) = (1.0, 1.0 / 128.0, 0.0);
    for _ in 0..4 {
        s += a * noise.get([wx * f, wz * f]);
        a *= 0.5;
        f *= 2.0;
    }
    (HEIGHT_BASE + s * HEIGHT_AMP) as i32
}

/// The terrain block at one position, given the precomputed surface height `h`.
fn terrain_block(cave: &Perlin, seed: u64, wx: i32, wy: i32, wz: i32, h: i32) -> u16 {
    if wy > h {
        return if wy <= SEA_LEVEL {
            ids::WATER
        } else {
            ids::AIR
        };
    }
    let mut b = if wy == h {
        if h <= SEA_LEVEL + 1 {
            ids::SAND
        } else {
            ids::GRASS
        }
    } else if wy >= h - 3 {
        ids::DIRT
    } else {
        ids::STONE
    };
    // Caves: 3D noise worms below the topsoil.
    if wy < h - 2
        && cave.get([
            f64::from(wx) / 24.0,
            f64::from(wy) / 16.0,
            f64::from(wz) / 24.0,
        ]) > 0.55
    {
        return ids::AIR;
    }
    // Ores: deterministic 2x2x2 clusters scattered in stone (~2% of stone).
    if b == ids::STONE {
        let roll = splitmix64(
            seed ^ ORE_DOMAIN
                ^ (i64::from(wx >> 1) as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
                ^ (i64::from(wy >> 1) as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
                ^ (i64::from(wz >> 1) as u64).wrapping_mul(0x1656_67B1_9E37_79F9),
        ) % 100;
        if roll == 0 {
            b = ids::COAL_ORE;
        } else if roll == 1 && wy < IRON_MAX_Y {
            b = ids::IRON_ORE;
        }
    }
    b
}

/// Trees for the column's 32x32 placement cell.
///
/// A 16-wide column lies entirely inside one 32x32 cell, and trunks (cell offset
/// 3..=28) plus leaf blobs (reach 2) never leave the cell, so considering only
/// the column's own cell is exact. Both columns of a cell replay the same
/// candidate sequence and keep only the writes inside themselves; the result
/// equals generating the whole cell at once, independent of order.
fn place_trees(
    seed: u64,
    height_noise: &Perlin,
    x: i32,
    z: i32,
    base_y: i32,
    blocks: &mut [BlockStateId],
) {
    let (cx, cz) = (x.div_euclid(2), z.div_euclid(2));
    let mut r = Xorshift(
        splitmix64(
            seed ^ TREE_DOMAIN
                ^ (i64::from(cx) as u64).wrapping_mul(73_856_093)
                ^ (i64::from(cz) as u64).wrapping_mul(19_349_663),
        ) | 1,
    );
    let height_blocks = (blocks.len() / 256) as i32;
    let mut put = |wx: i32, wy: i32, wz: i32, id: u16| {
        let (lx, lz, by) = (wx - x * 16, wz - z * 16, wy - base_y);
        if (0..16).contains(&lx) && (0..16).contains(&lz) && (0..height_blocks).contains(&by) {
            let i = (by * 16 + lz) * 16 + lx;
            if blocks[i as usize] == BlockStateId::AIR {
                blocks[i as usize] = BlockStateId(id);
            }
        }
    };
    for _ in 0..TREES_PER_CELL {
        let tx = cx * TREE_CELL + 3 + (r.next() % 26) as i32;
        let tz = cz * TREE_CELL + 3 + (r.next() % 26) as i32;
        let h = surface_height(height_noise, tx, tz);
        if h <= SEA_LEVEL + 1 {
            continue;
        }
        for dy in 1..=5 {
            put(tx, h + dy, tz, ids::DIRT);
        }
        for dy in 4..=7 {
            for dx in -2i32..=2 {
                for dz in -2i32..=2 {
                    if dx.abs() + dz.abs() <= 3 {
                        put(tx + dx, h + dy, tz + dz, ids::LEAVES);
                    }
                }
            }
        }
    }
}

/// xorshift64, the deterministic RNG of the research corpus.
struct Xorshift(u64);

impl Xorshift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// splitmix64 finalizer: seed and coordinate mixing.
fn splitmix64(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn fnv_byte(h: u64, b: u8) -> u64 {
    (h ^ u64::from(b)).wrapping_mul(FNV_PRIME)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area() -> CorpusArea {
        CorpusArea {
            x: 0..2,
            y: -1..5,
            z: 0..2,
        }
    }

    #[test]
    fn corpus_has_terrain_water_trees_and_ores() {
        let columns = test_corpus(0, &area());
        assert_eq!(columns.len(), 4);
        let comp = composition(columns.iter().flat_map(|c| c.sections()));
        assert!(comp.air > 0);
        assert!(comp.opaque > 0);
        // ~8% transparent blocks (of non-air), the research corpus figure.
        let share = comp.transparent_share_of_non_air();
        assert!((0.04..0.15).contains(&share), "transparent share {share}");

        let mut seen = [false; 10];
        let mut arr = [BlockStateId::AIR; SECTION_VOLUME];
        for column in &columns {
            for section in column.sections() {
                section.unpack_into(&mut arr);
                for &b in &arr {
                    seen[b.0 as usize] = true;
                }
            }
        }
        for id in [
            ids::STONE,
            ids::DIRT,
            ids::GRASS,
            ids::SAND,
            ids::WATER,
            ids::LEAVES,
            ids::COAL_ORE,
            ids::IRON_ORE,
        ] {
            assert!(seen[id as usize], "missing block id {id}");
        }
        assert!(!seen[ids::GLASS as usize], "glass is edit-only");
    }

    #[test]
    fn different_seeds_give_different_terrain() {
        let a = test_column(0, 0, 0, -1..5);
        let b = test_column(1, 0, 0, -1..5);
        assert_ne!(a.hash(), b.hash());
    }

    #[test]
    fn hash_is_representation_independent() {
        let mut column = test_column(7, 1, -2, -1..5);
        let before = column.hash();
        for section in &mut column.sections {
            section.compact();
        }
        assert_eq!(column.hash(), before);
    }

    #[test]
    fn column_matches_the_same_column_of_a_larger_area() {
        // Padding independence: a column generated alone must equal the same
        // column cut from a larger corpus (trees, caves and ores included).
        let single = test_column(42, 1, 1, -1..5);
        let bigger = test_corpus(
            42,
            &CorpusArea {
                x: -1..3,
                y: -1..5,
                z: -1..3,
            },
        );
        let cut = &bigger[2 * 4 + 2];
        assert_eq!(single.hash(), cut.hash());
    }

    #[test]
    fn empty_area_and_empty_y_range() {
        let columns = test_corpus(
            0,
            &CorpusArea {
                x: 2..2,
                y: 0..4,
                z: 0..3,
            },
        );
        assert!(columns.is_empty());
        let column = test_column(0, 0, 0, 3..3);
        assert!(column.sections().is_empty());
        assert_eq!(column.y_range(), 3..3);
    }

    #[test]
    fn section_accessor_covers_the_y_range() {
        let column = test_column(3, -1, 2, -2..3);
        assert_eq!(column.y_range(), -2..3);
        assert_eq!(column.sections().len(), 5);
        for sy in -2..3 {
            let _ = column.section(sy);
        }
    }
}
