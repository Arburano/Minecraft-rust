//! 64^3 render batches: quads of 8 mesh chunks (2x2x2) copied with an offset
//! of 0/32 per axis, 6 ranges by face direction (ADR-0005, T0-05).
//!
//! # Coordinate convention
//!
//! Batch coordinates are stored modulo 64 in the crate's 6-bit fields.
//! A quad anchor field that is a *max corner* (the plane of a positive
//! normal, or the far corner of faces whose vertices subtract an extent:
//! Up y, Down x, Right x/y, Front x/z) is always >= 1 in true coordinates,
//! so a stored 0 there unambiguously means 64. Min-corner fields are always
//! 0..=63 and never wrap. Adding the 0/32 offset is a flip of bit 5 of each
//! coordinate field (addition of 32 modulo 64), which never touches the
//! width/height/id bits. See [`crate::reference::batch_faces`] for decoding.
//!
//! # Update variants (open question No. 8, decided by the T0-05 benchmark)
//!
//! - Sub-ranges: [`RenderBatch`] keeps one quad list per slot and direction
//!   (48 sub-ranges with their own counters); an edit re-copies only the
//!   edited slot via [`RenderBatch::set_chunk`].
//! - Full re-copy: [`RenderBatch::pack_into`] concatenates all slots into 6
//!   contiguous per-direction ranges ([`PackedBatch`]), one GPU range and
//!   draw per direction. Re-copying the whole batch measured well under the
//!   50 us budget on the T0-04 corpus (see the task report), so the full
//!   re-copy is kept: 6 ranges instead of 48, no per-slot counters.

use crate::{MeshChunkQuads, Quad};
use world::RENDER_BATCH_MESH_CHUNKS;

/// Mesh chunks per axis in a render batch (2).
pub const B: usize = RENDER_BATCH_MESH_CHUNKS as usize;
/// Mesh chunks in a render batch (8).
pub const SLOTS: usize = B * B * B;

const _: () = assert!(crate::CS * B <= 64);

/// Slot index of a mesh chunk at batch-local chunk coordinates `0..B`,
/// y-major: `(sy * B + sz) * B + sx`.
///
/// # Panics
/// Panics if any coordinate is outside `0..B`.
#[must_use]
pub const fn slot(sx: usize, sy: usize, sz: usize) -> usize {
    assert!(sx < B && sy < B && sz < B);
    (sy * B + sz) * B + sx
}

/// Bit-5 flip mask per slot: XOR-ing a quad with it adds the slot's 0/32
/// offset to each 6-bit coordinate field modulo 64.
const OFFSET_XOR: [u64; SLOTS] = {
    let mut table = [0u64; SLOTS];
    let mut i = 0;
    while i < SLOTS {
        let (sy, sz, sx) = (i / (B * B), (i / B) % B, i % B);
        table[i] = ((sx * crate::CS) as u64)
            | (((sy * crate::CS) as u64) << 6)
            | (((sz * crate::CS) as u64) << 12);
        i += 1;
    }
    table
};

/// Quads of one 64^3 render batch, stored as 8 sub-ranges (one per mesh
/// chunk slot, see [`slot`]) of 6 per-direction lists. Quads are already in
/// batch coordinates (modulo 64, see the module docs). This is the
/// sub-range update variant: an edit re-copies only its slot.
#[derive(Clone, Debug)]
pub struct RenderBatch {
    chunks: Box<[MeshChunkQuads; SLOTS]>,
}

impl Default for RenderBatch {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderBatch {
    /// An empty batch (all 8 slots hold no quads).
    #[must_use]
    pub fn new() -> Self {
        Self {
            chunks: Box::new(std::array::from_fn(|_| MeshChunkQuads::default())),
        }
    }

    /// Copies the quads of one mesh chunk into `slot`, adding the 0/32
    /// offset per axis. Replaces the slot's previous content and reuses its
    /// capacity (no allocation after warm-up).
    ///
    /// # Panics
    /// Panics if `slot >= SLOTS`.
    pub fn set_chunk(&mut self, slot: usize, quads: &MeshChunkQuads) {
        assert!(slot < SLOTS);
        let xor = OFFSET_XOR[slot];
        for (dst, src) in self.chunks[slot].quads.iter_mut().zip(&quads.quads) {
            dst.clear();
            dst.extend(src.iter().map(|q| Quad(q.0 ^ xor)));
        }
    }

    /// The sub-range of one slot: 6 per-direction lists in batch coordinates.
    ///
    /// # Panics
    /// Panics if `slot >= SLOTS`.
    #[must_use]
    pub fn chunk(&self, slot: usize) -> &MeshChunkQuads {
        assert!(slot < SLOTS);
        &self.chunks[slot]
    }

    /// Total number of quads over all slots and directions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.chunks.iter().map(MeshChunkQuads::len).sum()
    }

    /// True when no slot holds geometry.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.chunks.iter().all(MeshChunkQuads::is_empty)
    }

    /// Full re-copy variant: concatenates all 8 slots into 6 contiguous
    /// per-direction ranges, slots in [`slot`] order. Reuses the capacity
    /// of `out` (no allocation after warm-up).
    pub fn pack_into(&self, out: &mut PackedBatch) {
        for dir in 0..6 {
            let dst = &mut out.quads[dir];
            dst.clear();
            for chunk in self.chunks.iter() {
                dst.extend_from_slice(&chunk.quads[dir]);
            }
        }
    }
}

/// A render batch packed into 6 contiguous ranges, one per [`crate::Face`]
/// direction: the upload/draw layout (one GPU buffer range and one draw
/// command per direction). Quads are in batch coordinates (modulo 64).
#[derive(Clone, Debug, Default)]
pub struct PackedBatch {
    /// One contiguous quad range per face direction.
    pub quads: [Vec<Quad>; 6],
}

impl PackedBatch {
    /// Total number of quads over all directions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.quads.iter().map(Vec::len).sum()
    }

    /// True when there is no visible geometry.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.quads.iter().all(Vec::is_empty)
    }
}

/// True when `dir`'s anchor stores the max corner (>= 1, so 0 means 64)
/// on `axis`. See the module docs for the per-face layout.
#[must_use]
pub(crate) const fn max_corner_field(dir: usize, axis: usize) -> bool {
    match dir {
        0 => axis == 1,              // Up: y plane
        1 => axis == 0,              // Down: x max
        2 => axis == 0 || axis == 1, // Right: x plane, y max
        3 => false,                  // Left: all min corners
        4 => axis == 0 || axis == 2, // Front: x max, z plane
        _ => false,                  // Back: all min corners
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MeshChunkBuilder;
    use crate::reference::{batch_faces, covered_faces};
    use world::BlockStateId;

    fn is_transparent(b: BlockStateId) -> bool {
        matches!(b.0, 5..=7)
    }

    #[test]
    fn offset_flips_bit5_only() {
        // Anchor 32 on each axis wraps to 0; width/height/id stay intact.
        let q = Quad::pack(32, 31, 0, 5, 7, 1234);
        let shifted = Quad(q.0 ^ OFFSET_XOR[slot(1, 1, 1)]);
        assert_eq!(shifted.xyz(), [0, 63, 32]);
        assert_eq!(shifted.width(), 5);
        assert_eq!(shifted.height(), 7);
        assert_eq!(shifted.voxel_id(), 1234);
    }

    #[test]
    fn set_chunk_replaces_and_keeps_other_slots() {
        let mut batch = RenderBatch::new();
        assert!(batch.is_empty());
        let mut a = MeshChunkQuads::default();
        a.quads[0].push(Quad::pack(1, 2, 3, 1, 1, 9));
        batch.set_chunk(slot(1, 0, 1), &a);
        assert_eq!(batch.len(), 1);
        assert_eq!(batch.chunk(slot(1, 0, 1)).quads[0][0].xyz(), [33, 2, 35]);
        assert!(batch.chunk(slot(0, 0, 0)).is_empty());
        // Re-copy with fewer quads shrinks the slot.
        batch.set_chunk(slot(1, 0, 1), &MeshChunkQuads::default());
        assert!(batch.is_empty());
    }

    #[test]
    fn pack_concatenates_slots_in_order() {
        let mut batch = RenderBatch::new();
        let mut packed = PackedBatch::default();
        for s in 0..SLOTS {
            let mut c = MeshChunkQuads::default();
            for dir in 0..6 {
                c.quads[dir].push(Quad::pack(0, 0, 0, 1, 1, s * 6 + dir));
            }
            batch.set_chunk(s, &c);
        }
        batch.pack_into(&mut packed);
        assert_eq!(packed.len(), SLOTS * 6);
        for dir in 0..6 {
            let ids: Vec<u64> = packed.quads[dir].iter().map(|q| q.voxel_id()).collect();
            let want: Vec<u64> = (0..SLOTS).map(|s| (s * 6 + dir) as u64).collect();
            assert_eq!(ids, want);
        }
        // Packing twice reuses the buffers without growing them.
        batch.pack_into(&mut packed);
        assert_eq!(packed.len(), SLOTS * 6);
    }

    #[test]
    fn boundary_faces_decode_to_block_63() {
        // A solid 32^3 chunk meshes to 6 boundary quads; placed in slot
        // (1,1,1) its positive planes sit at 64 (stored 0) and must decode
        // back to block coordinate 63, negative planes to block 32.
        let n = crate::SectionNeighborhood::from_fn(
            world::coords::MeshChunkPos { x: 0, y: 0, z: 0 },
            |sp| {
                if (0..=1).contains(&sp.x) && (0..=1).contains(&sp.y) && (0..=1).contains(&sp.z) {
                    world::Section::uniform(BlockStateId(1))
                } else {
                    world::Section::new()
                }
            },
        );
        let mut builder = MeshChunkBuilder::new();
        let quads = builder.build(&n, is_transparent);
        assert_eq!(quads.len(), 6);
        let mut batch = RenderBatch::new();
        batch.set_chunk(slot(1, 1, 1), &quads);
        let mut packed = PackedBatch::default();
        batch.pack_into(&mut packed);
        let faces = batch_faces(&packed.quads);
        assert_eq!(faces.len(), 6 * 32 * 32);
        for f in &faces {
            assert!(f.x >= 32 && f.y >= 32 && f.z >= 32);
        }
        // The same chunk at slot (0,0,0) of a fresh batch must decode
        // identically to the crate-based reference decoder (no wrapping).
        let mut batch = RenderBatch::new();
        batch.set_chunk(slot(0, 0, 0), &quads);
        batch.pack_into(&mut packed);
        let mut got = batch_faces(&packed.quads);
        let mut want = covered_faces(&quads);
        got.sort_unstable();
        want.sort_unstable();
        assert_eq!(got, want);
    }

    #[test]
    fn shifted_faces_equal_chunk_faces_plus_offset() {
        let n = crate::SectionNeighborhood::from_fn(
            world::coords::MeshChunkPos { x: 0, y: 0, z: 0 },
            |sp| {
                let mut arr = [BlockStateId::AIR; world::SECTION_VOLUME];
                for (i, cell) in arr.iter_mut().enumerate() {
                    let (x, y, z) = (i % 16, i / 256, (i / 16) % 16);
                    let b = (sp.x * 7 + sp.y * 13 + sp.z * 31 + (x + y * 3 + z * 5) as i32)
                        .rem_euclid(9);
                    *cell = BlockStateId(b as u16);
                }
                world::Section::from_blocks(&arr)
            },
        );
        let mut builder = MeshChunkBuilder::new();
        let quads = builder.build(&n, is_transparent);
        let base = covered_faces(&quads);
        for s in 0..SLOTS {
            let (sy, sz, sx) = (s / 4, (s / 2) % 2, s % 2);
            let mut batch = RenderBatch::new();
            batch.set_chunk(s, &quads);
            let mut packed = PackedBatch::default();
            batch.pack_into(&mut packed);
            let mut got = batch_faces(&packed.quads);
            let mut want: Vec<_> = base
                .iter()
                .map(|f| crate::reference::UnitFace {
                    x: f.x + (sx * 32) as u8,
                    y: f.y + (sy * 32) as u8,
                    z: f.z + (sz * 32) as u8,
                    ..*f
                })
                .collect();
            got.sort_unstable();
            want.sort_unstable();
            assert_eq!(got, want, "slot {s}");
        }
    }
}
