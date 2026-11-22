//! Block picking: the first colliding cell along a ray (voxel DDA,
//! Amanatides & Woo), with the face it was entered through.

use glam::{DVec3, IVec3};
use world::coords::BlockPos;

/// A hit block and the cell in front of the hit face (where a block is placed).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayHit {
    /// The hit cell.
    pub block: BlockPos,
    /// Unit normal of the entered face (zero when the ray starts inside).
    pub normal: IVec3,
    /// Distance from the origin to the entry point.
    pub distance: f64,
}

impl RayHit {
    /// The cell adjacent to the hit face.
    #[must_use]
    pub fn place_pos(&self) -> BlockPos {
        BlockPos::new(
            self.block.x + self.normal.x,
            self.block.y + self.normal.y,
            self.block.z + self.normal.z,
        )
    }
}

/// Walks cells along `dir` from `origin` up to `max_dist`; returns the first
/// cell for which `hit` is true.
pub fn raycast(
    origin: DVec3,
    dir: DVec3,
    max_dist: f64,
    hit: impl Fn(BlockPos) -> bool,
) -> Option<RayHit> {
    let len = dir.length();
    if len == 0.0 || !len.is_finite() {
        return None;
    }
    let d = dir / len;
    let mut cell = origin.floor().as_ivec3();
    let to_block = |c: IVec3| BlockPos::new(c.x, c.y, c.z);
    if hit(to_block(cell)) {
        return Some(RayHit {
            block: to_block(cell),
            normal: IVec3::ZERO,
            distance: 0.0,
        });
    }
    let step = IVec3::new(sign(d.x), sign(d.y), sign(d.z));
    let next = |o: f64, c: i32, s: i32, v: f64| {
        if s == 0 {
            f64::INFINITY
        } else {
            let boundary = if s > 0 {
                f64::from(c) + 1.0
            } else {
                f64::from(c)
            };
            (boundary - o) / v
        }
    };
    let delta = |v: f64| {
        if v == 0.0 {
            f64::INFINITY
        } else {
            1.0 / v.abs()
        }
    };
    let mut t_max = DVec3::new(
        next(origin.x, cell.x, step.x, d.x),
        next(origin.y, cell.y, step.y, d.y),
        next(origin.z, cell.z, step.z, d.z),
    );
    let t_delta = DVec3::new(delta(d.x), delta(d.y), delta(d.z));
    loop {
        // The axis whose boundary comes first (ties: x, then y, then z).
        let axis = if t_max.x <= t_max.y && t_max.x <= t_max.z {
            0
        } else if t_max.y <= t_max.z {
            1
        } else {
            2
        };
        let t = t_max[axis];
        if t > max_dist {
            return None;
        }
        cell[axis] += step[axis];
        t_max[axis] += t_delta[axis];
        if hit(to_block(cell)) {
            let mut normal = IVec3::ZERO;
            normal[axis] = -step[axis];
            return Some(RayHit {
                block: to_block(cell),
                normal,
                distance: t,
            });
        }
    }
}

fn sign(v: f64) -> i32 {
    if v > 0.0 {
        1
    } else if v < 0.0 {
        -1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference: march in tiny steps and report the first solid cell.
    fn naive(
        origin: DVec3,
        dir: DVec3,
        max: f64,
        hit: impl Fn(BlockPos) -> bool,
    ) -> Option<BlockPos> {
        let d = dir.normalize();
        let mut t = 0.0;
        while t <= max {
            let p = (origin + d * t).floor();
            let b = BlockPos::new(p.x as i32, p.y as i32, p.z as i32);
            if hit(b) {
                return Some(b);
            }
            t += 1e-4;
        }
        None
    }

    #[test]
    fn hits_the_face_towards_the_eye() {
        let solid = |p: BlockPos| p == BlockPos::new(3, 0, 0);
        let h = raycast(DVec3::new(0.5, 0.5, 0.5), DVec3::X, 5.0, solid).expect("hit");
        assert_eq!(h.block, BlockPos::new(3, 0, 0));
        assert_eq!(h.normal, IVec3::new(-1, 0, 0));
        assert!((h.distance - 2.5).abs() < 1e-12);
        assert_eq!(h.place_pos(), BlockPos::new(2, 0, 0));
        assert!(
            raycast(DVec3::new(0.5, 0.5, 0.5), DVec3::X, 2.0, solid).is_none(),
            "out of reach"
        );
    }

    #[test]
    fn matches_a_naive_march() {
        let solid = |p: BlockPos| {
            (p.x * 7 + p.y * 13 + p.z * 5).rem_euclid(17) == 0 && p != BlockPos::new(0, 1, 0)
        };
        let mut s = 0x9e37_79b9_7f4a_7c15u64;
        let mut rnd = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s % 10_000) as f64 / 10_000.0
        };
        for _ in 0..300 {
            let o = DVec3::new(rnd() + 0.0, rnd() + 1.0, rnd() + 0.0);
            let d = DVec3::new(rnd() - 0.5, rnd() - 0.5, rnd() - 0.5);
            if d.length() < 0.1 {
                continue;
            }
            let fast = raycast(o, d, 6.0, solid).map(|h| h.block);
            let slow = naive(o, d, 6.0, solid);
            assert_eq!(fast, slow, "origin {o} dir {d}");
        }
    }
}
