//! Small wire types and quantization helpers shared by several channels.

use bitcode::{Decode, Encode};
use world::ColumnPos;
use world::coords::BlockPos;

/// Network identity of an entity: stable for its lifetime on the server and
/// the same number as `sim::NetId` (the protocol does not depend on `sim`).
#[derive(Encode, Decode, Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NetId(pub u64);

/// A column position on the wire.
#[derive(Encode, Decode, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ColumnXz {
    /// Column x = block x div 16.
    pub x: i32,
    /// Column z = block z div 16.
    pub z: i32,
}

impl From<ColumnPos> for ColumnXz {
    fn from(p: ColumnPos) -> Self {
        Self { x: p.x, z: p.z }
    }
}

impl From<ColumnXz> for ColumnPos {
    fn from(p: ColumnXz) -> Self {
        Self { x: p.x, z: p.z }
    }
}

/// A block position on the wire.
#[derive(Encode, Decode, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct BlockXyz {
    /// East-west.
    pub x: i32,
    /// Up-down.
    pub y: i32,
    /// North-south.
    pub z: i32,
}

impl From<BlockPos> for BlockXyz {
    fn from(p: BlockPos) -> Self {
        Self {
            x: p.x,
            y: p.y,
            z: p.z,
        }
    }
}

impl From<BlockXyz> for BlockPos {
    fn from(p: BlockXyz) -> Self {
        Self {
            x: p.x,
            y: p.y,
            z: p.z,
        }
    }
}

/// A position quantized to 1/32 block relative to an origin block
/// (`networking-protocol.md`, "Entity deltas"). Range ±1024 blocks on each
/// axis — 64 chunks, more than any view distance.
#[derive(Encode, Decode, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct RelPos {
    /// x − origin x, in 1/32 block.
    pub x: i16,
    /// y − origin y, in 1/32 block.
    pub y: i16,
    /// z − origin z, in 1/32 block.
    pub z: i16,
}

impl RelPos {
    /// Units per block.
    pub const SCALE: f64 = 32.0;

    /// Quantizes a world position (rounding to the nearest 1/32 block), or
    /// `None` if it is farther than the range from `origin` or not finite.
    #[must_use]
    pub fn quantize(pos: [f64; 3], origin: BlockXyz) -> Option<Self> {
        let q = |v: f64, o: i32| -> Option<i16> {
            let units = ((v - f64::from(o)) * Self::SCALE).round();
            (units.is_finite() && units >= f64::from(i16::MIN) && units <= f64::from(i16::MAX))
                .then_some(units as i16)
        };
        Some(Self {
            x: q(pos[0], origin.x)?,
            y: q(pos[1], origin.y)?,
            z: q(pos[2], origin.z)?,
        })
    }

    /// The world position this quantized value stands for.
    #[must_use]
    pub fn to_world(self, origin: BlockXyz) -> [f64; 3] {
        let d = |u: i16, o: i32| f64::from(o) + f64::from(u) / Self::SCALE;
        [
            d(self.x, origin.x),
            d(self.y, origin.y),
            d(self.z, origin.z),
        ]
    }
}

/// An angle as a fraction of a full turn in 8 bits (1.41° steps), for entity
/// orientation.
#[derive(Encode, Decode, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Angle8(pub u8);

/// An angle as a fraction of a full turn in 16 bits (0.0055° steps), for the
/// player's own view in inputs, where 8 bits would be visible when aiming.
#[derive(Encode, Decode, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Angle16(pub u16);

fn turns(radians: f32) -> f64 {
    let t = f64::from(radians) / std::f64::consts::TAU;
    if t.is_finite() { t - t.floor() } else { 0.0 }
}

impl Angle8 {
    /// Quantizes radians (any value; wrapped to one turn, rounded to nearest;
    /// NaN becomes 0).
    #[must_use]
    pub fn from_radians(radians: f32) -> Self {
        Self(((turns(radians) * 256.0).round() as u32 % 256) as u8)
    }

    /// Radians in `0..2π`.
    #[must_use]
    pub fn to_radians(self) -> f32 {
        (f64::from(self.0) / 256.0 * std::f64::consts::TAU) as f32
    }
}

impl Angle16 {
    /// Quantizes radians (any value; wrapped to one turn, rounded to nearest;
    /// NaN becomes 0).
    #[must_use]
    pub fn from_radians(radians: f32) -> Self {
        Self(((turns(radians) * 65536.0).round() as u32 % 65536) as u16)
    }

    /// Radians in `0..2π`.
    #[must_use]
    pub fn to_radians(self) -> f32 {
        (f64::from(self.0) / 65536.0 * std::f64::consts::TAU) as f32
    }

    /// Radians in `-π..π` — the form for pitch.
    #[must_use]
    pub fn to_signed_radians(self) -> f32 {
        (f64::from(self.0 as i16) / 65536.0 * std::f64::consts::TAU) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::{FRAC_PI_2, PI, TAU};

    fn angle_diff(a: f32, b: f32) -> f32 {
        let d = (a - b).rem_euclid(TAU);
        d.min(TAU - d)
    }

    #[test]
    fn rel_pos_round_trip_within_half_unit() {
        let origin = BlockXyz {
            x: -100,
            y: 64,
            z: 2_000_000,
        };
        for p in [
            [-100.0, 64.0, 2_000_000.0],
            [-99.97, 70.51, 2_000_123.3],
            [-1123.9, -60.0, 1_999_000.2],
            [923.96, 1087.0, 2_001_023.9],
        ] {
            let q = RelPos::quantize(p, origin).expect("in range");
            let back = q.to_world(origin);
            for i in 0..3 {
                assert!(
                    (back[i] - p[i]).abs() <= 0.5 / RelPos::SCALE + 1e-9,
                    "{p:?}"
                );
            }
        }
    }

    #[test]
    fn rel_pos_rejects_out_of_range_and_nan() {
        let o = BlockXyz::default();
        assert!(RelPos::quantize([1024.5, 0.0, 0.0], o).is_none());
        assert!(RelPos::quantize([0.0, -1024.1, 0.0], o).is_none());
        assert!(RelPos::quantize([f64::NAN, 0.0, 0.0], o).is_none());
        assert!(RelPos::quantize([1023.9, -1024.0, 0.0], o).is_some());
    }

    #[test]
    fn angles_wrap_and_round() {
        for r in [0.0, 0.3, FRAC_PI_2, PI, 3.9, -0.7, -FRAC_PI_2, 7.5, -20.0] {
            let a8 = Angle8::from_radians(r).to_radians();
            assert!(angle_diff(a8, r) <= TAU / 512.0 + 1e-5, "{r}");
            let a16 = Angle16::from_radians(r).to_radians();
            assert!(angle_diff(a16, r) <= TAU / 131_072.0 + 1e-5, "{r}");
        }
        assert_eq!(Angle8::from_radians(TAU - 0.001), Angle8(0));
        assert_eq!(Angle8::from_radians(f32::NAN), Angle8(0));
        let pitch = Angle16::from_radians(-FRAC_PI_2).to_signed_radians();
        assert!((pitch + FRAC_PI_2).abs() < 1e-4);
    }
}
