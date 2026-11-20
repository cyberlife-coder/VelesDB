//! Compact trigram fingerprint for SIMD-accelerated intersection scoring.
//!
//! Uses a 256-bit bloom filter (`4 x u64`) with 2 hash positions per trigram.
//! Approximate intersection via bitwise AND + `popcount` runs in ~8 CPU cycles
//! vs ~1000+ cycles for `HashSet` intersection on 30 trigrams.
//!
//! False positive rate: ~4% at 30 trigrams (`(1 - e^(-2·30/256))^2`).
//! [`TrigramFingerprint::approx_jaccard`] corrects for it with the exact set
//! sizes; LIKE ranking (`TrigramIndex::search_like_ranked`) needs no
//! fingerprint, since a candidate's Jaccard is exact from counts alone.

#![allow(clippy::cast_possible_truncation)] // Bloom positions are always < 256
#![allow(clippy::cast_precision_loss)] // Precision loss acceptable for Jaccard scoring

use std::collections::HashSet;

/// A 256-bit bloom filter for fast approximate trigram intersection.
///
/// Each trigram maps to 2 bit positions of one well-mixed 32-bit hash.
/// Intersection is computed as `(self AND other).popcount()` — four
/// `AND` + four `POPCNT` instructions (~8 cycles on modern x86).
///
/// # Approximation
///
/// This is a **lossy** representation. `approx_intersection_count` may
/// overcount (false positives from hash collisions), but never undercounts.
/// Use exact `score_jaccard` on `TrigramIndex` when precision matters.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Hash)]
pub struct TrigramFingerprint {
    bits: [u64; 4],
}

/// Compute two independent bit positions for a 3-byte trigram.
///
/// The packed trigram goes through the `MurmurHash3` 32-bit finalizer
/// (`fmix32`), whose full avalanche makes every output bit depend on every
/// input byte; the two positions are two disjoint bytes of that hash.
///
/// Reducing the raw product `v * C` modulo 256 instead — as this function
/// once did — keeps its low byte, which depends on `trigram[0]` alone: every
/// trigram sharing a first byte then set the same two bits, and
/// `approx_jaccard` overestimated long documents several-fold.
#[inline]
fn trigram_hash(trigram: [u8; 3]) -> (usize, usize) {
    let h = fmix32(u32::from_le_bytes([trigram[0], trigram[1], trigram[2], 0]));
    ((h & 0xFF) as usize, ((h >> 8) & 0xFF) as usize)
}

/// `MurmurHash3` 32-bit finalizer: a bijective mixer with full avalanche.
#[inline]
const fn fmix32(mut h: u32) -> u32 {
    h ^= h >> 16;
    h = h.wrapping_mul(0x85EB_CA6B);
    h ^= h >> 13;
    h = h.wrapping_mul(0xC2B2_AE35);
    h ^ (h >> 16)
}

/// Width of the fingerprint in bits.
const BITS: u32 = 256;

/// Bits set per inserted trigram (`k`).
const HASHES_PER_TRIGRAM: f64 = 2.0;

/// Expected number of distinct trigrams behind `set_bits` set bits:
/// `-(m/k) ln(1 - X/m)`. Infinite for a saturated fingerprint.
#[inline]
fn estimated_cardinality(set_bits: u32) -> f64 {
    let m = f64::from(BITS);
    -(m / HASHES_PER_TRIGRAM) * (1.0 - f64::from(set_bits) / m).ln()
}

/// Set a single bit in the 4-word bitmap at the given position (0..255).
#[inline]
fn set_bit(bits: &mut [u64; 4], pos: usize) {
    let word = pos / 64;
    let bit = pos % 64;
    bits[word] |= 1u64 << bit;
}

impl TrigramFingerprint {
    /// Build a fingerprint from an existing trigram set.
    ///
    /// Hashes each trigram to 2 positions in the 256-bit bloom filter.
    ///
    /// # Example
    ///
    /// ```
    /// use velesdb_core::index::trigram::fingerprint::TrigramFingerprint;
    /// use std::collections::HashSet;
    ///
    /// let mut set = HashSet::new();
    /// set.insert(*b"hel");
    /// set.insert(*b"ell");
    /// let fp = TrigramFingerprint::from_trigram_set(&set);
    /// assert!(!fp.is_empty());
    /// ```
    #[must_use]
    pub fn from_trigram_set(trigrams: &HashSet<[u8; 3]>) -> Self {
        let mut fp = Self::default();
        for trigram in trigrams {
            fp.insert(trigram);
        }
        fp
    }

    /// Build a fingerprint directly from text.
    ///
    /// Extracts trigrams (with `pg_trgm`-style padding) then builds
    /// the bloom filter in a single pass.
    ///
    /// # Example
    ///
    /// ```
    /// use velesdb_core::index::trigram::fingerprint::TrigramFingerprint;
    ///
    /// let fp = TrigramFingerprint::from_text("hello");
    /// assert!(!fp.is_empty());
    /// ```
    #[must_use]
    pub fn from_text(text: &str) -> Self {
        let trigrams = super::extract_trigrams(text);
        Self::from_trigram_set(&trigrams)
    }

    /// Insert a single trigram into the fingerprint.
    ///
    /// Sets 2 bits in the 256-bit bloom filter corresponding to
    /// the trigram's hash positions.
    #[inline]
    pub fn insert(&mut self, trigram: &[u8; 3]) {
        let (h1, h2) = trigram_hash(*trigram);
        set_bit(&mut self.bits, h1);
        set_bit(&mut self.bits, h2);
    }

    /// Approximate intersection count via bitwise AND + popcount.
    ///
    /// Returns the number of set bits in `self AND other`. This
    /// **overestimates** the true intersection size due to hash
    /// collisions, but is computed in ~8 CPU cycles (4 AND + 4 POPCNT).
    ///
    /// # Example
    ///
    /// ```
    /// use velesdb_core::index::trigram::fingerprint::TrigramFingerprint;
    ///
    /// let a = TrigramFingerprint::from_text("hello world");
    /// let b = TrigramFingerprint::from_text("hello there");
    /// let shared = a.approx_intersection_count(&b);
    /// assert!(shared > 0);
    /// ```
    #[must_use]
    #[inline]
    pub fn approx_intersection_count(&self, other: &Self) -> u32 {
        (self.bits[0] & other.bits[0]).count_ones()
            + (self.bits[1] & other.bits[1]).count_ones()
            + (self.bits[2] & other.bits[2]).count_ones()
            + (self.bits[3] & other.bits[3]).count_ones()
    }

    /// Approximate Jaccard similarity of the two trigram sets.
    ///
    /// `self_count` and `other_count` are the exact sizes of the sets the
    /// fingerprints were built from. Only their intersection is estimated,
    /// from bit counts:
    ///
    /// 1. After `n` trigrams set `k = 2` bits each in `m = 256`, a bit is
    ///    still clear with probability `e^(-kn/m)`, so `X` set bits stand
    ///    for `n̂(X) = -(m/k) ln(1 - X/m)` distinct trigrams.
    /// 2. The OR of two fingerprints is the fingerprint of the union, so
    ///    `n̂(A OR B)` estimates `|A ∪ B|`. It is rescaled by
    ///    `(|A| + |B|) / (n̂(A) + n̂(B))`, which cancels the per-fingerprint
    ///    error the exact counts reveal (identical fingerprints score 1.0).
    /// 3. `|A ∩ B| = |A| + |B| - |A ∪ B|`, clamped to `[0, min(|A|, |B|)]`.
    ///
    /// The plain bit ratio `|A AND B| / |A OR B|` overestimates by ~0.1 on
    /// average: two trigrams of one set sharing a bit shrink `A OR B` but not
    /// the shared bits. This estimator is unbiased on average.
    ///
    /// A saturated union (all 256 bits set, past ~700 distinct trigrams)
    /// carries no cardinality signal; the bit ratio is returned instead.
    ///
    /// Returns 0.0 when either set is empty.
    #[must_use]
    pub fn approx_jaccard(&self, other: &Self, self_count: usize, other_count: usize) -> f32 {
        if self_count == 0 || other_count == 0 || self.is_empty() || other.is_empty() {
            return 0.0;
        }
        let union_bits = self.union_popcount(other);
        if union_bits >= BITS {
            return self.approx_intersection_count(other) as f32 / union_bits as f32;
        }

        let (a, b) = (self_count as f64, other_count as f64);
        let estimated_counts =
            estimated_cardinality(self.popcount()) + estimated_cardinality(other.popcount());
        let union = estimated_cardinality(union_bits) * (a + b) / estimated_counts;
        let intersection = (a + b - union).clamp(0.0, a.min(b));
        (intersection / (a + b - intersection)) as f32
    }

    /// Number of set bits.
    #[inline]
    fn popcount(&self) -> u32 {
        self.bits.iter().map(|word| word.count_ones()).sum()
    }

    /// Popcount of `self OR other`: the bit count of the union's fingerprint.
    #[inline]
    fn union_popcount(&self, other: &Self) -> u32 {
        (self.bits[0] | other.bits[0]).count_ones()
            + (self.bits[1] | other.bits[1]).count_ones()
            + (self.bits[2] | other.bits[2]).count_ones()
            + (self.bits[3] | other.bits[3]).count_ones()
    }

    /// Check whether the fingerprint has no bits set.
    #[must_use]
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.bits[0] == 0 && self.bits[1] == 0 && self.bits[2] == 0 && self.bits[3] == 0
    }
}
