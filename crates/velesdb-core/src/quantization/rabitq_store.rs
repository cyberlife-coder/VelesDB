//! Contiguous storage for `RaBitQ`-encoded vectors.
//!
//! Split layout: binary codes (`u64` words) and scalar corrections are stored
//! in separate contiguous arrays. This layout is SIMD-friendly — the hot
//! XOR + popcount loop touches only the bits array, keeping corrections out
//! of the cache line until the affine correction step.
//!
//! # Memory Layout
//!
//! ```text
//! bits_data:    [vec0_w0, vec0_w1, ..., vec0_wN, vec1_w0, ...]
//! corrections:  [corr0, corr1, ...]
//! ```

use super::rabitq::RaBitQCorrection;

/// Contiguous storage for `RaBitQ`-encoded vectors with split layout.
///
/// Bits and corrections are stored in separate arrays so that the
/// XOR + popcount hot loop only touches the bits array (better cache
/// utilization during graph traversal).
#[derive(Debug)]
#[allow(dead_code)] // Phase 3: methods used progressively as integration expands
pub struct RaBitQVectorStore {
    /// Contiguous binary codes: `[count * words_per_vector]` u64 words.
    bits_data: Vec<u64>,
    /// Scalar correction factors, one per vector.
    corrections: Vec<RaBitQCorrection>,
    /// Number of u64 words per vector (`ceil(dimension / 64)`).
    words_per_vector: usize,
    /// Original vector dimension.
    dimension: usize,
    /// Number of vectors stored.
    count: usize,
}

#[allow(dead_code)] // Phase 3: methods used progressively as integration expands
impl RaBitQVectorStore {
    /// Creates a new store with pre-allocated capacity.
    ///
    /// # Arguments
    ///
    /// * `dimension` - Vector dimension (determines `words_per_vector`)
    /// * `capacity` - Expected number of vectors (for pre-allocation)
    #[must_use]
    pub fn new(dimension: usize, capacity: usize) -> Self {
        let words_per_vector = dimension.div_ceil(64);
        Self {
            bits_data: Vec::with_capacity(capacity * words_per_vector),
            corrections: Vec::with_capacity(capacity),
            words_per_vector,
            dimension,
            count: 0,
        }
    }

    /// Appends an encoded vector's bits and correction to the store.
    ///
    /// The unused padding bits of the last word are masked to zero so that the
    /// XOR + popcount distance estimator cannot underflow even if `bits` came
    /// from untrusted (deserialized/tampered) input. Masking is correctness-
    /// preserving: padding bits carry no information (see
    /// [`super::rabitq::signs_to_bits`]).
    pub fn push(&mut self, bits: &[u64], correction: RaBitQCorrection) {
        debug_assert_eq!(bits.len(), self.words_per_vector);
        self.bits_data.extend_from_slice(bits);
        if !bits.is_empty() {
            if let Some(last) = self.bits_data.last_mut() {
                *last &= super::rabitq::last_word_mask(self.dimension);
            }
        }
        self.corrections.push(correction);
        self.count += 1;
    }

    /// Returns a zero-copy slice of the binary codes for vector at `index`.
    #[must_use]
    pub fn get_bits_slice(&self, index: usize) -> Option<&[u64]> {
        if index >= self.count {
            return None;
        }
        let start = index * self.words_per_vector;
        let end = start + self.words_per_vector;
        Some(&self.bits_data[start..end])
    }

    /// Returns a zero-copy reference to the correction for vector at `index`.
    #[must_use]
    pub fn get_correction(&self, index: usize) -> Option<&RaBitQCorrection> {
        self.corrections.get(index)
    }

    /// Prefetches the bits data for `index` into the L1 cache.
    ///
    /// Call this 2-3 neighbors ahead in the graph traversal loop to hide
    /// memory latency. No-op if the index is out of bounds.
    #[inline]
    pub fn prefetch(&self, index: usize) {
        if index < self.count {
            let start = index * self.words_per_vector;
            crate::simd_native::prefetch_vector_u64(&self.bits_data[start..]);
        }
    }

    /// Returns the number of stored vectors.
    #[must_use]
    pub fn len(&self) -> usize {
        self.count
    }

    /// Returns `true` if no vectors are stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Returns the original vector dimension.
    #[must_use]
    pub fn dimension(&self) -> usize {
        self.dimension
    }
}

#[cfg(test)]
#[path = "rabitq_store_tests.rs"]
mod tests;
