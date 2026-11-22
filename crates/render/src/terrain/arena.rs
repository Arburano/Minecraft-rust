//! Range allocator for the quad buffer: one contiguous range per render batch
//! (its six direction lists back to back). First fit with coalescing; the
//! free list keeps its capacity, so steady-state updates do not allocate.

use std::ops::Range;

/// Allocator over `0..capacity` units (quads).
#[derive(Clone, Debug)]
pub struct RangeArena {
    capacity: u32,
    /// Free ranges sorted by start, never adjacent (coalesced).
    free: Vec<Range<u32>>,
}

impl RangeArena {
    /// An empty arena of `capacity` units.
    #[must_use]
    pub fn new(capacity: u32) -> Self {
        Self {
            capacity,
            free: std::iter::once(0..capacity).collect(),
        }
    }

    /// Total units.
    #[must_use]
    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    /// Free units in total.
    #[must_use]
    pub fn free_units(&self) -> u32 {
        self.free.iter().map(|r| r.end - r.start).sum()
    }

    /// Allocates `len` units (first fit); `None` if no free range is large enough.
    pub fn alloc(&mut self, len: u32) -> Option<Range<u32>> {
        if len == 0 {
            return Some(0..0);
        }
        let i = self.free.iter().position(|r| r.end - r.start >= len)?;
        let start = self.free[i].start;
        self.free[i].start += len;
        if self.free[i].is_empty() {
            self.free.remove(i);
        }
        Some(start..start + len)
    }

    /// Returns a range (must have come from `alloc` and not be freed twice).
    pub fn free(&mut self, r: Range<u32>) {
        if r.is_empty() {
            return;
        }
        let i = self.free.partition_point(|f| f.start < r.start);
        self.free.insert(i, r);
        // Coalesce with the right neighbor, then the left one.
        if i + 1 < self.free.len() && self.free[i].end == self.free[i + 1].start {
            self.free[i].end = self.free[i + 1].end;
            self.free.remove(i + 1);
        }
        if i > 0 && self.free[i - 1].end == self.free[i].start {
            self.free[i - 1].end = self.free[i].end;
            self.free.remove(i);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alloc_free_coalesce() {
        let mut a = RangeArena::new(100);
        let x = a.alloc(30).expect("fits");
        let y = a.alloc(30).expect("fits");
        let z = a.alloc(30).expect("fits");
        assert_eq!((x.clone(), y.clone(), z.clone()), (0..30, 30..60, 60..90));
        assert!(a.alloc(20).is_none());
        a.free(y);
        assert_eq!(a.alloc(25), Some(30..55), "first fit reuses the hole");
        a.free(x);
        a.free(30..55);
        a.free(z);
        assert_eq!(a.free_units(), 100);
        assert_eq!(a.alloc(100), Some(0..100), "fully coalesced");
    }

    #[test]
    fn random_ops_never_overlap() {
        let mut a = RangeArena::new(1000);
        let mut live: Vec<Range<u32>> = Vec::new();
        let mut s = 0x1234_5678u64;
        for _ in 0..5000 {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            if s % 3 != 0 || live.is_empty() {
                if let Some(r) = a.alloc((s % 60) as u32 + 1) {
                    assert!(
                        live.iter().all(|l| r.end <= l.start || l.end <= r.start),
                        "overlap"
                    );
                    live.push(r);
                }
            } else {
                let r = live.swap_remove((s % live.len() as u64) as usize);
                a.free(r);
            }
            let used: u32 = live.iter().map(|r| r.end - r.start).sum();
            assert_eq!(used + a.free_units(), 1000);
        }
    }
}
