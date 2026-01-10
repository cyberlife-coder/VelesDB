//! Bloom Filter for existence checks.
//!
//! Space-efficient probabilistic data structure for fast negative lookups.
//! Based on arXiv:2310.11703v2 recommendations.

use parking_lot::RwLock;
use rustc_hash::FxHasher;
use std::hash::{Hash, Hasher};

/// Bloom filter for probabilistic existence checks.
///
/// Provides O(1) lookups with configurable false positive rate.
/// False negatives are impossible - if `contains()` returns false,
/// the element is definitely not in the set.
pub struct BloomFilter {
    /// Bit array.
    bits: RwLock<Vec<u64>>,
    /// Number of bits (m).
    num_bits: usize,
    /// Number of hash functions (k).
    num_hashes: u32,
    /// Number of items inserted.
    count: RwLock<usize>,
}

impl BloomFilter {
    /// Create a new Bloom filter optimized for the given capacity and FPR.
    ///
    /// # Arguments
    ///
    /// * `capacity` - Expected number of items
    /// * `false_positive_rate` - Target FPR (e.g., 0.01 for 1%)
    #[must_use]
    pub fn new(capacity: usize, false_positive_rate: f64) -> Self {
        // Optimal number of bits: m = -n * ln(p) / (ln(2)^2)
        let num_bits = Self::optimal_bits(capacity, false_positive_rate);
        // Optimal number of hashes: k = (m/n) * ln(2)
        let num_hashes = Self::optimal_hashes(num_bits, capacity);

        // Round up to multiple of 64 for efficient storage
        let num_words = num_bits.div_ceil(64);

        Self {
            bits: RwLock::new(vec![0u64; num_words]),
            num_bits,
            num_hashes,
            count: RwLock::new(0),
        }
    }

    /// Create with explicit parameters.
    #[must_use]
    pub fn with_params(num_bits: usize, num_hashes: u32) -> Self {
        let num_words = num_bits.div_ceil(64);
        Self {
            bits: RwLock::new(vec![0u64; num_words]),
            num_bits,
            num_hashes,
            count: RwLock::new(0),
        }
    }

    /// Insert an item into the filter.
    pub fn insert<T: Hash>(&self, item: &T) {
        let mut bits = self.bits.write();

        for i in 0..self.num_hashes {
            let hash = self.hash_with_seed(item, i);
            let bit_index = (hash as usize) % self.num_bits;
            let word_index = bit_index / 64;
            let bit_offset = bit_index % 64;
            bits[word_index] |= 1u64 << bit_offset;
        }

        *self.count.write() += 1;
    }

    /// Check if an item might be in the filter.
    ///
    /// Returns `true` if the item might be present (possible false positive).
    /// Returns `false` if the item is definitely not present.
    #[must_use]
    pub fn contains<T: Hash>(&self, item: &T) -> bool {
        let bits = self.bits.read();

        for i in 0..self.num_hashes {
            let hash = self.hash_with_seed(item, i);
            let bit_index = (hash as usize) % self.num_bits;
            let word_index = bit_index / 64;
            let bit_offset = bit_index % 64;

            if bits[word_index] & (1u64 << bit_offset) == 0 {
                return false;
            }
        }

        true
    }

    /// Check if item is definitely not present.
    #[must_use]
    pub fn definitely_not_contains<T: Hash>(&self, item: &T) -> bool {
        !self.contains(item)
    }

    /// Get the number of items inserted.
    #[must_use]
    pub fn count(&self) -> usize {
        *self.count.read()
    }

    /// Clear all bits.
    pub fn clear(&self) {
        let mut bits = self.bits.write();
        for word in bits.iter_mut() {
            *word = 0;
        }
        *self.count.write() = 0;
    }

    /// Get the estimated false positive rate based on current fill.
    #[must_use]
    pub fn estimated_fpr(&self) -> f64 {
        let bits = self.bits.read();
        let set_bits: usize = bits.iter().map(|w| w.count_ones() as usize).sum();
        let fill_ratio = set_bits as f64 / self.num_bits as f64;
        fill_ratio.powi(self.num_hashes as i32)
    }

    /// Calculate optimal number of bits.
    fn optimal_bits(capacity: usize, fpr: f64) -> usize {
        let ln2_sq = std::f64::consts::LN_2 * std::f64::consts::LN_2;
        (-(capacity as f64) * fpr.ln() / ln2_sq).ceil() as usize
    }

    /// Calculate optimal number of hash functions.
    fn optimal_hashes(num_bits: usize, capacity: usize) -> u32 {
        let k = (num_bits as f64 / capacity as f64) * std::f64::consts::LN_2;
        k.ceil() as u32
    }

    /// Hash with seed for multiple hash functions.
    fn hash_with_seed<T: Hash>(&self, item: &T, seed: u32) -> u64 {
        let mut hasher = FxHasher::default();
        seed.hash(&mut hasher);
        item.hash(&mut hasher);
        hasher.finish()
    }
}

impl Default for BloomFilter {
    fn default() -> Self {
        // Default: 10K capacity, 1% FPR
        Self::new(10_000, 0.01)
    }
}

#[cfg(test)]
mod bloom_tests {
    use super::*;

    #[test]
    fn test_bloom_new() {
        let bloom = BloomFilter::new(1000, 0.01);
        assert_eq!(bloom.count(), 0);
    }

    #[test]
    fn test_bloom_insert_and_contains() {
        let bloom = BloomFilter::new(1000, 0.01);

        bloom.insert(&"hello");

        assert!(bloom.contains(&"hello"));
        assert_eq!(bloom.count(), 1);
    }

    #[test]
    fn test_bloom_definitely_not_contains() {
        let bloom = BloomFilter::new(1000, 0.01);

        bloom.insert(&"hello");

        // "world" definitely not inserted
        assert!(bloom.definitely_not_contains(&"world"));
    }

    #[test]
    fn test_bloom_no_false_negatives() {
        let bloom = BloomFilter::new(10_000, 0.01);

        // Insert 1000 items
        for i in 0..1000 {
            bloom.insert(&i);
        }

        // All inserted items must be found (no false negatives)
        for i in 0..1000 {
            assert!(bloom.contains(&i), "Item {i} should be found");
        }
    }

    #[test]
    fn test_bloom_false_positive_rate() {
        let bloom = BloomFilter::new(1000, 0.01);

        // Insert 1000 items
        for i in 0..1000 {
            bloom.insert(&i);
        }

        // Check 10000 items not inserted
        let mut false_positives = 0;
        for i in 1000..11000 {
            if bloom.contains(&i) {
                false_positives += 1;
            }
        }

        // FPR should be around 1% (allow margin for statistical variance)
        let fpr = f64::from(false_positives) / 10000.0;
        assert!(fpr < 0.10, "FPR {fpr} should be < 10%");
    }

    #[test]
    fn test_bloom_clear() {
        let bloom = BloomFilter::new(1000, 0.01);

        bloom.insert(&"hello");
        bloom.clear();

        assert_eq!(bloom.count(), 0);
        assert!(!bloom.contains(&"hello"));
    }

    #[test]
    fn test_bloom_integer_keys() {
        let bloom = BloomFilter::new(1000, 0.01);

        bloom.insert(&42u64);
        bloom.insert(&123u64);

        assert!(bloom.contains(&42u64));
        assert!(bloom.contains(&123u64));
        assert!(bloom.definitely_not_contains(&999u64));
    }
}
