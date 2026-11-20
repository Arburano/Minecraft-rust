//! Deterministic hashing for state fingerprints and fixture schedules.
//!
//! FNV-1a: no process-random seed (unlike `std` hashers), so the same input
//! gives the same hash in every run, as the repeatability criterion requires.

/// FNV-1a offset basis.
const BASIS: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a prime.
const PRIME: u64 = 0x0000_0100_0000_01b3;

/// Accumulates a deterministic FNV-1a fingerprint of simulation state.
#[derive(Clone, Debug)]
pub struct StateHasher(u64);

impl Default for StateHasher {
    fn default() -> Self {
        Self::new()
    }
}

impl StateHasher {
    /// A fresh hasher.
    #[must_use]
    pub fn new() -> Self {
        Self(BASIS)
    }

    /// Mixes raw bytes.
    pub fn bytes(&mut self, data: &[u8]) {
        for &b in data {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(PRIME);
        }
    }

    /// Mixes a `u64` (little-endian bytes).
    pub fn u64(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }

    /// Mixes an `i32`.
    pub fn i32(&mut self, v: i32) {
        self.u64(v as u64);
    }

    /// Mixes an `f32` by its bit pattern (no floating-point normalization).
    pub fn f32(&mut self, v: f32) {
        self.u64(u64::from(v.to_bits()));
    }

    /// The fingerprint so far.
    #[must_use]
    pub fn finish(&self) -> u64 {
        self.0
    }
}

/// One-shot FNV-1a over two `u64` values; used by the deterministic fixture.
#[must_use]
pub fn fnv_mix(a: u64, b: u64) -> u64 {
    let mut h = StateHasher::new();
    h.u64(a);
    h.u64(b);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_stable_and_order_sensitive() {
        let mut a = StateHasher::new();
        a.u64(1);
        a.u64(2);
        let mut b = StateHasher::new();
        b.u64(1);
        b.u64(2);
        assert_eq!(a.finish(), b.finish());
        let mut c = StateHasher::new();
        c.u64(2);
        c.u64(1);
        assert_ne!(a.finish(), c.finish());
    }

    #[test]
    fn float_bits_distinguish_signed_zero() {
        let mut a = StateHasher::new();
        a.f32(0.0);
        let mut b = StateHasher::new();
        b.f32(-0.0);
        assert_ne!(a.finish(), b.finish());
    }
}
