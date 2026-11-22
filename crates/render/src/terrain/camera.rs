//! Camera matrices (right-handed, wgpu depth `0..1`). The workspace glam has
//! no projection helpers, so they live here.

use glam::{Mat4, Vec3, Vec4};

/// Perspective projection: vertical field of view in radians, depth `0..1`.
#[must_use]
pub fn perspective(fov_y: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    let f = 1.0 / (fov_y * 0.5).tan();
    let r = far / (near - far);
    Mat4::from_cols(
        Vec4::new(f / aspect, 0.0, 0.0, 0.0),
        Vec4::new(0.0, f, 0.0, 0.0),
        Vec4::new(0.0, 0.0, r, -1.0),
        Vec4::new(0.0, 0.0, r * near, 0.0),
    )
}

/// View matrix looking along `dir` from `eye`.
#[must_use]
pub fn look_to(eye: Vec3, dir: Vec3, up: Vec3) -> Mat4 {
    let f = dir.normalize();
    let s = f.cross(up).normalize();
    let u = s.cross(f);
    Mat4::from_cols(
        Vec4::new(s.x, u.x, -f.x, 0.0),
        Vec4::new(s.y, u.y, -f.y, 0.0),
        Vec4::new(s.z, u.z, -f.z, 0.0),
        Vec4::new(-s.dot(eye), -u.dot(eye), f.dot(eye), 1.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_ahead_maps_into_clip_volume() {
        let m = perspective(1.2, 1.5, 0.1, 100.0) * look_to(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y);
        let near = m * Vec4::new(0.0, 0.0, -0.1, 1.0);
        let far = m * Vec4::new(0.0, 0.0, -100.0, 1.0);
        assert!((near.z / near.w).abs() < 1e-5);
        assert!((far.z / far.w - 1.0).abs() < 1e-5);
    }
}
