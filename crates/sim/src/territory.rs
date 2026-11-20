//! Region territory geometry: a rectangle of sections in the flat prototype
//! layer (y = 0), with the ≥ 1 section buffer of ADR-0008.
//!
//! The buffer is the outer ring of the rectangle: local interactions of the
//! inner area stay inside the region. Entities are owned by the region whose
//! territory contains their section; while walking they may transiently be in
//! the buffer ring (see `docs/architecture/server-regions.md` and the T0-09
//! report for the prototype relaxation). Structural changes keep the buffer:
//! a split is deferred while an entity sits in the two section columns next
//! to the split line.

use world::coords::SectionPos;

/// Minimum side of a region rectangle: 1 inner section + 2 buffer sections.
pub const MIN_SIDE: i32 = 3;

/// Split axis of a region rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    /// Split along x: sections with `x < line` go to the first half.
    X,
    /// Split along z: sections with `z < line` go to the first half.
    Z,
}

/// An inclusive rectangle of section coordinates in the prototype layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SectionRect {
    /// Minimum section x (inclusive).
    pub min_x: i32,
    /// Minimum section z (inclusive).
    pub min_z: i32,
    /// Maximum section x (inclusive).
    pub max_x: i32,
    /// Maximum section z (inclusive).
    pub max_z: i32,
}

impl SectionRect {
    /// A rectangle, or `None` if the bounds are inverted.
    #[must_use]
    pub fn new(min_x: i32, min_z: i32, max_x: i32, max_z: i32) -> Option<Self> {
        if min_x <= max_x && min_z <= max_z {
            Some(Self {
                min_x,
                min_z,
                max_x,
                max_z,
            })
        } else {
            None
        }
    }

    /// Side length along x, in sections.
    #[must_use]
    pub fn width(&self) -> i32 {
        self.max_x - self.min_x + 1
    }

    /// Side length along z, in sections.
    #[must_use]
    pub fn depth(&self) -> i32 {
        self.max_z - self.min_z + 1
    }

    /// Whether the section (x, z) is inside the rectangle.
    #[must_use]
    pub fn contains(&self, x: i32, z: i32) -> bool {
        x >= self.min_x && x <= self.max_x && z >= self.min_z && z <= self.max_z
    }

    /// Whether the section is inside the rectangle (y is the prototype layer).
    #[must_use]
    pub fn contains_pos(&self, p: SectionPos) -> bool {
        self.contains(p.x, p.z)
    }

    /// The rectangle without its buffer ring, or `None` if a side is < 3.
    #[must_use]
    pub fn inner(&self) -> Option<Self> {
        Self::new(
            self.min_x + 1,
            self.min_z + 1,
            self.max_x - 1,
            self.max_z - 1,
        )
    }

    /// Whether the section is at least `buffer` sections inside every edge.
    #[must_use]
    pub fn contains_with_buffer(&self, x: i32, z: i32, buffer: i32) -> bool {
        x >= self.min_x + buffer
            && x <= self.max_x - buffer
            && z >= self.min_z + buffer
            && z <= self.max_z - buffer
    }

    /// If the rectangles share a full edge, the axis they are adjacent on.
    #[must_use]
    pub fn adjacent_axis(&self, other: &Self) -> Option<Axis> {
        let same_z = self.min_z == other.min_z && self.max_z == other.max_z;
        let same_x = self.min_x == other.min_x && self.max_x == other.max_x;
        if same_z && (self.max_x + 1 == other.min_x || other.max_x + 1 == self.min_x) {
            Some(Axis::X)
        } else if same_x && (self.max_z + 1 == other.min_z || other.max_z + 1 == self.min_z) {
            Some(Axis::Z)
        } else {
            None
        }
    }

    /// The bounding rectangle of two adjacent rectangles.
    #[must_use]
    pub fn union(&self, other: &Self) -> Self {
        Self {
            min_x: self.min_x.min(other.min_x),
            min_z: self.min_z.min(other.min_z),
            max_x: self.max_x.max(other.max_x),
            max_z: self.max_z.max(other.max_z),
        }
    }

    /// Split into two rectangles at `line` (coordinate < line goes first).
    /// `None` if the line is outside or a half would be narrower than
    /// [`MIN_SIDE`].
    #[must_use]
    pub fn split(&self, axis: Axis, line: i32) -> Option<(Self, Self)> {
        let (lo, hi) = match axis {
            Axis::X => (self.min_x, self.max_x),
            Axis::Z => (self.min_z, self.max_z),
        };
        if line - lo < MIN_SIDE || hi - line + 1 < MIN_SIDE {
            return None;
        }
        let (first, second) = match axis {
            Axis::X => (
                Self {
                    max_x: line - 1,
                    ..*self
                },
                Self {
                    min_x: line,
                    ..*self
                },
            ),
            Axis::Z => (
                Self {
                    max_z: line - 1,
                    ..*self
                },
                Self {
                    min_z: line,
                    ..*self
                },
            ),
        };
        Some((first, second))
    }

    /// Iterates all section coordinates of the rectangle in the y = 0 layer,
    /// in row-major (x, z) order.
    pub fn sections(&self) -> impl Iterator<Item = SectionPos> {
        let (min_x, max_x, min_z, max_z) = (self.min_x, self.max_x, self.min_z, self.max_z);
        (min_z..=max_z).flat_map(move |z| (min_x..=max_x).map(move |x| SectionPos { x, y: 0, z }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> SectionRect {
        SectionRect::new(0, 0, 7, 7).expect("valid")
    }

    #[test]
    fn inner_drops_the_buffer_ring() {
        let r = rect();
        let inner = r.inner().expect("8x8 has an inner area");
        assert_eq!(inner, SectionRect::new(1, 1, 6, 6).expect("valid"));
        assert!(r.contains_with_buffer(1, 1, 1));
        assert!(!r.contains_with_buffer(0, 3, 1));
        assert_eq!(
            SectionRect::new(0, 0, 2, 2).expect("valid").inner(),
            Some(SectionRect::new(1, 1, 1, 1).expect("valid"))
        );
        assert!(
            SectionRect::new(0, 0, 1, 1)
                .expect("valid")
                .inner()
                .is_none()
        );
    }

    #[test]
    fn adjacency_requires_a_shared_full_edge() {
        let a = SectionRect::new(0, 0, 3, 7).expect("valid");
        let b = SectionRect::new(4, 0, 7, 7).expect("valid");
        assert_eq!(a.adjacent_axis(&b), Some(Axis::X));
        assert_eq!(b.adjacent_axis(&a), Some(Axis::X));
        let shifted = SectionRect::new(4, 1, 7, 7).expect("valid");
        assert_eq!(a.adjacent_axis(&shifted), None);
        assert_eq!(a.union(&b), rect());
    }

    #[test]
    fn split_respects_min_side() {
        let r = rect();
        assert!(r.split(Axis::X, 3).is_some());
        assert!(r.split(Axis::X, 2).is_none());
        assert!(r.split(Axis::X, 6).is_none());
        let (a, b) = r.split(Axis::Z, 4).expect("valid split");
        assert_eq!(a, SectionRect::new(0, 0, 7, 3).expect("valid"));
        assert_eq!(b, SectionRect::new(0, 4, 7, 7).expect("valid"));
        assert_eq!(a.adjacent_axis(&b), Some(Axis::Z));
    }

    #[test]
    fn sections_cover_the_rectangle_once() {
        let r = SectionRect::new(2, 3, 4, 5).expect("valid");
        let all: Vec<_> = r.sections().collect();
        assert_eq!(all.len(), 9);
        assert_eq!(all[0], SectionPos { x: 2, y: 0, z: 3 });
        assert_eq!(all[8], SectionPos { x: 4, y: 0, z: 5 });
    }
}
