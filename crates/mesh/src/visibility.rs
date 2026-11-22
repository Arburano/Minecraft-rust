//! Mesh chunk face-to-face visibility for cave culling
//! (`meshing-and-rendering.md`, "Culling", item 3).
//!
//! For each 32³ mesh chunk: which pairs of its six faces are connected through
//! non-opaque cells. A traversal from the camera only enters a neighbor
//! through a face reachable from the face it came in by; sealed caves and
//! solid ground stop it.

use crate::{CS, P3, pad_index};

/// Face indices: 0 +Y, 1 −Y, 2 +X, 3 −X, 4 +Z, 5 −Z.
pub const FACES: usize = 6;
/// Unit offsets of the faces, in mesh chunks.
pub const FACE_OFFSETS: [[i32; 3]; 6] = [
    [0, 1, 0],
    [0, -1, 0],
    [1, 0, 0],
    [-1, 0, 0],
    [0, 0, 1],
    [0, 0, -1],
];

/// The opposite face.
#[must_use]
pub const fn opposite(face: usize) -> usize {
    face ^ 1
}

/// Symmetric 6×6 connectivity matrix, one bit per face pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ChunkVisibility(u64);

impl ChunkVisibility {
    /// Every face sees every face (empty or unknown chunks).
    pub const ALL: Self = Self((1 << 36) - 1);
    /// No face sees another (solid chunks).
    pub const NONE: Self = Self(0);

    /// Whether light/sight entering through `a` can leave through `b`.
    #[must_use]
    pub const fn connected(self, a: usize, b: usize) -> bool {
        self.0 & (1 << (a * FACES + b)) != 0
    }

    fn connect(&mut self, a: usize, b: usize) {
        self.0 |= (1 << (a * FACES + b)) | (1 << (b * FACES + a));
    }

    /// Computes the matrix by flood-filling the non-opaque cells of the core
    /// of a gathered mesh chunk (`MeshChunkBuilder::voxels`).
    #[must_use]
    pub fn from_voxels(voxels: &[u16; P3], is_opaque: impl Fn(u16) -> bool) -> Self {
        let at = |x: usize, y: usize, z: usize| voxels[pad_index(x + 1, y + 1, z + 1)];
        let idx = |x: usize, y: usize, z: usize| (y * CS + z) * CS + x;
        let mut visited = vec![false; CS * CS * CS];
        let mut stack: Vec<(u8, u8, u8)> = Vec::new();
        let mut result = Self::NONE;
        let mut open_cells = 0usize;
        for y in 0..CS {
            for z in 0..CS {
                for x in 0..CS {
                    if visited[idx(x, y, z)] || is_opaque(at(x, y, z)) {
                        continue;
                    }
                    // New component.
                    let mut touched = 0u8;
                    visited[idx(x, y, z)] = true;
                    stack.push((x as u8, y as u8, z as u8));
                    while let Some((cx, cy, cz)) = stack.pop() {
                        let (cx, cy, cz) = (cx as usize, cy as usize, cz as usize);
                        open_cells += 1;
                        let last = CS - 1;
                        touched |= u8::from(cy == last)
                            | u8::from(cy == 0) << 1
                            | u8::from(cx == last) << 2
                            | u8::from(cx == 0) << 3
                            | u8::from(cz == last) << 4
                            | u8::from(cz == 0) << 5;
                        let mut visit = |nx: usize, ny: usize, nz: usize| {
                            let i = idx(nx, ny, nz);
                            if !visited[i] && !is_opaque(at(nx, ny, nz)) {
                                visited[i] = true;
                                stack.push((nx as u8, ny as u8, nz as u8));
                            }
                        };
                        if cx + 1 < CS {
                            visit(cx + 1, cy, cz);
                        }
                        if cx > 0 {
                            visit(cx - 1, cy, cz);
                        }
                        if cy + 1 < CS {
                            visit(cx, cy + 1, cz);
                        }
                        if cy > 0 {
                            visit(cx, cy - 1, cz);
                        }
                        if cz + 1 < CS {
                            visit(cx, cy, cz + 1);
                        }
                        if cz > 0 {
                            visit(cx, cy, cz - 1);
                        }
                    }
                    for a in 0..FACES {
                        for b in 0..FACES {
                            if touched & (1 << a) != 0 && touched & (1 << b) != 0 {
                                result.connect(a, b);
                            }
                        }
                    }
                }
            }
        }
        if open_cells == CS * CS * CS {
            return Self::ALL;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::P;

    fn voxels(f: impl Fn(usize, usize, usize) -> bool) -> Box<[u16; P3]> {
        let mut v = Box::new([0u16; P3]);
        for y in 0..P {
            for z in 0..P {
                for x in 0..P {
                    if x >= 1
                        && y >= 1
                        && z >= 1
                        && x <= CS
                        && y <= CS
                        && z <= CS
                        && f(x - 1, y - 1, z - 1)
                    {
                        v[pad_index(x, y, z)] = 1;
                    }
                }
            }
        }
        v
    }

    #[test]
    fn empty_solid_and_tunnel() {
        let opaque = |b: u16| b == 1;
        assert_eq!(
            ChunkVisibility::from_voxels(&voxels(|_, _, _| false), opaque),
            ChunkVisibility::ALL
        );
        assert_eq!(
            ChunkVisibility::from_voxels(&voxels(|_, _, _| true), opaque),
            ChunkVisibility::NONE
        );
        // A straight tunnel along x through solid rock: only +X and −X see each other.
        let tunnel = ChunkVisibility::from_voxels(&voxels(|_, y, z| !(y == 10 && z == 10)), opaque);
        assert!(tunnel.connected(2, 3) && tunnel.connected(3, 2));
        assert!(!tunnel.connected(0, 1) && !tunnel.connected(2, 0) && !tunnel.connected(4, 5));
        // A floor at y = 16: the upper air touches +Y and the sides, the lower too, but not each other.
        let floor = ChunkVisibility::from_voxels(&voxels(|_, y, _| y == 16), opaque);
        assert!(floor.connected(0, 2) && floor.connected(1, 2));
        assert!(!floor.connected(0, 1));
        assert_eq!(opposite(4), 5);
    }
}
