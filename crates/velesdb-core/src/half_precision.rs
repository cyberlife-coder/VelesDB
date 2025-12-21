//! Half-precision floating point support for memory-efficient vector storage.
//!
//! This module provides f16 (IEEE 754 half-precision) and bf16 (bfloat16) support,
//! reducing memory usage by 50% compared to f32 with minimal precision loss.
//!
//! # Memory Savings
//!
//! | Dimension | f32 Size | f16 Size | Savings |
//! |-----------|----------|----------|---------|
//! | 768 (BERT)| 3.0 KB   | 1.5 KB   | 50%     |
//! | 1536 (GPT)| 6.0 KB   | 3.0 KB   | 50%     |
//! | 4096      | 16.0 KB  | 8.0 KB   | 50%     |
//!
//! # Format Comparison
//!
//! - **f16**: IEEE 754 half-precision, best general compatibility
//! - **bf16**: Brain float16, same exponent range as f32, better for ML
//!
//! # Usage
//!
//! ```rust
//! use velesdb_core::half_precision::{VectorData, VectorPrecision};
//!
//! // Create from f32
//! let v = VectorData::from_f32_slice(&[0.1, 0.2, 0.3], VectorPrecision::F16);
//!
//! // Convert back to f32 for calculations
//! let f32_vec = v.to_f32_vec();
//! ```

use half::{bf16, f16};
use serde::{Deserialize, Serialize};

/// Vector precision format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum VectorPrecision {
    /// 32-bit floating point (4 bytes per dimension)
    #[default]
    F32,
    /// 16-bit floating point IEEE 754 (2 bytes per dimension)
    F16,
    /// Brain float 16-bit (2 bytes per dimension, same exponent as f32)
    BF16,
}

impl VectorPrecision {
    /// Returns the size in bytes per dimension.
    #[must_use]
    pub const fn bytes_per_element(&self) -> usize {
        match self {
            Self::F32 => 4,
            Self::F16 | Self::BF16 => 2,
        }
    }

    /// Calculates total memory for a vector of given dimension.
    #[must_use]
    pub const fn memory_size(&self, dimension: usize) -> usize {
        self.bytes_per_element() * dimension
    }
}

/// Vector data supporting multiple precision formats.
///
/// Stores vectors in their native precision format to minimize memory usage.
/// Provides conversion methods for distance calculations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VectorData {
    /// Full precision f32 vector
    F32(Vec<f32>),
    /// Half precision f16 vector (50% memory reduction)
    F16(Vec<f16>),
    /// Brain float bf16 vector (50% memory reduction, ML-optimized)
    BF16(Vec<bf16>),
}

impl VectorData {
    /// Creates a new `VectorData` from an f32 slice with the specified precision.
    ///
    /// # Arguments
    ///
    /// * `data` - Source f32 data
    /// * `precision` - Target precision format
    ///
    /// # Example
    ///
    /// ```
    /// use velesdb_core::half_precision::{VectorData, VectorPrecision};
    ///
    /// let v = VectorData::from_f32_slice(&[0.1, 0.2, 0.3], VectorPrecision::F16);
    /// assert_eq!(v.len(), 3);
    /// ```
    #[must_use]
    pub fn from_f32_slice(data: &[f32], precision: VectorPrecision) -> Self {
        match precision {
            VectorPrecision::F32 => Self::F32(data.to_vec()),
            VectorPrecision::F16 => Self::F16(data.iter().map(|&x| f16::from_f32(x)).collect()),
            VectorPrecision::BF16 => Self::BF16(data.iter().map(|&x| bf16::from_f32(x)).collect()),
        }
    }

    /// Creates a new `VectorData` from an f32 vec, taking ownership.
    #[must_use]
    pub fn from_f32_vec(data: Vec<f32>, precision: VectorPrecision) -> Self {
        match precision {
            VectorPrecision::F32 => Self::F32(data),
            VectorPrecision::F16 => Self::F16(data.iter().map(|&x| f16::from_f32(x)).collect()),
            VectorPrecision::BF16 => Self::BF16(data.iter().map(|&x| bf16::from_f32(x)).collect()),
        }
    }

    /// Returns the precision of this vector.
    #[must_use]
    pub const fn precision(&self) -> VectorPrecision {
        match self {
            Self::F32(_) => VectorPrecision::F32,
            Self::F16(_) => VectorPrecision::F16,
            Self::BF16(_) => VectorPrecision::BF16,
        }
    }

    /// Returns the dimension (length) of the vector.
    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            Self::F32(v) => v.len(),
            Self::F16(v) => v.len(),
            Self::BF16(v) => v.len(),
        }
    }

    /// Returns true if the vector is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the memory size in bytes.
    #[must_use]
    pub fn memory_size(&self) -> usize {
        self.precision().memory_size(self.len())
    }

    /// Converts the vector to f32 for calculations.
    ///
    /// For F32 vectors, this clones the data.
    /// For F16/BF16 vectors, this converts each element.
    #[must_use]
    pub fn to_f32_vec(&self) -> Vec<f32> {
        match self {
            Self::F32(v) => v.clone(),
            Self::F16(v) => v.iter().map(|x| x.to_f32()).collect(),
            Self::BF16(v) => v.iter().map(|x| x.to_f32()).collect(),
        }
    }

    /// Returns a reference to the underlying f32 data if precision is F32.
    ///
    /// Returns `None` for F16/BF16 vectors.
    #[must_use]
    pub fn as_f32_slice(&self) -> Option<&[f32]> {
        match self {
            Self::F32(v) => Some(v.as_slice()),
            Self::F16(_) | Self::BF16(_) => None,
        }
    }

    /// Converts to another precision format.
    #[must_use]
    pub fn convert(&self, target: VectorPrecision) -> Self {
        if self.precision() == target {
            return self.clone();
        }
        Self::from_f32_slice(&self.to_f32_vec(), target)
    }
}

impl From<Vec<f32>> for VectorData {
    fn from(data: Vec<f32>) -> Self {
        Self::F32(data)
    }
}

impl From<&[f32]> for VectorData {
    fn from(data: &[f32]) -> Self {
        Self::F32(data.to_vec())
    }
}

// =============================================================================
// Distance calculations for half-precision vectors
// =============================================================================

/// Computes dot product between two `VectorData` with optimal precision handling.
///
/// For F32 vectors, uses SIMD-optimized f32 path.
/// For F16/BF16 vectors, converts to f32 and uses SIMD.
#[must_use]
pub fn dot_product(a: &VectorData, b: &VectorData) -> f32 {
    use crate::simd_avx512::dot_product_auto;

    if let (VectorData::F32(va), VectorData::F32(vb)) = (a, b) {
        dot_product_auto(va, vb)
    } else {
        let va = a.to_f32_vec();
        let vb = b.to_f32_vec();
        dot_product_auto(&va, &vb)
    }
}

/// Computes cosine similarity between two `VectorData`.
#[must_use]
pub fn cosine_similarity(a: &VectorData, b: &VectorData) -> f32 {
    use crate::simd_avx512::cosine_similarity_auto;

    if let (VectorData::F32(va), VectorData::F32(vb)) = (a, b) {
        cosine_similarity_auto(va, vb)
    } else {
        let va = a.to_f32_vec();
        let vb = b.to_f32_vec();
        cosine_similarity_auto(&va, &vb)
    }
}

/// Computes Euclidean distance between two `VectorData`.
#[must_use]
pub fn euclidean_distance(a: &VectorData, b: &VectorData) -> f32 {
    use crate::simd_avx512::euclidean_auto;

    if let (VectorData::F32(va), VectorData::F32(vb)) = (a, b) {
        euclidean_auto(va, vb)
    } else {
        let va = a.to_f32_vec();
        let vb = b.to_f32_vec();
        euclidean_auto(&va, &vb)
    }
}

// =============================================================================
// Tests (TDD)
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f32 = 1e-3; // Relaxed for f16 precision loss

    fn generate_test_vector(dim: usize, seed: f32) -> Vec<f32> {
        #[allow(clippy::cast_precision_loss)]
        (0..dim).map(|i| (seed + i as f32 * 0.1).sin()).collect()
    }

    // =========================================================================
    // VectorPrecision tests
    // =========================================================================

    #[test]
    fn test_precision_bytes_per_element() {
        assert_eq!(VectorPrecision::F32.bytes_per_element(), 4);
        assert_eq!(VectorPrecision::F16.bytes_per_element(), 2);
        assert_eq!(VectorPrecision::BF16.bytes_per_element(), 2);
    }

    #[test]
    fn test_precision_memory_size() {
        // 768D BERT embedding
        assert_eq!(VectorPrecision::F32.memory_size(768), 3072); // 3 KB
        assert_eq!(VectorPrecision::F16.memory_size(768), 1536); // 1.5 KB
        assert_eq!(VectorPrecision::BF16.memory_size(768), 1536); // 1.5 KB
    }

    #[test]
    fn test_precision_default() {
        assert_eq!(VectorPrecision::default(), VectorPrecision::F32);
    }

    // =========================================================================
    // VectorData creation tests
    // =========================================================================

    #[test]
    fn test_vector_data_from_f32_slice_f32() {
        let data = vec![0.1, 0.2, 0.3];
        let v = VectorData::from_f32_slice(&data, VectorPrecision::F32);

        assert_eq!(v.precision(), VectorPrecision::F32);
        assert_eq!(v.len(), 3);
        assert!(!v.is_empty());
    }

    #[test]
    fn test_vector_data_from_f32_slice_f16() {
        let data = vec![0.1, 0.2, 0.3];
        let v = VectorData::from_f32_slice(&data, VectorPrecision::F16);

        assert_eq!(v.precision(), VectorPrecision::F16);
        assert_eq!(v.len(), 3);
    }

    #[test]
    fn test_vector_data_from_f32_slice_bf16() {
        let data = vec![0.1, 0.2, 0.3];
        let v = VectorData::from_f32_slice(&data, VectorPrecision::BF16);

        assert_eq!(v.precision(), VectorPrecision::BF16);
        assert_eq!(v.len(), 3);
    }

    #[test]
    #[allow(clippy::similar_names)]
    fn test_vector_data_memory_size() {
        let data = generate_test_vector(768, 0.0);

        let full = VectorData::from_f32_slice(&data, VectorPrecision::F32);
        let half = VectorData::from_f32_slice(&data, VectorPrecision::F16);
        let brain = VectorData::from_f32_slice(&data, VectorPrecision::BF16);

        assert_eq!(full.memory_size(), 3072);
        assert_eq!(half.memory_size(), 1536);
        assert_eq!(brain.memory_size(), 1536);

        // 50% memory reduction
        assert_eq!(half.memory_size(), full.memory_size() / 2);
    }

    // =========================================================================
    // Conversion tests
    // =========================================================================

    #[test]
    fn test_vector_data_to_f32_roundtrip() {
        let original = vec![0.1, 0.5, 1.0, -0.5, 0.0];

        // F32 -> F32 (exact)
        let v_f32 = VectorData::from_f32_slice(&original, VectorPrecision::F32);
        let back = v_f32.to_f32_vec();
        assert_eq!(original, back);
    }

    #[test]
    fn test_vector_data_f16_roundtrip_precision() {
        let original = vec![0.1, 0.5, 1.0, -0.5, 0.0];

        let v_f16 = VectorData::from_f32_slice(&original, VectorPrecision::F16);
        let back = v_f16.to_f32_vec();

        // f16 has ~3.3 decimal digits of precision
        for (orig, converted) in original.iter().zip(back.iter()) {
            assert!(
                (orig - converted).abs() < 0.001,
                "f16 roundtrip error: {orig} vs {converted}"
            );
        }
    }

    #[test]
    fn test_vector_data_bf16_roundtrip_precision() {
        let original = vec![0.1, 0.5, 1.0, -0.5, 0.0];

        let v_bf16 = VectorData::from_f32_slice(&original, VectorPrecision::BF16);
        let back = v_bf16.to_f32_vec();

        // bf16 has ~2.4 decimal digits of precision
        for (orig, converted) in original.iter().zip(back.iter()) {
            assert!(
                (orig - converted).abs() < 0.01,
                "bf16 roundtrip error: {orig} vs {converted}"
            );
        }
    }

    #[test]
    fn test_vector_data_convert() {
        let data = vec![0.1, 0.2, 0.3];
        let original = VectorData::from_f32_slice(&data, VectorPrecision::F32);

        let to_half = original.convert(VectorPrecision::F16);
        assert_eq!(to_half.precision(), VectorPrecision::F16);

        let to_brain = original.convert(VectorPrecision::BF16);
        assert_eq!(to_brain.precision(), VectorPrecision::BF16);

        // Same precision returns clone
        let same = original.convert(VectorPrecision::F32);
        assert_eq!(same.precision(), VectorPrecision::F32);
    }

    #[test]
    fn test_vector_data_as_f32_slice() {
        let data = vec![0.1, 0.2, 0.3];

        let v_f32 = VectorData::from_f32_slice(&data, VectorPrecision::F32);
        assert!(v_f32.as_f32_slice().is_some());

        let v_f16 = VectorData::from_f32_slice(&data, VectorPrecision::F16);
        assert!(v_f16.as_f32_slice().is_none());
    }

    // =========================================================================
    // Distance calculation tests
    // =========================================================================

    #[test]
    fn test_dot_product_f32() {
        let a = VectorData::from_f32_slice(&[1.0, 2.0, 3.0], VectorPrecision::F32);
        let b = VectorData::from_f32_slice(&[4.0, 5.0, 6.0], VectorPrecision::F32);

        let result = dot_product(&a, &b);
        assert!(
            (result - 32.0).abs() < EPSILON,
            "Expected 32.0, got {result}"
        );
    }

    #[test]
    fn test_dot_product_f16() {
        let a = VectorData::from_f32_slice(&[1.0, 2.0, 3.0], VectorPrecision::F16);
        let b = VectorData::from_f32_slice(&[4.0, 5.0, 6.0], VectorPrecision::F16);

        let result = dot_product(&a, &b);
        assert!((result - 32.0).abs() < 0.1, "f16 dot product: got {result}");
    }

    #[test]
    fn test_dot_product_bf16() {
        let a = VectorData::from_f32_slice(&[1.0, 2.0, 3.0], VectorPrecision::BF16);
        let b = VectorData::from_f32_slice(&[4.0, 5.0, 6.0], VectorPrecision::BF16);

        let result = dot_product(&a, &b);
        assert!(
            (result - 32.0).abs() < 0.5,
            "bf16 dot product: got {result}"
        );
    }

    #[test]
    fn test_cosine_similarity_identical_f16() {
        let data = generate_test_vector(768, 0.0);
        let a = VectorData::from_f32_slice(&data, VectorPrecision::F16);
        let b = VectorData::from_f32_slice(&data, VectorPrecision::F16);

        let result = cosine_similarity(&a, &b);
        assert!(
            (result - 1.0).abs() < 0.01,
            "Identical f16 vectors cosine ≈ 1.0, got {result}"
        );
    }

    #[test]
    fn test_euclidean_distance_f16() {
        let a = VectorData::from_f32_slice(&[0.0, 0.0, 0.0], VectorPrecision::F16);
        let b = VectorData::from_f32_slice(&[3.0, 4.0, 0.0], VectorPrecision::F16);

        let result = euclidean_distance(&a, &b);
        assert!(
            (result - 5.0).abs() < 0.1,
            "f16 euclidean 3-4-5: got {result}"
        );
    }

    #[test]
    fn test_mixed_precision_distance() {
        let a = VectorData::from_f32_slice(&[1.0, 2.0, 3.0], VectorPrecision::F32);
        let b = VectorData::from_f32_slice(&[1.0, 2.0, 3.0], VectorPrecision::F16);

        // Should work with mixed precision
        let result = cosine_similarity(&a, &b);
        assert!(
            (result - 1.0).abs() < 0.01,
            "Mixed precision cosine: got {result}"
        );
    }

    // =========================================================================
    // Precision impact tests (recall quality)
    // =========================================================================

    #[test]
    fn test_f16_preserves_ranking() {
        // Verify that f16 preserves relative ordering of distances
        let query = generate_test_vector(768, 0.0);
        let close = generate_test_vector(768, 0.1); // Similar
        let far = generate_test_vector(768, 5.0); // Different

        // F32 distances
        let q_f32 = VectorData::from_f32_slice(&query, VectorPrecision::F32);
        let close_f32 = VectorData::from_f32_slice(&close, VectorPrecision::F32);
        let far_f32 = VectorData::from_f32_slice(&far, VectorPrecision::F32);

        let dist_close_f32 = cosine_similarity(&q_f32, &close_f32);
        let dist_far_f32 = cosine_similarity(&q_f32, &far_f32);

        // F16 distances
        let q_f16 = VectorData::from_f32_slice(&query, VectorPrecision::F16);
        let close_f16 = VectorData::from_f32_slice(&close, VectorPrecision::F16);
        let far_f16 = VectorData::from_f32_slice(&far, VectorPrecision::F16);

        let dist_close_f16 = cosine_similarity(&q_f16, &close_f16);
        let dist_far_f16 = cosine_similarity(&q_f16, &far_f16);

        // Ranking should be preserved
        assert!(
            dist_close_f32 > dist_far_f32,
            "F32: close should be more similar than far"
        );
        assert!(
            dist_close_f16 > dist_far_f16,
            "F16: ranking should be preserved"
        );
    }

    // =========================================================================
    // Serialization tests
    // =========================================================================

    #[test]
    fn test_vector_data_serialization() {
        let data = vec![0.1, 0.2, 0.3];

        for precision in [
            VectorPrecision::F32,
            VectorPrecision::F16,
            VectorPrecision::BF16,
        ] {
            let v = VectorData::from_f32_slice(&data, precision);
            let json = serde_json::to_string(&v).expect("serialize");
            let back: VectorData = serde_json::from_str(&json).expect("deserialize");

            assert_eq!(v.precision(), back.precision());
            assert_eq!(v.len(), back.len());
        }
    }

    #[test]
    fn test_precision_serialization() {
        for precision in [
            VectorPrecision::F32,
            VectorPrecision::F16,
            VectorPrecision::BF16,
        ] {
            let json = serde_json::to_string(&precision).expect("serialize");
            let back: VectorPrecision = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(precision, back);
        }
    }

    // =========================================================================
    // Edge cases
    // =========================================================================

    #[test]
    fn test_empty_vector() {
        let v = VectorData::from_f32_slice(&[], VectorPrecision::F16);
        assert!(v.is_empty());
        assert_eq!(v.len(), 0);
        assert_eq!(v.memory_size(), 0);
    }

    #[test]
    fn test_large_vector_4096d() {
        let data = generate_test_vector(4096, 0.0);

        let v_f16 = VectorData::from_f32_slice(&data, VectorPrecision::F16);
        assert_eq!(v_f16.len(), 4096);
        assert_eq!(v_f16.memory_size(), 8192); // 8 KB

        // Verify conversion works
        let back = v_f16.to_f32_vec();
        assert_eq!(back.len(), 4096);
    }

    #[test]
    fn test_from_impls() {
        let data = vec![0.1, 0.2, 0.3];

        // From Vec<f32>
        let v: VectorData = data.clone().into();
        assert_eq!(v.precision(), VectorPrecision::F32);

        // From &[f32]
        let v: VectorData = data.as_slice().into();
        assert_eq!(v.precision(), VectorPrecision::F32);
    }
}
