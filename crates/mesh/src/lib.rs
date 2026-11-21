//! Mesh chunks 32^3: gather from sections, incremental opacity masks and
//! binary greedy meshing (`binary-greedy-meshing`, `Mesher<32>::fast_mesh`).
//!
//! Spec:
//! - `docs/architecture/meshing-and-rendering.md`
//! - `docs/adr/0005-mesh-chunk-32-with-batches.md`
//!
//! The voxel buffer uses the crate's padded ZXY order: 34^3 cells, one block of
//! padding on each side, index `(y * 34 + x) * 34 + z` with padded coordinates
//! `0..=33` (chunk-local block `b` sits at padded coordinate `b + 1`).
//!
//! Render batches ([`RenderBatch`], [`batch`]) copy the quads of 8 mesh chunks
//! into a 64^3 batch with 0/32 offsets, 6 ranges by face direction.

pub mod batch;
pub mod reference;

pub use batch::{PackedBatch, RenderBatch};

use binary_greedy_meshing::Mesher;
use world::coords::{BlockPos, MeshChunkPos, SectionPos};
use world::{BlockStateId, MESH_CHUNK_SIZE, Section};

pub use binary_greedy_meshing::{Face, Quad};

/// Edge length of a mesh chunk in blocks.
pub const CS: usize = MESH_CHUNK_SIZE as usize;
/// Edge length of the padded voxel buffer (CS + 2).
pub const P: usize = CS + 2;
/// Columns in the padded buffer (one u64 mask word per column).
pub const P2: usize = P * P;
/// Cells in the padded voxel buffer.
pub const P3: usize = P * P * P;

const _: () = assert!(CS == Mesher::<CS>::CS_2 / CS && P == Mesher::<CS>::CS_P);
const _: () = assert!(P2 == Mesher::<CS>::CS_P2 && P3 == Mesher::<CS>::CS_P3);

/// Linear index into the padded buffer, ZXY order (padded coordinates).
#[inline]
#[must_use]
pub const fn pad_index(x: usize, y: usize, z: usize) -> usize {
    (y * P + x) * P + z
}

/// The 4x4x4 sections touched by a mesh chunk and its one-block padding:
/// the 2x2x2 core plus the surrounding shell. Indexed by local section
/// coordinates `-1..=2` on each axis, y-major.
pub struct SectionNeighborhood {
    sections: Box<[Section; 64]>,
}

impl SectionNeighborhood {
    /// Wraps 64 sections in local order `(sy + 1) * 16 + (sz + 1) * 4 + (sx + 1)`,
    /// where `sx, sy, sz` are in `-1..=2` relative to the mesh chunk's core.
    #[must_use]
    pub fn from_array(sections: [Section; 64]) -> Self {
        Self {
            sections: Box::new(sections),
        }
    }

    /// Builds the neighborhood of `chunk`, requesting each section by its
    /// absolute position. Missing sections should be returned as all-air.
    pub fn from_fn(chunk: MeshChunkPos, mut f: impl FnMut(SectionPos) -> Section) -> Self {
        let base = SectionPos {
            x: chunk.x * 2 - 1,
            y: chunk.y * 2 - 1,
            z: chunk.z * 2 - 1,
        };
        let sections = std::array::from_fn(|i| {
            let (sy, rem) = (i / 16, i % 16);
            let (sz, sx) = (rem / 4, rem % 4);
            f(SectionPos {
                x: base.x + sx as i32,
                y: base.y + sy as i32,
                z: base.z + sz as i32,
            })
        });
        Self {
            sections: Box::new(sections),
        }
    }

    /// Section at local coordinates `-1..=2` (0 and 1 are the core).
    ///
    /// # Panics
    /// Panics if any coordinate is outside `-1..=2`.
    #[must_use]
    pub fn section(&self, sx: i32, sy: i32, sz: i32) -> &Section {
        assert!((-1..=2).contains(&sx) && (-1..=2).contains(&sy) && (-1..=2).contains(&sz));
        let i = ((sy + 1) * 16 + (sz + 1) * 4 + (sx + 1)) as usize;
        &self.sections[i]
    }
}

/// Opaque and transparent column masks for `fast_mesh`: one u64 per padded
/// `(y, x)` column, bit `z` set when the cell is opaque / transparent.
/// Built from scratch on chunk load, maintained incrementally on edits.
pub struct MaskCache {
    opaque: Box<[u64; P2]>,
    transparent: Box<[u64; P2]>,
}

impl Default for MaskCache {
    fn default() -> Self {
        Self::new()
    }
}

impl MaskCache {
    /// Zeroed masks.
    #[must_use]
    pub fn new() -> Self {
        Self {
            opaque: Box::new([0; P2]),
            transparent: Box::new([0; P2]),
        }
    }

    /// Rebuilds both masks from the whole padded buffer (chunk load path).
    pub fn rebuild(&mut self, voxels: &[u16; P3], is_transparent: impl Fn(BlockStateId) -> bool) {
        self.opaque.fill(0);
        self.transparent.fill(0);
        for (i, &v) in voxels.iter().enumerate() {
            if v == 0 {
                continue;
            }
            let (r, q) = (i / P, i % P);
            if is_transparent(BlockStateId(v)) {
                self.transparent[r] |= 1 << q;
            } else {
                self.opaque[r] |= 1 << q;
            }
        }
    }

    /// Incremental update after a block edit at chunk-local coordinates
    /// `0..=31`. Only the one affected bit is touched (target ≤ 20 ns).
    ///
    /// # Panics
    /// Panics if any coordinate is outside `0..=31`.
    pub fn on_block_set(
        &mut self,
        x: usize,
        y: usize,
        z: usize,
        new: BlockStateId,
        is_transparent: impl Fn(BlockStateId) -> bool,
    ) {
        assert!(x < CS && y < CS && z < CS);
        let r = (y + 1) * P + (x + 1);
        let bit = 1u64 << (z + 1);
        self.opaque[r] &= !bit;
        self.transparent[r] &= !bit;
        if new != BlockStateId::AIR {
            if is_transparent(new) {
                self.transparent[r] |= bit;
            } else {
                self.opaque[r] |= bit;
            }
        }
    }

    /// Opaque column masks, ready for `fast_mesh`.
    #[must_use]
    pub fn opaque(&self) -> &[u64; P2] {
        &self.opaque
    }

    /// Transparent column masks, ready for `fast_mesh`.
    #[must_use]
    pub fn transparent(&self) -> &[u64; P2] {
        &self.transparent
    }
}

/// A section's intersection with the padded box: `lo..hi` in section-local
/// coordinates, `base` the padded coordinate of `lo`, per axis (x, y, z).
#[derive(Clone, Copy)]
struct SectionBox {
    lo: [usize; 3],
    hi: [usize; 3],
    base: [usize; 3],
}

/// Copies one section's intersection box into the padded buffer, reading
/// section-local cells through `read` and accumulating mask bits per column.
fn copy_box(
    voxels: &mut [u16; P3],
    opaque: &mut [u64; P2],
    transparent: &mut [u64; P2],
    b: SectionBox,
    read: impl Fn(usize, usize, usize) -> BlockStateId,
    is_transparent: &impl Fn(BlockStateId) -> bool,
) {
    let (lo, hi, base) = (b.lo, b.hi, b.base);
    for y in lo[1]..hi[1] {
        for x in lo[0]..hi[0] {
            let r = (base[1] + y - lo[1]) * P + (base[0] + x - lo[0]);
            let col = &mut voxels[r * P + base[2]..r * P + base[2] + (hi[2] - lo[2])];
            let mut ob = 0u64;
            let mut tb = 0u64;
            for (k, z) in (lo[2]..hi[2]).enumerate() {
                let v = read(x, y, z);
                col[k] = v.0;
                if v.0 != 0 {
                    let bit = 1u64 << (base[2] + k);
                    if is_transparent(v) {
                        tb |= bit;
                    } else {
                        ob |= bit;
                    }
                }
            }
            opaque[r] |= ob;
            transparent[r] |= tb;
        }
    }
}

/// Greedy-meshed output: 6 quad lists indexed by [`Face`]
/// (Up, Down, Right, Left, Front, Back). Quads are the crate's 8-byte packed
/// format with chunk-local coordinates; the batch copy ([`batch`]) adds the
/// 0/32 offsets.
#[derive(Clone, Debug, Default)]
pub struct MeshChunkQuads {
    /// One quad list per face direction.
    pub quads: [Vec<Quad>; 6],
}

impl MeshChunkQuads {
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

    /// Clears all lists, keeping their capacity for the next re-mesh.
    pub fn clear(&mut self) {
        for q in &mut self.quads {
            q.clear();
        }
    }
}

/// Reusable meshing pipeline for one 32^3 mesh chunk: gather, masks, mesher.
///
/// Load path: [`Self::build`] / [`Self::build_into`] (gather + masks from
/// scratch + `fast_mesh`). Edit path: [`Self::set_block`] (buffer + masks
/// incrementally) then [`Self::remesh_into`]. No allocations on the edit path
/// when the output quads are reused.
pub struct MeshChunkBuilder {
    mesher: Mesher<CS>,
    voxels: Box<[u16; P3]>,
    masks: MaskCache,
    scratch: Box<[BlockStateId; world::SECTION_VOLUME]>,
    non_air_core: u32,
    solid: bool,
}

impl Default for MeshChunkBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl MeshChunkBuilder {
    /// Allocates the buffers (once per worker, not per chunk).
    #[must_use]
    pub fn new() -> Self {
        Self {
            mesher: Mesher::new(),
            voxels: Box::new([0; P3]),
            masks: MaskCache::new(),
            scratch: Box::new([BlockStateId::AIR; world::SECTION_VOLUME]),
            non_air_core: 0,
            solid: false,
        }
    }

    /// Gathers the 34^3 buffer and rebuilds the masks from the neighborhood.
    /// Core sections are unpacked word by word (`Section::unpack_into`);
    /// uniform sections are filled without unpacking; the thin padding shell
    /// is read per cell (see `gather_section`). Called on chunk load; edits
    /// use [`Self::set_block`] instead.
    pub fn gather(
        &mut self,
        neighborhood: &SectionNeighborhood,
        is_transparent: impl Fn(BlockStateId) -> bool,
    ) {
        self.masks.opaque.fill(0);
        self.masks.transparent.fill(0);
        self.non_air_core = 0;
        self.solid = true;
        let is_opaque = |b: BlockStateId| b != BlockStateId::AIR && !is_transparent(b);
        for sy in -1..=2i32 {
            for sz in -1..=2i32 {
                for sx in -1..=2i32 {
                    let section = neighborhood.section(sx, sy, sz);
                    let core =
                        (0..=1).contains(&sx) && (0..=1).contains(&sy) && (0..=1).contains(&sz);
                    if core {
                        self.non_air_core += u32::from(section.non_air());
                    }
                    self.solid &= section.all_opaque(is_opaque);
                    self.gather_section(sx, sy, sz, section, &is_transparent);
                }
            }
        }
    }

    /// Copies one section's intersection with the padded box into the buffer
    /// and accumulates its mask bits column by column.
    ///
    /// Core sections (full 16^3 intersection) are unpacked word by word.
    /// Shell sections contribute only 1..=256 padding cells each, so they
    /// are read with per-cell `get` instead: decoding a whole 4096-cell
    /// section to use a 1-block-thick sliver measured ~3x slower on the
    /// T0-04 corpus (see the task report).
    fn gather_section(
        &mut self,
        sx: i32,
        sy: i32,
        sz: i32,
        section: &Section,
        is_transparent: &impl Fn(BlockStateId) -> bool,
    ) {
        // Intersection of the section's 16^3 box with the padded box (-1..33)
        // in section-local coordinates, and the padded origin of the box.
        let mut lo = [0usize; 3];
        let mut hi = [16usize; 3];
        let mut base = [0usize; 3];
        for (axis, s) in [sx, sy, sz].into_iter().enumerate() {
            let start = s * 16;
            let a = start.max(-1) - start;
            let b = (start + 16).min(33) - start;
            lo[axis] = a as usize;
            hi[axis] = b as usize;
            base[axis] = (start + a + 1) as usize;
        }
        let zlen = hi[2] - lo[2];
        if section.bits() == 0 {
            // Uniform section: fill runs and set whole mask bit ranges.
            let v = section.get(0);
            let (ob, tb) = if v == BlockStateId::AIR {
                (0, 0)
            } else if is_transparent(v) {
                (0, ((1u64 << zlen) - 1) << base[2])
            } else {
                (((1u64 << zlen) - 1) << base[2], 0)
            };
            for y in lo[1]..hi[1] {
                for x in lo[0]..hi[0] {
                    let r = (base[1] + y - lo[1]) * P + (base[0] + x - lo[0]);
                    self.voxels[r * P + base[2]..r * P + base[2] + zlen].fill(v.0);
                    self.masks.opaque[r] |= ob;
                    self.masks.transparent[r] |= tb;
                }
            }
            return;
        }
        let full = (hi[0] - lo[0]) * (hi[1] - lo[1]) * zlen == world::SECTION_VOLUME;
        let bounds = SectionBox { lo, hi, base };
        if full {
            section.unpack_into(&mut self.scratch);
            copy_box(
                &mut self.voxels,
                &mut self.masks.opaque,
                &mut self.masks.transparent,
                bounds,
                |x, y, z| self.scratch[(y * 16 + z) * 16 + x],
                is_transparent,
            );
        } else {
            copy_box(
                &mut self.voxels,
                &mut self.masks.opaque,
                &mut self.masks.transparent,
                bounds,
                |x, y, z| section.get((y * 16 + z) * 16 + x),
                is_transparent,
            );
        }
    }

    /// Gathers, rebuilds masks and meshes, returning fresh quad lists.
    /// Empty and solid chunks are skipped by section flags without calling
    /// the mesher.
    pub fn build(
        &mut self,
        neighborhood: &SectionNeighborhood,
        is_transparent: impl Fn(BlockStateId) -> bool,
    ) -> MeshChunkQuads {
        let mut out = MeshChunkQuads::default();
        self.build_into(neighborhood, is_transparent, &mut out);
        out
    }

    /// Like [`Self::build`], reusing the capacity of `out` (no allocation
    /// after warm-up).
    pub fn build_into(
        &mut self,
        neighborhood: &SectionNeighborhood,
        is_transparent: impl Fn(BlockStateId) -> bool,
        out: &mut MeshChunkQuads,
    ) {
        engine_core::zone!("mesh::build");
        self.gather(neighborhood, is_transparent);
        self.remesh_into(out);
    }

    /// Applies a block edit at chunk-local coordinates `0..=31` to the
    /// gathered buffer and updates the masks incrementally. The caller
    /// re-meshes with [`Self::remesh_into`] and marks boundary neighbors
    /// dirty via [`affected_mesh_chunks`].
    ///
    /// # Panics
    /// Panics if any coordinate is outside `0..=31`.
    pub fn set_block(
        &mut self,
        x: usize,
        y: usize,
        z: usize,
        new: BlockStateId,
        is_transparent: impl Fn(BlockStateId) -> bool,
    ) {
        assert!(x < CS && y < CS && z < CS);
        let i = pad_index(x + 1, y + 1, z + 1);
        let old = self.voxels[i];
        if old == new.0 {
            return;
        }
        self.voxels[i] = new.0;
        self.masks.on_block_set(x, y, z, new, &is_transparent);
        self.non_air_core += u32::from(new != BlockStateId::AIR);
        self.non_air_core -= u32::from(old != 0);
        if new == BlockStateId::AIR || is_transparent(new) {
            self.solid = false;
        }
    }

    /// Re-meshes with the cached masks, reusing the capacity of `out`.
    /// Skips the mesher for empty and solid chunks.
    pub fn remesh_into(&mut self, out: &mut MeshChunkQuads) {
        engine_core::zone!("mesh::remesh");
        out.clear();
        if self.non_air_core == 0 || self.solid {
            return;
        }
        self.mesher.clear();
        self.mesher
            .fast_mesh(&*self.voxels, &*self.masks.opaque, &*self.masks.transparent);
        for (dst, src) in out.quads.iter_mut().zip(&self.mesher.quads) {
            dst.extend_from_slice(src);
        }
    }

    /// The gathered padded buffer (ZXY order, see [`pad_index`]).
    #[must_use]
    pub fn voxels(&self) -> &[u16; P3] {
        &self.voxels
    }

    /// The current masks.
    #[must_use]
    pub fn masks(&self) -> &MaskCache {
        &self.masks
    }
}

/// The mesh chunks whose geometry or padding contains `pos`: the chunk itself
/// plus up to 3 face neighbors when the edit sits on a chunk border (up to 4
/// total; diagonal AO neighbors are added in a later task). All of them must
/// be marked dirty after an edit at `pos`.
#[derive(Clone, Copy, Debug)]
pub struct AffectedChunks {
    chunks: [MeshChunkPos; 4],
    len: usize,
}

impl AffectedChunks {
    /// The affected chunks; the first one always contains `pos`.
    #[must_use]
    pub fn as_slice(&self) -> &[MeshChunkPos] {
        &self.chunks[..self.len]
    }
}

/// Computes which mesh chunks must be re-meshed after an edit at `pos`.
#[must_use]
pub fn affected_mesh_chunks(pos: BlockPos) -> AffectedChunks {
    let base = pos.mesh_chunk();
    let mut chunks = [base; 4];
    let mut len = 1;
    let local = [
        pos.x.rem_euclid(MESH_CHUNK_SIZE),
        pos.y.rem_euclid(MESH_CHUNK_SIZE),
        pos.z.rem_euclid(MESH_CHUNK_SIZE),
    ];
    for (axis, &l) in local.iter().enumerate() {
        let delta = if l == 0 {
            -1
        } else if l == MESH_CHUNK_SIZE - 1 {
            1
        } else {
            continue;
        };
        let mut p = base;
        match axis {
            0 => p.x += delta,
            1 => p.y += delta,
            _ => p.z += delta,
        }
        chunks[len] = p;
        len += 1;
    }
    AffectedChunks { chunks, len }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stone() -> BlockStateId {
        BlockStateId(1)
    }

    fn chunk(x: i32, y: i32, z: i32) -> MeshChunkPos {
        MeshChunkPos { x, y, z }
    }

    fn glass() -> BlockStateId {
        BlockStateId(7)
    }

    fn is_transparent(b: BlockStateId) -> bool {
        matches!(b.0, 5..=7)
    }

    /// Builds a neighborhood from a function over chunk-local block
    /// coordinates (padding included, -1..=32); outside is air.
    fn neighborhood_from(f: impl Fn(i32, i32, i32) -> BlockStateId) -> SectionNeighborhood {
        SectionNeighborhood::from_fn(chunk(0, 0, 0), |sp| {
            let mut arr = [BlockStateId::AIR; world::SECTION_VOLUME];
            for y in 0..16i32 {
                for z in 0..16i32 {
                    for x in 0..16i32 {
                        let (bx, by, bz) = (sp.x * 16 + x, sp.y * 16 + y, sp.z * 16 + z);
                        if (-1..=32).contains(&bx)
                            && (-1..=32).contains(&by)
                            && (-1..=32).contains(&bz)
                        {
                            arr[((y * 16 + z) * 16 + x) as usize] = f(bx, by, bz);
                        }
                    }
                }
            }
            Section::from_blocks(&arr)
        })
    }

    #[test]
    fn gather_matches_source_field() {
        let f =
            |x: i32, y: i32, z: i32| BlockStateId(((x * 7 + y * 13 + z * 31).rem_euclid(9)) as u16);
        let n = neighborhood_from(f);
        let mut b = MeshChunkBuilder::new();
        b.gather(&n, is_transparent);
        for y in -1..=32i32 {
            for x in -1..=32i32 {
                for z in -1..=32i32 {
                    let i = pad_index((x + 1) as usize, (y + 1) as usize, (z + 1) as usize);
                    assert_eq!(b.voxels()[i], f(x, y, z).0, "at {x},{y},{z}");
                }
            }
        }
        let mut rebuilt = MaskCache::new();
        rebuilt.rebuild(b.voxels(), is_transparent);
        assert_eq!(b.masks().opaque(), rebuilt.opaque());
        assert_eq!(b.masks().transparent(), rebuilt.transparent());
    }

    #[test]
    fn uniform_and_paletted_gather_agree() {
        let n_uniform = SectionNeighborhood::from_fn(chunk(0, 0, 0), |_| Section::uniform(stone()));
        let n_paletted = neighborhood_from(|_, _, _| stone());
        let mut a = MeshChunkBuilder::new();
        let mut b = MeshChunkBuilder::new();
        a.gather(&n_uniform, is_transparent);
        b.gather(&n_paletted, is_transparent);
        assert_eq!(a.voxels(), b.voxels());
        assert_eq!(a.masks().opaque(), b.masks().opaque());
        assert_eq!(a.masks().transparent(), b.masks().transparent());
    }

    #[test]
    fn empty_and_solid_chunks_skip_mesher() {
        let empty = SectionNeighborhood::from_fn(chunk(0, 0, 0), |_| Section::new());
        let solid = SectionNeighborhood::from_fn(chunk(0, 0, 0), |_| Section::uniform(stone()));
        let mut b = MeshChunkBuilder::new();
        assert!(b.build(&empty, is_transparent).is_empty());
        assert!(b.build(&solid, is_transparent).is_empty());
    }

    #[test]
    fn solid_core_with_air_padding_still_meshes_boundary() {
        // Core all stone, padding air: the 6 boundary faces must be emitted.
        let n = neighborhood_from(|x, y, z| {
            if (0..32).contains(&x) && (0..32).contains(&y) && (0..32).contains(&z) {
                stone()
            } else {
                BlockStateId::AIR
            }
        });
        let mut b = MeshChunkBuilder::new();
        let q = b.build(&n, is_transparent);
        // 6 faces of 32x32, greedily merged to 1 quad each.
        assert_eq!(q.len(), 6);
        let faces = reference::covered_faces(&q);
        assert_eq!(faces.len(), 6 * 32 * 32);
    }

    #[test]
    fn set_block_updates_buffer_and_masks_incrementally() {
        let n = neighborhood_from(|_, y, _| if y < 16 { stone() } else { BlockStateId::AIR });
        let mut b = MeshChunkBuilder::new();
        b.gather(&n, is_transparent);
        b.set_block(5, 20, 9, glass(), is_transparent);
        b.set_block(3, 4, 7, BlockStateId::AIR, is_transparent);
        b.set_block(31, 31, 31, stone(), is_transparent);
        let mut rebuilt = MaskCache::new();
        rebuilt.rebuild(b.voxels(), is_transparent);
        assert_eq!(b.masks().opaque(), rebuilt.opaque());
        assert_eq!(b.masks().transparent(), rebuilt.transparent());
        assert_eq!(b.voxels()[pad_index(6, 21, 10)], 7);
        assert_eq!(b.voxels()[pad_index(4, 5, 8)], 0);
    }

    #[test]
    fn interior_edit_affects_only_own_chunk() {
        let a = affected_mesh_chunks(BlockPos::new(5, 5, 5));
        assert_eq!(a.as_slice(), &[chunk(0, 0, 0)]);
    }

    #[test]
    fn boundary_edit_marks_face_neighbors() {
        let a = affected_mesh_chunks(BlockPos::new(31, 16, 16));
        assert_eq!(a.as_slice(), &[chunk(0, 0, 0), chunk(1, 0, 0)]);
        let a = affected_mesh_chunks(BlockPos::new(0, 0, 16));
        assert_eq!(
            a.as_slice(),
            &[chunk(0, 0, 0), chunk(-1, 0, 0), chunk(0, -1, 0)]
        );
    }

    #[test]
    fn corner_edit_marks_four_chunks() {
        let a = affected_mesh_chunks(BlockPos::new(0, 0, 0));
        assert_eq!(
            a.as_slice(),
            &[
                chunk(0, 0, 0),
                chunk(-1, 0, 0),
                chunk(0, -1, 0),
                chunk(0, 0, -1)
            ]
        );
        // Negative coordinates: block -1 is local 31 of chunk -1.
        let a = affected_mesh_chunks(BlockPos::new(-1, -1, -1));
        assert_eq!(
            a.as_slice(),
            &[
                chunk(-1, -1, -1),
                chunk(0, -1, -1),
                chunk(-1, 0, -1),
                chunk(-1, -1, 0)
            ]
        );
    }

    #[test]
    fn single_block_mesh_matches_reference() {
        let n = neighborhood_from(|x, y, z| {
            if (x, y, z) == (16, 16, 16) {
                stone()
            } else {
                BlockStateId::AIR
            }
        });
        let mut b = MeshChunkBuilder::new();
        let q = b.build(&n, is_transparent);
        assert_eq!(q.len(), 6);
        let mut got = reference::covered_faces(&q);
        let mut want = reference::naive_faces(b.voxels(), is_transparent);
        got.sort_unstable();
        want.sort_unstable();
        assert_eq!(got, want);
    }
}
