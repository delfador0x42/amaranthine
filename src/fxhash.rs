//! FxHash — fast, non-cryptographic hasher for trusted internal data.
//! ~3ns/hash vs SipHash ~20ns/hash. 7x speedup on every HashMap/HashSet op.
//! NOT DoS-resistant — only use for internal data structures, never for external input.

use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};

const SEED: u64 = 0x517cc1b727220a95;

pub struct FxHasher {
    hash: u64,
}

impl Default for FxHasher {
    #[inline]
    fn default() -> Self { Self { hash: 0 } }
}

impl Hasher for FxHasher {
    #[inline]
    fn finish(&self) -> u64 { self.hash }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.hash = (self.hash.rotate_left(5) ^ b as u64).wrapping_mul(SEED);
        }
    }

    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.hash = (self.hash.rotate_left(5) ^ i as u64).wrapping_mul(SEED);
    }

    #[inline]
    fn write_u16(&mut self, i: u16) {
        self.hash = (self.hash.rotate_left(5) ^ i as u64).wrapping_mul(SEED);
    }

    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.hash = (self.hash.rotate_left(5) ^ i as u64).wrapping_mul(SEED);
    }

    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.hash = (self.hash.rotate_left(5) ^ i).wrapping_mul(SEED);
    }

    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.hash = (self.hash.rotate_left(5) ^ i as u64).wrapping_mul(SEED);
    }
}

pub type FxBuildHasher = BuildHasherDefault<FxHasher>;
pub type FxHashMap<K, V> = HashMap<K, V, FxBuildHasher>;
pub type FxHashSet<T> = HashSet<T, FxBuildHasher>;

/// Create a FxHashMap with specified capacity.
#[inline]
pub fn map_with_capacity<K, V>(cap: usize) -> FxHashMap<K, V> {
    HashMap::with_capacity_and_hasher(cap, FxBuildHasher::default())
}

/// Create a FxHashSet with specified capacity.
#[inline]
pub fn set_with_capacity<T>(cap: usize) -> FxHashSet<T> {
    HashSet::with_capacity_and_hasher(cap, FxBuildHasher::default())
}
