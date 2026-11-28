//! The mesher boundary and render buckets (ADR-0016).
//!
//! A [`ChunkMesher`] turns one gathered, padded 32^3 mesh chunk
//! ([`MeshInput`]) into render buckets ([`crate::MeshChunkQuads`]). Dispatch
//! is per mesh chunk and static: [`crate::MeshChunkBuilder::remesh_into`]
//! calls the concrete cubic mesher, and
//! [`crate::MeshChunkBuilder::remesh_with`] takes any mesher by generic
//! parameter, so the per-cell loop stays monomorphic (no virtual call per
//! cell).
//! [`CubicGreedyMesher`] is the only implementation; which mesher a block
//! uses is the registry property [`MesherKind`] (default cubic).
//!
//! Physics, raycasts and the server never read mesh output: they use the
//! registry's collision shapes (`world::Collision`), see ADR-0016 decision 3.

use binary_greedy_meshing::Mesher;
use world::BlockStateId;

use crate::{CS, MeshChunkQuads, P2, P3};

pub use world::MesherKind;

/// Geometry type of a render-batch range. A batch holds one range per
/// bucket; empty buckets cost nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RenderBucket {
    /// 8-byte packed greedy quads (paths B, Bi and C).
    PackedVoxelQuads,
    /// Instanced block-model templates (stairs, fences, plants), one
    /// [`ModelInstance`] per block. Ranges are kept per batch; the drawing
    /// pipeline arrives with block models.
    ModelInstances,
    /// Indexed smooth vertices. Declared only: not implemented until a
    /// decision to use it (open question 18).
    SmoothMesh,
    /// Displaceable water grid. Declared only: not implemented until a
    /// decision to use it (open question 18).
    WaterSurface,
}

impl RenderBucket {
    /// Every bucket, in range order.
    pub const ALL: [Self; 4] = [
        Self::PackedVoxelQuads,
        Self::ModelInstances,
        Self::SmoothMesh,
        Self::WaterSurface,
    ];

    /// Whether meshers may write this bucket today.
    #[must_use]
    pub const fn is_implemented(self) -> bool {
        matches!(self, Self::PackedVoxelQuads | Self::ModelInstances)
    }
}

/// One instanced block model in the `ModelInstances` bucket: the block's
/// min corner (6 bits per axis, same field layout as [`crate::Quad`] so the
/// batch 0/32 offset is the same bit flip) and its block state.
///
/// Bits: `0..6` x, `6..12` y, `12..18` z, `18..34` block state id.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct ModelInstance(pub u64);

impl ModelInstance {
    /// Packs a min corner (`0..64` per axis) and a block state.
    ///
    /// # Panics
    /// Panics if a coordinate is 64 or more.
    #[must_use]
    pub const fn pack(x: u32, y: u32, z: u32, state: BlockStateId) -> Self {
        assert!(x < 64 && y < 64 && z < 64);
        Self(x as u64 | (y as u64) << 6 | (z as u64) << 12 | (state.0 as u64) << 18)
    }

    /// The min corner `[x, y, z]`.
    #[must_use]
    pub const fn xyz(self) -> [u32; 3] {
        [
            (self.0 & 63) as u32,
            ((self.0 >> 6) & 63) as u32,
            ((self.0 >> 12) & 63) as u32,
        ]
    }

    /// The block state.
    #[must_use]
    pub const fn state(self) -> BlockStateId {
        BlockStateId(((self.0 >> 18) & 0xffff) as u16)
    }
}

/// The gathered input of a mesher: the padded 34^3 voxel buffer (ZXY order,
/// see [`crate::pad_index`]) and its opaque / transparent column masks.
/// A density layer joins here when smooth terrain is decided (ADR-0016,
/// decision 4).
#[derive(Clone, Copy)]
pub struct MeshInput<'a> {
    /// Block states of the padded buffer.
    pub voxels: &'a [u16; P3],
    /// Opaque column masks (bit `z` per padded `(y, x)` column).
    pub opaque: &'a [u64; P2],
    /// Transparent column masks.
    pub transparent: &'a [u64; P2],
}

/// Turns one gathered mesh chunk into render buckets.
pub trait ChunkMesher {
    /// The registry value that selects this mesher.
    const KIND: MesherKind;
    /// The buckets this mesher writes.
    const BUCKETS: &'static [RenderBucket];

    /// Meshes `input`, appending to the buckets of `out`. The caller clears
    /// `out` first; implementations reuse its capacity.
    fn mesh(&mut self, input: MeshInput<'_>, out: &mut MeshChunkQuads);
}

/// The binary greedy mesher (`binary-greedy-meshing`, `fast_mesh`) writing
/// [`RenderBucket::PackedVoxelQuads`]: the engine's original mesher.
pub struct CubicGreedyMesher {
    inner: Mesher<CS>,
}

impl Default for CubicGreedyMesher {
    fn default() -> Self {
        Self::new()
    }
}

impl CubicGreedyMesher {
    /// Allocates the mesher's buffers (once per worker).
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Mesher::new(),
        }
    }
}

impl ChunkMesher for CubicGreedyMesher {
    const KIND: MesherKind = MesherKind::Cubic;
    const BUCKETS: &'static [RenderBucket] = &[RenderBucket::PackedVoxelQuads];

    fn mesh(&mut self, input: MeshInput<'_>, out: &mut MeshChunkQuads) {
        self.inner.clear();
        self.inner
            .fast_mesh(input.voxels, input.opaque, input.transparent);
        for (dst, src) in out.quads.iter_mut().zip(&self.inner.quads) {
            dst.extend_from_slice(src);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_instance_roundtrip() {
        let m = ModelInstance::pack(63, 0, 31, BlockStateId(0xbeef));
        assert_eq!(m.xyz(), [63, 0, 31]);
        assert_eq!(m.state(), BlockStateId(0xbeef));
    }

    #[test]
    fn only_quads_and_models_are_implemented() {
        let implemented: Vec<_> = RenderBucket::ALL
            .into_iter()
            .filter(|b| b.is_implemented())
            .collect();
        assert_eq!(
            implemented,
            [RenderBucket::PackedVoxelQuads, RenderBucket::ModelInstances]
        );
        assert!(
            CubicGreedyMesher::BUCKETS
                .iter()
                .all(|b| b.is_implemented())
        );
        assert_eq!(CubicGreedyMesher::KIND, MesherKind::Cubic);
    }
}
