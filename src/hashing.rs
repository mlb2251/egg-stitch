//! Small deterministic hashing primitives built on `FxHasher`, which — unlike
//! the std default's randomised keys — is stable across runs, so every hash here
//! is reproducible. The dedup machinery relies on that determinism.

use rustc_hash::FxHasher;
use std::hash::{Hash, Hasher};

/// The two halves of [`h128`] are derived with these independent salts (the
/// golden-ratio and xxHash mixing primes).
const H128_SALT_HI: u64 = 0x9E37_79B9_7F4A_7C15;
const H128_SALT_LO: u64 = 0x85EB_CA6B_C2B2_AE35;

/// SplitMix64 finaliser multipliers.
const SPLITMIX_MUL_1: u64 = 0xBF58_476D_1CE4_E5B9;
const SPLITMIX_MUL_2: u64 = 0x94D0_49BB_1331_11EB;

/// SplitMix64 finaliser: a cheap full-avalanche mix of a single `u64`.
pub fn mix64(mut x: u64) -> u64 {
    x = (x ^ (x >> 30)).wrapping_mul(SPLITMIX_MUL_1);
    x = (x ^ (x >> 27)).wrapping_mul(SPLITMIX_MUL_2);
    x ^ (x >> 31)
}

/// Deterministic 64-bit hash of `val`, domain-separated by `salt` (use a distinct
/// salt at each call site so unrelated values can't alias).
pub fn h64<T: Hash + ?Sized>(salt: u64, val: &T) -> u64 {
    let mut h = FxHasher::default();
    salt.hash(&mut h);
    val.hash(&mut h);
    h.finish()
}

/// Deterministic 128-bit hash: two independently-salted 64-bit hashes
/// concatenated, so accidental collisions stay negligible over millions of items.
pub fn h128<T: Hash + ?Sized>(val: &T) -> u128 {
    ((h64(H128_SALT_HI, val) as u128) << 64) | h64(H128_SALT_LO, val) as u128
}
