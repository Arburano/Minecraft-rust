//! Palette-compressed 16^3 block section.
//!
//! Spec: `docs/architecture/world-storage.md` ("Section"), ADR-0004, task T0-03.
//! Starting point: `research/bench/palette`.
//!
//! Three representations, chosen automatically by the number of distinct states:
//!
//! - [`Section`] is uniform (`Single`) while every block is the same state.
//! - `Paletted` stores a palette of 2..=256 states plus packed indices. Indices
//!   never straddle a `u64` word (the Minecraft 1.16+ layout): a word holds
//!   `64 / bits` indices and the leftover high bits stay zero. This makes `get`
//!   and word-by-word [`Section::unpack_into`] cheap.
//! - `Direct` is a flat `u16` per block once a section exceeds 256 states.
//!
//! The bit width grows when the palette overflows; shrinking back to a minimal
//! representation happens only in [`Section::compact`] (PostTick or save), never
//! on the edit hot path. A reverse hash index is kept for palettes larger than 16
//! entries so `set` stays O(1) instead of scanning the palette linearly.
//!
//! The engine does not know block semantics: opacity is registry/mod data
//! (`datapacks`, `opaque`). The section therefore tracks only the intrinsic
//! `non_air` counter; [`Section::all_opaque`] takes the caller's opacity predicate.

use std::collections::HashMap;

use crate::{BlockStateId, SECTION_VOLUME};

/// Number of blocks in a section, local alias for readability.
const VOLUME: usize = SECTION_VOLUME;

/// The block storage of a section. Private; [`Section`] wraps it with counters.
///
/// The layout is tuned so `size_of::<Section>()` stays at 56 bytes (the measured
/// `Single` size in world-storage.md): the palette and packed words are boxed
/// slices (16 bytes each) and the reverse index is an optional boxed map (8 bytes).
#[derive(Clone)]
enum SectionBlocks {
    /// Every block is the same state.
    Single(BlockStateId),
    /// 2..=256 distinct states, packed indices that do not cross word boundaries.
    Paletted {
        /// Distinct states, in first-seen order. Index into this is the packed value.
        palette: Box<[BlockStateId]>,
        /// Bits per index: 1..=8.
        bits: u8,
        /// Packed indices, `64 / bits` per word, high bits unused.
        data: Box<[u64]>,
        /// State -> palette index, present only when `palette.len() > 16`.
        reverse: Option<Box<ReverseIndex>>,
    },
    /// More than 256 distinct states: a flat `u16` per block.
    Direct(Box<[BlockStateId; VOLUME]>),
}

/// A compact open-addressing hash index from block state to palette index.
///
/// Used only for palettes larger than 16 entries, where a linear scan would make
/// `set` O(palette). Each slot is one `u32`: the key in the low 16 bits, the
/// palette index in the high byte, and an occupancy bit (bit 16). An empty slot
/// is `0`. The table is a power of two at most half full, hashed by multiply-shift,
/// so a lookup is one multiply and a single load per probe.
///
/// Keys and values are interleaved in one array on purpose: with separate key and
/// value arrays, random `set` on an 8-bit section (whose packed data is exactly
/// 4 KiB) hit severe 4K-aliasing stalls between the data stores and the index
/// loads (~260 ns/op on the test VM). One interleaved array keeps the index small
/// and contiguous and removes the stall (≤ 10 ns/op).
#[derive(Clone)]
struct ReverseIndex {
    /// One entry per slot: `(index << 24) | OCCUPIED | key`, or `0` when empty.
    entries: Box<[u32]>,
    /// `capacity - 1`; capacity is a power of two.
    mask: usize,
}

impl ReverseIndex {
    /// Occupancy flag (bit 16). Lets every index `0..=255` and any key be stored,
    /// including index 255 and key 0, without colliding with the empty marker.
    const OCCUPIED: u32 = 1 << 16;

    /// An empty table sized for at least `len` entries at load factor ≤ 0.5.
    fn with_capacity(len: usize) -> Self {
        let mut cap = 16usize;
        while cap < len * 2 {
            cap <<= 1;
        }
        Self {
            entries: vec![0u32; cap].into_boxed_slice(),
            mask: cap - 1,
        }
    }

    /// Builds an index covering every palette entry.
    fn from_palette(palette: &[BlockStateId]) -> Self {
        let mut index = Self::with_capacity(palette.len());
        for (i, &v) in palette.iter().enumerate() {
            index.insert(v, i as u8);
        }
        index
    }

    /// Multiply-shift hash: the high bits of `key * golden_ratio` are well spread.
    #[inline]
    fn slot(&self, key: BlockStateId) -> usize {
        ((key.0 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 32) as usize & self.mask
    }

    /// True when `len` entries would exceed the half-full load factor.
    #[inline]
    fn needs_grow(&self, len: usize) -> bool {
        len * 2 > self.mask + 1
    }

    /// Looks up the palette index of `key`.
    #[inline]
    fn get(&self, key: BlockStateId) -> Option<u8> {
        let mut i = self.slot(key);
        loop {
            let entry = self.entries[i];
            if entry & Self::OCCUPIED == 0 {
                return None;
            }
            if entry as u16 == key.0 {
                return Some((entry >> 24) as u8);
            }
            i = (i + 1) & self.mask;
        }
    }

    /// Inserts or updates `key`. The table must have room (load factor ≤ 0.5).
    fn insert(&mut self, key: BlockStateId, value: u8) {
        let packed = (u32::from(value) << 24) | Self::OCCUPIED | u32::from(key.0);
        let mut i = self.slot(key);
        loop {
            let entry = self.entries[i];
            if entry & Self::OCCUPIED == 0 {
                self.entries[i] = packed;
                return;
            }
            if entry as u16 == key.0 {
                self.entries[i] = packed;
                return;
            }
            i = (i + 1) & self.mask;
        }
    }
}

/// A 16^3 section of block states with palette compression (ADR-0004).
///
/// Create with [`Section::new`] (all air), [`Section::uniform`] or
/// [`Section::from_blocks`]. Read with [`Section::get`], edit with
/// [`Section::set`], bulk-read for meshing with [`Section::unpack_into`].
#[derive(Clone)]
pub struct Section {
    blocks: SectionBlocks,
    /// Count of blocks that are not air. Maintained on every edit.
    non_air: u16,
}

impl Default for Section {
    fn default() -> Self {
        Self::new()
    }
}

/// Bits needed to index `len` palette entries (1..=8 for 2..=256).
fn bits_for(len: usize) -> u8 {
    debug_assert!((2..=256).contains(&len));
    (usize::BITS - (len - 1).leading_zeros()) as u8
}

/// Indices that fit in one `u64` word at this bit width.
fn entries_per_word(bits: u8) -> usize {
    64 / bits as usize
}

/// Words needed to hold a whole section at this bit width.
fn words_for(bits: u8) -> usize {
    let per = entries_per_word(bits);
    VOLUME.div_ceil(per)
}

/// Low `bits` bits set.
fn index_mask(bits: u8) -> u64 {
    (1u64 << bits) - 1
}

/// `64 / bits`, indexed by bit width 0..=8 (0 is unused).
const PER_WORD: [u32; 9] = [1, 64, 32, 21, 16, 12, 10, 9, 8];

/// Multiplier for dividing an index `< 4096` by `PER_WORD[bits]` without a
/// hardware divide: `i / per == (i * MAGIC[bits]) >> MAGIC_SHIFT`
/// (`MAGIC = ceil(2^20 / per)`; the product stays below 2^32; exhaustively tested). Integer division by a
/// run-time value is slow on some CPUs (Zen+: the T0-16 `get`/`set` misses).
const MAGIC: [u32; 9] = {
    let mut m = [0u32; 9];
    let mut b = 1;
    while b < 9 {
        m[b] = (1u32 << MAGIC_SHIFT).div_ceil(PER_WORD[b]);
        b += 1;
    }
    m
};
const MAGIC_SHIFT: u32 = 20;

/// Word index and bit shift of packed index `i` at this width.
#[inline]
fn locate(bits: u8, i: usize) -> (usize, usize) {
    let b = bits as usize;
    let word = ((i as u32 * MAGIC[b]) >> MAGIC_SHIFT) as usize;
    let slot = i - word * PER_WORD[b] as usize;
    (word, slot * b)
}

/// Reads packed index `i` from `data`.
#[inline]
fn get_raw(data: &[u64], bits: u8, i: usize) -> u64 {
    let (word, shift) = locate(bits, i);
    (data[word] >> shift) & index_mask(bits)
}

/// Writes packed index `i` into `data`, leaving neighbouring indices intact.
#[inline]
fn set_raw(data: &mut [u64], bits: u8, i: usize, value: u64) {
    let (word, shift) = locate(bits, i);
    let mask = index_mask(bits) << shift;
    let word = &mut data[word];
    *word = (*word & !mask) | (value << shift);
}

/// Looks up the palette index of `value`: hash for large palettes, linear otherwise.
#[inline]
fn palette_index(
    palette: &[BlockStateId],
    reverse: Option<&ReverseIndex>,
    value: BlockStateId,
) -> Option<u8> {
    match reverse {
        Some(index) => index.get(value),
        None => palette.iter().position(|&p| p == value).map(|i| i as u8),
    }
}

/// Builds the reverse index for a palette larger than 16 entries.
fn build_reverse(palette: &[BlockStateId]) -> Option<Box<ReverseIndex>> {
    if palette.len() > 16 {
        Some(Box::new(ReverseIndex::from_palette(palette)))
    } else {
        None
    }
}

/// Unpacks paletted words into `out`, word by word (the fast gather path).
fn unpack_paletted(
    palette: &[BlockStateId],
    bits: u8,
    data: &[u64],
    out: &mut [BlockStateId; VOLUME],
) {
    let per = entries_per_word(bits);
    let mask = index_mask(bits);
    let mut i = 0;
    for &word in data {
        let mut rest = word;
        for _ in 0..per {
            if i >= VOLUME {
                return;
            }
            out[i] = palette[(rest & mask) as usize];
            rest >>= bits;
            i += 1;
        }
    }
}

/// Builds the smallest representation for a full block array.
fn build_from_array(arr: &[BlockStateId; VOLUME]) -> SectionBlocks {
    // One pass assigns palette indices: runs of equal blocks hit the last-value
    // cache, small palettes use a linear scan, large ones a hash map (T1-03:
    // a SipHash lookup per block made packing 5x slower than generation).
    let mut indices = [0u8; VOLUME];
    let mut palette: Vec<BlockStateId> = Vec::with_capacity(16);
    let mut map: Option<HashMap<BlockStateId, u8>> = None;
    palette.push(arr[0]);
    let (mut last, mut last_index) = (arr[0], 0u8);
    for (slot, &v) in indices.iter_mut().zip(arr.iter()) {
        if v != last {
            let found = match &map {
                Some(m) => m.get(&v).copied(),
                None => palette.iter().position(|&p| p == v).map(|i| i as u8),
            };
            last_index = match found {
                Some(i) => i,
                None => {
                    if palette.len() == 256 {
                        return SectionBlocks::Direct(Box::new(*arr));
                    }
                    let i = palette.len() as u8;
                    palette.push(v);
                    match &mut map {
                        Some(m) => {
                            m.insert(v, i);
                        }
                        None if palette.len() > 16 => {
                            map = Some(
                                palette
                                    .iter()
                                    .enumerate()
                                    .map(|(j, &p)| (p, j as u8))
                                    .collect(),
                            );
                        }
                        None => {}
                    }
                    i
                }
            };
            last = v;
        }
        *slot = last_index;
    }
    if palette.len() == 1 {
        return SectionBlocks::Single(palette[0]);
    }
    let bits = bits_for(palette.len());
    let per = entries_per_word(bits);
    let mut data = vec![0u64; words_for(bits)].into_boxed_slice();
    for (word, chunk) in data.iter_mut().zip(indices.chunks(per)) {
        *word = chunk
            .iter()
            .enumerate()
            .fold(0u64, |w, (j, &k)| w | (u64::from(k) << (j * bits as usize)));
    }
    let reverse = build_reverse(&palette);
    SectionBlocks::Paletted {
        palette: palette.into_boxed_slice(),
        bits,
        data,
        reverse,
    }
}

impl Section {
    /// An all-air section.
    #[must_use]
    pub fn new() -> Self {
        Self::uniform(BlockStateId::AIR)
    }

    /// A section filled with one state.
    #[must_use]
    pub fn uniform(value: BlockStateId) -> Self {
        Self {
            blocks: SectionBlocks::Single(value),
            non_air: if value == BlockStateId::AIR {
                0
            } else {
                VOLUME as u16
            },
        }
    }

    /// Builds the smallest representation for a full section of blocks.
    #[must_use]
    pub fn from_blocks(arr: &[BlockStateId; VOLUME]) -> Self {
        let non_air = arr.iter().filter(|&&v| v != BlockStateId::AIR).count() as u16;
        Self {
            blocks: build_from_array(arr),
            non_air,
        }
    }

    /// The block state at `index` (YZX order, `0..4096`).
    #[inline]
    #[must_use]
    pub fn get(&self, index: usize) -> BlockStateId {
        debug_assert!(index < VOLUME);
        match &self.blocks {
            SectionBlocks::Single(v) => *v,
            SectionBlocks::Paletted {
                palette,
                bits,
                data,
                ..
            } => palette[get_raw(data, *bits, index) as usize],
            SectionBlocks::Direct(arr) => arr[index],
        }
    }

    /// Sets the block at `index` (YZX order, `0..4096`), growing the palette or
    /// representation as needed and updating the `non_air` counter.
    ///
    /// Writing a state already in the palette takes no allocation. Adding a new
    /// state may grow the palette or repack to a wider representation; that is
    /// the cold path, not the per-edit hot loop.
    pub fn set(&mut self, index: usize, value: BlockStateId) {
        debug_assert!(index < VOLUME);
        let replacement = match &mut self.blocks {
            SectionBlocks::Single(s) => {
                let old = *s;
                if old == value {
                    return;
                }
                self.non_air += u16::from(value != BlockStateId::AIR);
                self.non_air -= u16::from(old != BlockStateId::AIR);
                let palette = vec![old, value].into_boxed_slice();
                let mut data = vec![0u64; words_for(1)].into_boxed_slice();
                set_raw(&mut data, 1, index, 1);
                Some(SectionBlocks::Paletted {
                    palette,
                    bits: 1,
                    data,
                    reverse: None,
                })
            }
            SectionBlocks::Paletted {
                palette,
                bits,
                data,
                reverse,
            } => {
                let width = *bits;
                let old = palette[get_raw(data, width, index) as usize];
                if old == value {
                    return;
                }
                self.non_air += u16::from(value != BlockStateId::AIR);
                self.non_air -= u16::from(old != BlockStateId::AIR);
                if let Some(idx) = palette_index(palette, reverse.as_deref(), value) {
                    set_raw(data, width, index, idx as u64);
                    None
                } else if palette.len() < (1usize << width) {
                    let mut grown = palette.to_vec();
                    grown.push(value);
                    let idx = (grown.len() - 1) as u8;
                    *palette = grown.into_boxed_slice();
                    if palette.len() > 16 {
                        match reverse {
                            Some(index) if !index.needs_grow(palette.len()) => {
                                index.insert(value, idx);
                            }
                            _ => *reverse = build_reverse(palette),
                        }
                    }
                    set_raw(data, width, index, idx as u64);
                    None
                } else {
                    // Palette full at this width: repack wider (or go direct).
                    let mut arr = [BlockStateId::AIR; VOLUME];
                    unpack_paletted(palette, width, data, &mut arr);
                    arr[index] = value;
                    Some(build_from_array(&arr))
                }
            }
            SectionBlocks::Direct(arr) => {
                let old = arr[index];
                if old == value {
                    return;
                }
                self.non_air += u16::from(value != BlockStateId::AIR);
                self.non_air -= u16::from(old != BlockStateId::AIR);
                arr[index] = value;
                None
            }
        };
        if let Some(blocks) = replacement {
            self.blocks = blocks;
        }
    }

    /// Appends the on-disk encoding (world-storage.md, format v1):
    /// tag `0` + u16 state (single), tag `1` + u8 bits + u16 palette length +
    /// palette u16s + packed u64 words (paletted), or tag `2` + 4096 u16s
    /// (direct). Little endian.
    pub fn encode(&self, out: &mut Vec<u8>) {
        match &self.blocks {
            SectionBlocks::Single(v) => {
                out.push(0);
                out.extend_from_slice(&v.0.to_le_bytes());
            }
            SectionBlocks::Paletted {
                palette,
                bits,
                data,
                ..
            } => {
                out.push(1);
                out.push(*bits);
                out.extend_from_slice(&(palette.len() as u16).to_le_bytes());
                for p in palette.iter() {
                    out.extend_from_slice(&p.0.to_le_bytes());
                }
                for w in data.iter() {
                    out.extend_from_slice(&w.to_le_bytes());
                }
            }
            SectionBlocks::Direct(arr) => {
                out.push(2);
                for v in arr.iter() {
                    out.extend_from_slice(&v.0.to_le_bytes());
                }
            }
        }
    }

    /// Decodes one section from the front of `bytes`, returning it and the
    /// number of bytes used. Every packed index is validated against the
    /// palette, so corrupt data yields an error, never a panic later.
    pub fn decode(bytes: &[u8]) -> Result<(Self, usize), SectionDecodeError> {
        let need = |n: usize| {
            if bytes.len() < n {
                Err(SectionDecodeError::Truncated)
            } else {
                Ok(())
            }
        };
        let u16_at = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
        need(1)?;
        match bytes[0] {
            0 => {
                need(3)?;
                Ok((Self::uniform(BlockStateId(u16_at(1))), 3))
            }
            1 => {
                need(4)?;
                let bits = bytes[1];
                let len = usize::from(u16_at(2));
                if !(1..=8).contains(&bits) || len < 2 || len > (1usize << bits) {
                    return Err(SectionDecodeError::Invalid);
                }
                let words = words_for(bits);
                let end = 4 + 2 * len + 8 * words;
                need(end)?;
                let palette: Box<[BlockStateId]> =
                    (0..len).map(|i| BlockStateId(u16_at(4 + 2 * i))).collect();
                let base = 4 + 2 * len;
                let data: Box<[u64]> = (0..words)
                    .map(|w| {
                        let mut b = [0u8; 8];
                        b.copy_from_slice(&bytes[base + 8 * w..base + 8 * w + 8]);
                        u64::from_le_bytes(b)
                    })
                    .collect();
                let mut non_air = 0u16;
                for i in 0..VOLUME {
                    let idx = get_raw(&data, bits, i) as usize;
                    let Some(&v) = palette.get(idx) else {
                        return Err(SectionDecodeError::Invalid);
                    };
                    non_air += u16::from(v != BlockStateId::AIR);
                }
                let reverse = build_reverse(&palette);
                Ok((
                    Self {
                        blocks: SectionBlocks::Paletted {
                            palette,
                            bits,
                            data,
                            reverse,
                        },
                        non_air,
                    },
                    end,
                ))
            }
            2 => {
                let end = 1 + 2 * VOLUME;
                need(end)?;
                let mut arr = Box::new([BlockStateId::AIR; VOLUME]);
                for (i, v) in arr.iter_mut().enumerate() {
                    *v = BlockStateId(u16_at(1 + 2 * i));
                }
                let non_air = arr.iter().filter(|&&v| v != BlockStateId::AIR).count() as u16;
                Ok((
                    Self {
                        blocks: SectionBlocks::Direct(arr),
                        non_air,
                    },
                    end,
                ))
            }
            _ => Err(SectionDecodeError::Invalid),
        }
    }

    /// Re-encodes states through `table` (`new = table[old]`, IDs outside the
    /// table become `fallback`), e.g. a saved world's IDs to the current
    /// registry (`Registry::remap`).
    pub fn remap(&mut self, table: &[BlockStateId], fallback: BlockStateId) {
        let map = |v: BlockStateId| table.get(usize::from(v.0)).copied().unwrap_or(fallback);
        let mut arr = [BlockStateId::AIR; VOLUME];
        self.unpack_into(&mut arr);
        for v in &mut arr {
            *v = map(*v);
        }
        *self = Self::from_blocks(&arr);
    }

    /// Unpacks the whole section into `out`, word by word. The basis of fast
    /// gather for meshing (see world-storage.md).
    pub fn unpack_into(&self, out: &mut [BlockStateId; VOLUME]) {
        match &self.blocks {
            SectionBlocks::Single(v) => out.fill(*v),
            SectionBlocks::Paletted {
                palette,
                bits,
                data,
                ..
            } => unpack_paletted(palette, *bits, data, out),
            SectionBlocks::Direct(arr) => out.copy_from_slice(arr.as_ref()),
        }
    }

    /// Repacks to the smallest representation for the current content, dropping
    /// unused palette entries and shrinking the bit width. Call in PostTick or on
    /// save, never on the edit hot path.
    pub fn compact(&mut self) {
        let mut arr = [BlockStateId::AIR; VOLUME];
        self.unpack_into(&mut arr);
        self.blocks = build_from_array(&arr);
    }

    /// Number of blocks that are not air.
    #[inline]
    #[must_use]
    pub fn non_air(&self) -> u16 {
        self.non_air
    }

    /// True when every block is air (the mesher can skip an empty mesh chunk).
    #[inline]
    #[must_use]
    pub fn all_air(&self) -> bool {
        self.non_air == 0
    }

    /// True when every block is opaque under the caller's `is_opaque` predicate.
    ///
    /// Opacity is registry/mod data, so the engine cannot decide it alone; the
    /// mesher passes the registry predicate. Returns false as soon as any air is
    /// present. For paletted sections the check runs over the palette, so it is
    /// conservative until [`Section::compact`] drops unused entries.
    pub fn all_opaque(&self, is_opaque: impl Fn(BlockStateId) -> bool) -> bool {
        if self.non_air != VOLUME as u16 {
            return false;
        }
        match &self.blocks {
            SectionBlocks::Single(v) => is_opaque(*v),
            SectionBlocks::Paletted { palette, .. } => palette.iter().all(|&v| is_opaque(v)),
            SectionBlocks::Direct(arr) => arr.iter().all(|&v| is_opaque(v)),
        }
    }

    /// Bits per packed index: 0 for `Single`, 1..=8 for `Paletted`, 16 for the
    /// flat `Direct` representation.
    #[must_use]
    pub fn bits(&self) -> u8 {
        match &self.blocks {
            SectionBlocks::Single(_) => 0,
            SectionBlocks::Paletted { bits, .. } => *bits,
            SectionBlocks::Direct(_) => 16,
        }
    }

    /// Number of distinct states in the palette (1 for `Single`, the palette
    /// length for `Paletted`, more than 256 for `Direct`).
    #[must_use]
    pub fn palette_len(&self) -> usize {
        match &self.blocks {
            SectionBlocks::Single(_) => 1,
            SectionBlocks::Paletted { palette, .. } => palette.len(),
            SectionBlocks::Direct(_) => VOLUME,
        }
    }

    /// Base storage size in bytes: the section struct plus the palette and packed
    /// words (or the flat array for `Direct`).
    ///
    /// Matches the measurement model in world-storage.md / optimization-tests.md:
    /// it excludes allocator overhead and the optional reverse hash index, which
    /// is a speed add-on accounted separately.
    #[must_use]
    pub fn storage_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + match &self.blocks {
                SectionBlocks::Single(_) => 0,
                SectionBlocks::Paletted { palette, data, .. } => palette.len() * 2 + data.len() * 8,
                SectionBlocks::Direct(_) => VOLUME * 2,
            }
    }
}

/// Errors decoding a stored section.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SectionDecodeError {
    /// The input ends early.
    Truncated,
    /// Unknown tag, bad bit width/palette size, or an index outside the palette.
    Invalid,
}

impl std::fmt::Display for SectionDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated => write!(f, "section data is truncated"),
            Self::Invalid => write!(f, "section data is invalid"),
        }
    }
}

impl std::error::Error for SectionDecodeError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_round_trip_and_remap() {
        let mut arr = [BlockStateId::AIR; VOLUME];
        for distinct in [1usize, 2, 3, 17, 200, 300] {
            for (i, v) in arr.iter_mut().enumerate() {
                *v = BlockStateId(((i * 7) % distinct) as u16);
            }
            let section = Section::from_blocks(&arr);
            let mut bytes = vec![9];
            section.encode(&mut bytes);
            let (back, used) = Section::decode(&bytes[1..]).expect("decodes");
            assert_eq!(used, bytes.len() - 1);
            for i in 0..VOLUME {
                assert_eq!(back.get(i), section.get(i));
            }
            assert_eq!(back.non_air(), section.non_air());
            for cut in [0, 1, used / 2, used - 1] {
                assert!(Section::decode(&bytes[1..1 + cut]).is_err(), "cut {cut}");
            }
        }
        // A packed index past the palette is rejected.
        let s = Section::from_blocks(&{
            let mut a = [BlockStateId(1); VOLUME];
            a[0] = BlockStateId(2);
            a[1] = BlockStateId(3);
            a
        });
        let mut bytes = Vec::new();
        s.encode(&mut bytes);
        bytes[2..4].copy_from_slice(&2u16.to_le_bytes()); // palette len 3 -> 2
        bytes.drain(8..10);
        assert!(Section::decode(&bytes).is_err());
        // Remap: 1 -> 5, unknown 3 -> fallback 9, air stays air.
        let mut r = s.clone();
        r.remap(
            &[BlockStateId::AIR, BlockStateId(5), BlockStateId(6)],
            BlockStateId(9),
        );
        assert_eq!(
            (r.get(0), r.get(1), r.get(2)),
            (BlockStateId(6), BlockStateId(9), BlockStateId(5))
        );
    }

    #[test]
    fn magic_division_is_exact_for_every_width_and_index() {
        for bits in 1u8..=8 {
            let per = entries_per_word(bits);
            assert_eq!(PER_WORD[bits as usize] as usize, per);
            for i in 0..VOLUME {
                assert_eq!(
                    locate(bits, i),
                    (i / per, (i % per) * bits as usize),
                    "bits {bits} i {i}"
                );
            }
        }
    }

    fn ids(values: &[u16]) -> [BlockStateId; VOLUME] {
        let mut arr = [BlockStateId::AIR; VOLUME];
        for (i, &v) in values.iter().enumerate().take(VOLUME) {
            arr[i] = BlockStateId(v);
        }
        arr
    }

    #[test]
    fn section_struct_is_56_bytes() {
        assert_eq!(std::mem::size_of::<Section>(), 56);
    }

    #[test]
    fn uniform_section_round_trips() {
        let s = Section::uniform(BlockStateId(7));
        assert_eq!(s.bits(), 0);
        assert_eq!(s.non_air(), 4096);
        assert!(!s.all_air());
        for i in 0..VOLUME {
            assert_eq!(s.get(i), BlockStateId(7));
        }
        let mut out = [BlockStateId::AIR; VOLUME];
        s.unpack_into(&mut out);
        assert!(out.iter().all(|&v| v == BlockStateId(7)));
    }

    #[test]
    fn air_section_flags() {
        let s = Section::new();
        assert!(s.all_air());
        assert_eq!(s.non_air(), 0);
        assert!(!s.all_opaque(|_| true));
    }

    #[test]
    fn set_grows_through_all_widths_to_direct() {
        let mut s = Section::new();
        let mut reference = [BlockStateId::AIR; VOLUME];
        // Insert 300 distinct states at spread indices, forcing 1..8 bit then direct.
        for k in 0..300u16 {
            let i = (k as usize * 13) % VOLUME;
            let v = BlockStateId(k + 1);
            s.set(i, v);
            reference[i] = v;
            assert_eq!(s.get(i), v);
        }
        assert_eq!(s.bits(), 16);
        for (i, &expected) in reference.iter().enumerate() {
            assert_eq!(s.get(i), expected);
        }
    }

    #[test]
    fn reverse_index_keeps_large_palette_correct() {
        let mut arr = [BlockStateId::AIR; VOLUME];
        for (i, v) in arr.iter_mut().enumerate() {
            *v = BlockStateId((i % 200) as u16);
        }
        let mut s = Section::from_blocks(&arr);
        assert_eq!(s.bits(), 8);
        // Edit existing palette entries and add a few new ones past 16.
        for k in 0..100u16 {
            let i = (k as usize * 7) % VOLUME;
            let v = BlockStateId(200 + k);
            s.set(i, v);
            arr[i] = v;
        }
        for (i, &expected) in arr.iter().enumerate() {
            assert_eq!(s.get(i), expected);
        }
    }

    #[test]
    fn reverse_index_handles_index_255_and_key_zero() {
        // First-seen palette [255, 254, .., 0]: state 0 lands at index 255 and
        // state 255 at index 0. Both must survive the reverse-index empty marker.
        let mut arr = [BlockStateId::AIR; VOLUME];
        for (i, v) in arr.iter_mut().enumerate() {
            *v = BlockStateId((255 - (i % 256)) as u16);
        }
        let mut s = Section::from_blocks(&arr);
        assert_eq!(s.bits(), 8);
        assert_eq!(s.palette_len(), 256);
        for (k, slot) in arr.iter_mut().enumerate() {
            let v = BlockStateId((k % 256) as u16);
            s.set(k, v);
            *slot = v;
        }
        for (i, &expected) in arr.iter().enumerate() {
            assert_eq!(s.get(i), expected);
        }
        // Explicitly round-trip the two edge states through the index.
        s.set(0, BlockStateId(0));
        s.set(1, BlockStateId(255));
        assert_eq!(s.get(0), BlockStateId(0));
        assert_eq!(s.get(1), BlockStateId(255));
    }

    #[test]
    fn compact_shrinks_width_and_drops_unused() {
        let mut arr = [BlockStateId::AIR; VOLUME];
        for (i, v) in arr.iter_mut().enumerate() {
            *v = BlockStateId((i % 200) as u16);
        }
        let mut s = Section::from_blocks(&arr);
        assert_eq!(s.bits(), 8);
        // Collapse to two states, then compact.
        for v in arr.iter_mut() {
            *v = BlockStateId(1);
        }
        for i in 0..VOLUME {
            s.set(i, BlockStateId(1));
        }
        s.set(0, BlockStateId(2));
        arr[0] = BlockStateId(2);
        s.compact();
        assert_eq!(s.bits(), 1);
        assert_eq!(s.palette_len(), 2);
        for (i, &expected) in arr.iter().enumerate() {
            assert_eq!(s.get(i), expected);
        }
    }

    #[test]
    fn compact_uniform_becomes_single() {
        let mut s = Section::from_blocks(&ids(&[1, 2, 3]));
        for i in 0..VOLUME {
            s.set(i, BlockStateId(9));
        }
        s.compact();
        assert_eq!(s.bits(), 0);
        assert_eq!(s.get(123), BlockStateId(9));
    }

    #[test]
    fn non_air_counter_tracks_edits() {
        let mut s = Section::new();
        s.set(0, BlockStateId(1));
        s.set(1, BlockStateId(2));
        assert_eq!(s.non_air(), 2);
        s.set(0, BlockStateId::AIR);
        assert_eq!(s.non_air(), 1);
        s.set(1, BlockStateId(3));
        assert_eq!(s.non_air(), 1);
        s.set(1, BlockStateId::AIR);
        assert_eq!(s.non_air(), 0);
        assert!(s.all_air());
    }

    #[test]
    fn all_opaque_uses_predicate() {
        let opaque = |v: BlockStateId| v != BlockStateId::AIR && v != BlockStateId(5);
        let solid = Section::uniform(BlockStateId(1));
        assert!(solid.all_opaque(opaque));
        let glass = Section::uniform(BlockStateId(5));
        assert!(!glass.all_opaque(opaque));
        let mut mixed = Section::uniform(BlockStateId(1));
        mixed.set(0, BlockStateId::AIR);
        assert!(!mixed.all_opaque(opaque));
    }

    #[test]
    fn storage_sizes_match_spec_table() {
        // Single: 56 bytes.
        assert_eq!(Section::new().storage_bytes(), 56);

        // Surface: 8 distinct, 3 bit -> 1640.
        let mut surface = [BlockStateId::AIR; VOLUME];
        for (i, v) in surface.iter_mut().enumerate() {
            *v = BlockStateId((i % 8) as u16);
        }
        let s = Section::from_blocks(&surface);
        assert_eq!(s.bits(), 3);
        assert_eq!(s.storage_bytes(), 1640);

        // Dungeon: 10 distinct, 4 bit -> 2124.
        let mut dungeon = [BlockStateId::AIR; VOLUME];
        for (i, v) in dungeon.iter_mut().enumerate() {
            *v = BlockStateId((i % 10) as u16);
        }
        let s = Section::from_blocks(&dungeon);
        assert_eq!(s.bits(), 4);
        assert_eq!(s.storage_bytes(), 2124);

        // 256 distinct, 8 bit -> 4664.
        let mut wide = [BlockStateId::AIR; VOLUME];
        for (i, v) in wide.iter_mut().enumerate() {
            *v = BlockStateId((i % 256) as u16);
        }
        let s = Section::from_blocks(&wide);
        assert_eq!(s.bits(), 8);
        assert_eq!(s.storage_bytes(), 4664);

        // Direct: flat u16 -> 8192 data + 56 struct.
        let mut worst = [BlockStateId::AIR; VOLUME];
        for (i, v) in worst.iter_mut().enumerate() {
            *v = BlockStateId(i as u16);
        }
        let s = Section::from_blocks(&worst);
        assert_eq!(s.bits(), 16);
        assert_eq!(s.storage_bytes(), 8248);
    }

    #[test]
    fn unpack_matches_get_for_every_width() {
        for distinct in [1usize, 2, 3, 8, 16, 17, 100, 256, 300, 4096] {
            let mut arr = [BlockStateId::AIR; VOLUME];
            for (i, v) in arr.iter_mut().enumerate() {
                *v = BlockStateId((i % distinct) as u16);
            }
            let s = Section::from_blocks(&arr);
            let mut out = [BlockStateId::AIR; VOLUME];
            s.unpack_into(&mut out);
            assert_eq!(out, arr, "distinct={distinct}");
            for (i, &expected) in arr.iter().enumerate() {
                assert_eq!(s.get(i), expected);
            }
        }
    }
}
