//! Naive reference mesher and quad-coverage decoding, for tests and
//! benchmarks (rule: an optimized implementation ships with a reference).
//!
//! Both sides produce the same unit: one [`UnitFace`] per visible block face
//! (position, direction, block id). Greedy merging changes the quad count,
//! not the covered faces, so comparisons sort face lists, not quad lists.

use crate::{CS, Face, MeshChunkQuads, P3, pad_index};
use world::BlockStateId;

/// One visible unit face of a block at chunk-local coordinates `0..=31`.
/// `dir` indexes [`Face`] (0 Up, 1 Down, 2 Right, 3 Left, 4 Front, 5 Back).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct UnitFace {
    /// Chunk-local x of the block.
    pub x: u8,
    /// Chunk-local y of the block.
    pub y: u8,
    /// Chunk-local z of the block.
    pub z: u8,
    /// Face direction, 0..=5.
    pub dir: u8,
    /// Block state id of the face's block.
    pub id: u16,
}

/// Face normals in `Face` order (Up, Down, Right, Left, Front, Back).
const NORMALS: [[i32; 3]; 6] = [
    [0, 1, 0],
    [0, -1, 0],
    [1, 0, 0],
    [-1, 0, 0],
    [0, 0, 1],
    [0, 0, -1],
];

/// Per-face culling without merging: a face of block `b` towards neighbor `n`
/// is visible iff `n` is air, or `n` is transparent and a different state.
/// Reads the padded buffer, emits faces only for core blocks.
#[must_use]
pub fn naive_faces(
    voxels: &[u16; P3],
    is_transparent: impl Fn(BlockStateId) -> bool,
) -> Vec<UnitFace> {
    let mut out = Vec::new();
    for y in 0..CS {
        for x in 0..CS {
            for z in 0..CS {
                let b = voxels[pad_index(x + 1, y + 1, z + 1)];
                if b == 0 {
                    continue;
                }
                for (dir, n) in NORMALS.iter().enumerate() {
                    let nx = (x as i32 + 1 + n[0]) as usize;
                    let ny = (y as i32 + 1 + n[1]) as usize;
                    let nz = (z as i32 + 1 + n[2]) as usize;
                    let nb = voxels[pad_index(nx, ny, nz)];
                    if nb == 0 || (nb != b && is_transparent(BlockStateId(nb))) {
                        out.push(UnitFace {
                            x: x as u8,
                            y: y as u8,
                            z: z as u8,
                            dir: dir as u8,
                            id: b,
                        });
                    }
                }
            }
        }
    }
    out
}

/// Expands greedy quads back into the unit faces they cover, using the
/// crate's own `Face::vertices_packed` geometry: the quad's corner bounding
/// box gives the plane and the 2D extent; the block sits on the inner side
/// of the plane along the face normal.
#[must_use]
pub fn covered_faces(quads: &MeshChunkQuads) -> Vec<UnitFace> {
    let mut out = Vec::new();
    for (dir, list) in quads.quads.iter().enumerate() {
        let face = Face::from(dir as u8);
        let n = face.n();
        let axis = if n[0] != 0 {
            0
        } else if n[1] != 0 {
            1
        } else {
            2
        };
        let (a1, a2) = ((axis + 1) % 3, (axis + 2) % 3);
        for &quad in list {
            let mut min = [u32::MAX; 3];
            let mut max = [0u32; 3];
            for v in face.vertices_packed(quad) {
                let p = v.xyz();
                for a in 0..3 {
                    min[a] = min[a].min(p[a]);
                    max[a] = max[a].max(p[a]);
                }
            }
            debug_assert_eq!(min[axis], max[axis], "quad is not planar");
            let plane = min[axis];
            let block_plane = if n[axis] > 0 { plane - 1 } else { plane };
            for u in min[a1]..max[a1] {
                for v in min[a2]..max[a2] {
                    let mut p = [0u32; 3];
                    p[axis] = block_plane;
                    p[a1] = u;
                    p[a2] = v;
                    debug_assert!(p.iter().all(|&c| c < CS as u32));
                    out.push(UnitFace {
                        x: p[0] as u8,
                        y: p[1] as u8,
                        z: p[2] as u8,
                        dir: dir as u8,
                        id: quad.voxel_id() as u16,
                    });
                }
            }
        }
    }
    out
}

/// Convenience for tests: sorted covered faces of the greedy output.
#[must_use]
pub fn covered_faces_sorted(quads: &MeshChunkQuads) -> Vec<UnitFace> {
    let mut f = covered_faces(quads);
    f.sort_unstable();
    f
}

/// Convenience for tests: sorted faces of the naive reference.
#[must_use]
pub fn naive_faces_sorted(
    voxels: &[u16; P3],
    is_transparent: impl Fn(BlockStateId) -> bool,
) -> Vec<UnitFace> {
    let mut f = naive_faces(voxels, is_transparent);
    f.sort_unstable();
    f
}
