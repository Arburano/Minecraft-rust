//! CPU culling (`meshing-and-rendering.md`, "Culling"): frustum by AABB,
//! face directions per render batch, cave culling by mesh chunk visibility.

use std::collections::{HashMap, VecDeque};

use glam::{IVec3, Mat4, Vec3, Vec4};
use mesh::visibility::{ChunkVisibility, FACE_OFFSETS, FACES, opposite};

/// Six planes `ax + by + cz + d >= 0` inside.
#[derive(Clone, Copy, Debug)]
pub struct Frustum {
    planes: [Vec4; 6],
}

impl Frustum {
    /// Planes of a view-projection matrix with depth in `0..1` (wgpu).
    #[must_use]
    pub fn from_view_proj(m: Mat4) -> Self {
        let r = |i: usize| m.row(i);
        let planes = [
            r(3) + r(0),
            r(3) - r(0),
            r(3) + r(1),
            r(3) - r(1),
            r(2),
            r(3) - r(2),
        ]
        .map(|p| {
            let n = p.truncate().length();
            if n > 0.0 { p / n } else { p }
        });
        Self { planes }
    }

    /// Whether an axis-aligned box may be visible (conservative).
    #[must_use]
    pub fn intersects(&self, min: Vec3, max: Vec3) -> bool {
        self.planes.iter().all(|p| {
            // The box corner farthest along the plane normal.
            let v = Vec3::new(
                if p.x >= 0.0 { max.x } else { min.x },
                if p.y >= 0.0 { max.y } else { min.y },
                if p.z >= 0.0 { max.z } else { min.z },
            );
            p.truncate().dot(v) + p.w >= 0.0
        })
    }
}

/// Bit `d` set when quads of face direction `d` (mesh order: Up, Down, Right
/// (+X), Left (−X), Front (+Z), Back (−Z)) in a box can face the camera.
/// At most... all six when the camera is inside the box; usually three.
#[must_use]
pub fn visible_face_mask(camera: Vec3, min: Vec3, max: Vec3) -> u8 {
    let mut m = 0u8;
    // A +Y face at plane y is visible from above it: possible if camera.y > min.y.
    m |= u8::from(camera.y > min.y);
    m |= u8::from(camera.y < max.y) << 1;
    m |= u8::from(camera.x > min.x) << 2;
    m |= u8::from(camera.x < max.x) << 3;
    m |= u8::from(camera.z > min.z) << 4;
    m |= u8::from(camera.z < max.z) << 5;
    m
}

/// Mesh chunks reachable from the camera's mesh chunk through connected faces
/// (cave culling), also inside the frustum and within `radius` mesh chunks.
/// Chunks without a visibility entry count as fully open.
#[must_use]
pub fn cave_visible(
    camera_chunk: IVec3,
    visibility: &HashMap<IVec3, ChunkVisibility>,
    frustum: Option<&Frustum>,
    radius: i32,
    chunk_size: f32,
) -> Vec<IVec3> {
    let mut out = Vec::new();
    let mut seen: HashMap<IVec3, ()> = HashMap::new();
    // (chunk, entry face or 6 for the start, directions used so far)
    let mut queue: VecDeque<(IVec3, usize, u8)> = VecDeque::new();
    queue.push_back((camera_chunk, FACES, 0));
    seen.insert(camera_chunk, ());
    while let Some((c, entry, used)) = queue.pop_front() {
        out.push(c);
        let vis = visibility.get(&c).copied().unwrap_or(ChunkVisibility::ALL);
        for (exit, off) in FACE_OFFSETS.iter().enumerate() {
            // Never step back against a direction already taken (a path from
            // the camera only goes "outwards").
            if used & (1 << opposite(exit)) != 0 {
                continue;
            }
            if entry < FACES && !vis.connected(entry, exit) {
                continue;
            }
            let n = c + IVec3::from_array(*off);
            if (n - camera_chunk).abs().max_element() > radius || seen.contains_key(&n) {
                continue;
            }
            if let Some(f) = frustum {
                let min = n.as_vec3() * chunk_size;
                if !f.intersects(min, min + Vec3::splat(chunk_size)) {
                    continue;
                }
            }
            seen.insert(n, ());
            queue.push_back((n, opposite(exit), used | (1 << exit)));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frustum_keeps_front_and_drops_behind() {
        let proj = crate::terrain::camera::perspective(1.2, 1.5, 0.1, 500.0);
        let view = crate::terrain::camera::look_to(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y);
        let f = Frustum::from_view_proj(proj * view);
        assert!(f.intersects(Vec3::new(-1.0, -1.0, -20.0), Vec3::new(1.0, 1.0, -10.0)));
        assert!(
            !f.intersects(Vec3::new(-1.0, -1.0, 10.0), Vec3::new(1.0, 1.0, 20.0)),
            "behind"
        );
        assert!(
            !f.intersects(Vec3::new(-1.0, -1.0, -900.0), Vec3::new(1.0, 1.0, -800.0)),
            "past far"
        );
        assert!(
            !f.intersects(Vec3::new(200.0, -1.0, -20.0), Vec3::new(210.0, 1.0, -10.0)),
            "far right"
        );
    }

    #[test]
    fn three_face_directions_from_outside() {
        let m = visible_face_mask(
            Vec3::new(100.0, 100.0, 100.0),
            Vec3::ZERO,
            Vec3::splat(64.0),
        );
        assert_eq!(m, 0b010101, "Up, Right, Front only");
        let inside = visible_face_mask(Vec3::splat(32.0), Vec3::ZERO, Vec3::splat(64.0));
        assert_eq!(inside, 0b111111);
    }

    #[test]
    fn sealed_chunks_stop_the_traversal() {
        let mut vis = HashMap::new();
        // A wall of solid chunks at x = 2 between the camera and x >= 3.
        for y in -6..=6 {
            for z in -6..=6 {
                vis.insert(IVec3::new(2, y, z), ChunkVisibility::NONE);
            }
        }
        let seen = cave_visible(IVec3::ZERO, &vis, None, 6, 32.0);
        assert!(seen.contains(&IVec3::new(1, 0, 0)));
        assert!(
            seen.contains(&IVec3::new(2, 0, 0)),
            "the wall itself is drawn"
        );
        assert!(!seen.iter().any(|c| c.x >= 3), "nothing behind the wall");
        let open = cave_visible(IVec3::ZERO, &HashMap::new(), None, 2, 32.0);
        assert_eq!(open.len(), 125, "everything within radius 2 when open");
    }
}
