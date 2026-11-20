//! Compact trigram fingerprint for SIMD-accelerated intersection scoring.
//!
//! Uses a 256-bit bloom filter (`4 x u64`) with 2 hash positions per trigram.
//! Approximate intersection via bitwise AND + `popcount` runs in ~8 CPU cycles
//! vs ~1000+ cycles for `HashSet` intersection on 30 trigrams.
//!
//! False positive rate: ~4% at 30 trigrams (`(1 - e^(-2·30/256))^2`),
//! acceptable for ranking; exact verification follows for final results.

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

    /// Approximate Jaccard similarity using bloom-filter Jaccard estimator.
    ///
    /// Computes `popcount(A AND B) / popcount(A OR B)` (Broder 1997).
    /// This estimates set Jaccard directly from the bit-level Jaccard of
    /// the bloom filters, without needing exact trigram counts.
    ///
    /// The `_self_count` and `_other_count` parameters are reserved for
    /// future bias-correction and kept for API stability.
    ///
    /// Returns 0.0 when both fingerprints are empty.
    #[must_use]
    pub fn approx_jaccard(&self, other: &Self, _self_count: usize, _other_count: usize) -> f32 {
        let and_pop = self.approx_intersection_count(other);
        let or_pop = self.union_popcount(other);
        if or_pop == 0 {
            return 0.0;
        }
        and_pop as f32 / or_pop as f32
    }

    /// Popcount of `self OR other` (used for bloom Jaccard denominator).
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
