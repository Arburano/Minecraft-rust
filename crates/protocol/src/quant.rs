//! Quantized positions and angles (networking-protocol.md, "Entity deltas").
//!
//! - Positions: 1/32 block fixed point. Absolute positions are `i32` per axis
//!   (±67 million blocks); deltas against the acknowledged snapshot are `i16`
//!   (±1,024 blocks), and anything farther is sent absolute.
//! - Angles: 8 bits for replicated entities (1.4°), 16 bits for the player's
//!   own view in input (0.0055°), so aiming stays precise.
//!
//! Quantization rounds to the nearest step; dequantize ∘ quantize is within
//! half a step of the input.

/// Fixed-point steps per block.
pub const POS_STEPS_PER_BLOCK: f64 = 32.0;

/// An absolute position in 1/32 block.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, bitcode::Encode, bitcode::Decode)]
pub struct FixedPos {
    /// X in 1/32 block.
    pub x: i32,
    /// Y in 1/32 block.
    pub y: i32,
    /// Z in 1/32 block.
    pub z: i32,
}

fn quantize_axis(v: f64) -> i32 {
    // `as` saturates and maps NaN to 0, so garbage in never panics.
    (v * POS_STEPS_PER_BLOCK).round() as i32
}

impl FixedPos {
    /// Quantizes a position in blocks.
    #[must_use]
    pub fn from_blocks(p: [f64; 3]) -> Self {
        Self {
            x: quantize_axis(p[0]),
            y: quantize_axis(p[1]),
            z: quantize_axis(p[2]),
        }
    }

    /// The position in blocks.
    #[must_use]
    pub fn to_blocks(self) -> [f64; 3] {
        [
            f64::from(self.x) / POS_STEPS_PER_BLOCK,
            f64::from(self.y) / POS_STEPS_PER_BLOCK,
            f64::from(self.z) / POS_STEPS_PER_BLOCK,
        ]
    }

    /// `self − base` as a delta, or `None` if an axis does not fit `i16`.
    #[must_use]
    pub fn delta_from(self, base: Self) -> Option<PosDelta> {
        let d = |a: i32, b: i32| i16::try_from(i64::from(a) - i64::from(b)).ok();
        Some(PosDelta {
            dx: d(self.x, base.x)?,
            dy: d(self.y, base.y)?,
            dz: d(self.z, base.z)?,
        })
    }

    /// `self + delta` (wrapping, so a garbage delta cannot overflow-panic).
    #[must_use]
    pub fn apply(self, d: PosDelta) -> Self {
        Self {
            x: self.x.wrapping_add(i32::from(d.dx)),
            y: self.y.wrapping_add(i32::from(d.dy)),
            z: self.z.wrapping_add(i32::from(d.dz)),
        }
    }
}

/// A position change in 1/32 block relative to the acknowledged snapshot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, bitcode::Encode, bitcode::Decode)]
pub struct PosDelta {
    /// X change in 1/32 block.
    pub dx: i16,
    /// Y change in 1/32 block.
    pub dy: i16,
    /// Z change in 1/32 block.
    pub dz: i16,
}

/// An angle in 1/256 turn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, bitcode::Encode, bitcode::Decode)]
pub struct Angle8(pub u8);

/// An angle in 1/65,536 turn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, bitcode::Encode, bitcode::Decode)]
pub struct Angle16(pub u16);

fn turns(radians: f64) -> f64 {
    let t = radians / std::f64::consts::TAU;
    t - t.floor()
}

impl Angle8 {
    /// Quantizes radians (any range; wraps to one turn).
    #[must_use]
    pub fn from_radians(r: f64) -> Self {
        // `as u32` saturates and maps NaN to 0; `% 256` wraps 1.0 turn to 0.
        Self(((turns(r) * 256.0).round() as u32 % 256) as u8)
    }

    /// The angle in radians, in `[0, 2π)`.
    #[must_use]
    pub fn to_radians(self) -> f64 {
        f64::from(self.0) / 256.0 * std::f64::consts::TAU
    }
}

impl Angle16 {
    /// Quantizes radians (any range; wraps to one turn).
    #[must_use]
    pub fn from_radians(r: f64) -> Self {
        Self(((turns(r) * 65536.0).round() as u32 % 65536) as u16)
    }

    /// The angle in radians, in `[0, 2π)`.
    #[must_use]
    pub fn to_radians(self) -> f64 {
        f64::from(self.0) / 65536.0 * std::f64::consts::TAU
    }

    /// The angle in radians, in `[−π, π)`, for pitch.
    #[must_use]
    pub fn to_signed_radians(self) -> f64 {
        f64::from(self.0 as i16) / 65536.0 * std::f64::consts::TAU
    }
}

/// Quantizes an analog axis in `[−1, 1]` to `i8` (`−127..=127`).
#[must_use]
pub fn axis_to_i8(v: f64) -> i8 {
    (v.clamp(-1.0, 1.0) * 127.0).round() as i8
}

/// The analog axis of [`axis_to_i8`]; `−128` reads as `−1`.
#[must_use]
pub fn axis_from_i8(v: i8) -> f64 {
    (f64::from(v) / 127.0).max(-1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::f64::consts::{PI, TAU};

    fn angle_dist(a: f64, b: f64) -> f64 {
        let d = (a - b).rem_euclid(TAU);
        d.min(TAU - d)
    }

    #[test]
    fn known_values() {
        assert_eq!(
            FixedPos::from_blocks([1.0, -0.5, 100.03125]),
            FixedPos {
                x: 32,
                y: -16,
                z: 3201
            }
        );
        assert_eq!(Angle8::from_radians(0.0), Angle8(0));
        assert_eq!(Angle8::from_radians(PI), Angle8(128));
        assert_eq!(Angle8::from_radians(-PI / 2.0), Angle8(192));
        assert_eq!(Angle8::from_radians(TAU - 1e-9), Angle8(0));
        assert_eq!(Angle16::from_radians(PI / 2.0), Angle16(16384));
        assert!((Angle16::from_radians(-0.5).to_signed_radians() + 0.5).abs() < 1e-4);
        assert_eq!(axis_to_i8(1.0), 127);
        assert_eq!(axis_to_i8(-3.0), -127);
        assert_eq!(axis_from_i8(-128), -1.0);
        // Garbage in never panics.
        let _ = FixedPos::from_blocks([f64::NAN, f64::INFINITY, -1e300]);
        let _ = Angle8::from_radians(f64::NAN);
        let _ = Angle16::from_radians(f64::INFINITY);
    }

    #[test]
    fn delta_range() {
        let a = FixedPos { x: 0, y: 0, z: 0 };
        let near = FixedPos {
            x: 32767,
            y: -32768,
            z: 5,
        };
        let d = near.delta_from(a).expect("fits");
        assert_eq!(a.apply(d), near);
        let far = FixedPos {
            x: 32768,
            y: 0,
            z: 0,
        };
        assert_eq!(far.delta_from(a), None);
        assert_eq!(
            FixedPos {
                x: i32::MAX,
                y: 0,
                z: 0
            }
            .delta_from(FixedPos {
                x: i32::MIN,
                y: 0,
                z: 0
            }),
            None
        );
    }

    proptest! {
        #[test]
        fn position_error_is_half_a_step(x in -1.0e6f64..1.0e6, y in -1.0e4f64..1.0e4, z in -1.0e6f64..1.0e6) {
            let back = FixedPos::from_blocks([x, y, z]).to_blocks();
            for (a, b) in [x, y, z].iter().zip(back) {
                prop_assert!((a - b).abs() <= 0.5 / POS_STEPS_PER_BLOCK + 1e-9);
            }
        }

        #[test]
        fn angle_error_is_half_a_step(r in -20.0f64..20.0) {
            prop_assert!(angle_dist(Angle8::from_radians(r).to_radians(), r) <= TAU / 512.0 + 1e-9);
            prop_assert!(angle_dist(Angle16::from_radians(r).to_radians(), r) <= TAU / 131072.0 + 1e-9);
        }

        #[test]
        fn delta_roundtrip(ax in any::<i32>(), bx in -40000i32..40000, ay in any::<i32>(), az in any::<i32>()) {
            let base = FixedPos { x: ax, y: ay, z: az };
            let moved = FixedPos { x: ax.wrapping_add(bx), y: ay, z: az };
            match moved.delta_from(base) {
                Some(d) => prop_assert_eq!(base.apply(d), moved),
                None => prop_assert!(i64::from(moved.x) - i64::from(base.x) > i64::from(i16::MAX)
                    || i64::from(moved.x) - i64::from(base.x) < i64::from(i16::MIN)),
            }
        }
    }
}
