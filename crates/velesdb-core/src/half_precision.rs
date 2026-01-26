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
/// For F16/BF16 vectors, converts to f32 on the fly without allocation.
#[must_use]
pub fn dot_product(a: &VectorData, b: &VectorData) -> f32 {
    use crate::simd_avx512::dot_product_auto;

    match (a, b) {
        (VectorData::F32(va), VectorData::F32(vb)) => dot_product_auto(va, vb),
        (VectorData::F32(va), VectorData::F16(vb)) => {
            va.iter().zip(vb.iter()).map(|(&x, y)| x * y.to_f32()).sum()
        }
        (VectorData::F16(va), VectorData::F32(vb)) => {
            va.iter().zip(vb.iter()).map(|(x, &y)| x.to_f32() * y).sum()
        }
        (VectorData::F16(va), VectorData::F16(vb)) => va
            .iter()
            .zip(vb.iter())
            .map(|(x, y)| x.to_f32() * y.to_f32())
            .sum(),
        (VectorData::F32(va), VectorData::BF16(vb)) => {
            va.iter().zip(vb.iter()).map(|(&x, y)| x * y.to_f32()).sum()
        }
        (VectorData::BF16(va), VectorData::F32(vb)) => {
            va.iter().zip(vb.iter()).map(|(x, &y)| x.to_f32() * y).sum()
        }
        (VectorData::BF16(va), VectorData::BF16(vb)) => va
            .iter()
            .zip(vb.iter())
            .map(|(x, y)| x.to_f32() * y.to_f32())
            .sum(),
        // Fallback for mixed F16/BF16 (rare)
        _ => {
            let va = a.to_f32_vec();
            let vb = b.to_f32_vec();
            dot_product_auto(&va, &vb)
        }
    }
}

/// Computes cosine similarity between two `VectorData`.
#[must_use]
pub fn cosine_similarity(a: &VectorData, b: &VectorData) -> f32 {
    use crate::simd_avx512::cosine_similarity_auto;

    if let (VectorData::F32(va), VectorData::F32(vb)) = (a, b) {
        cosine_similarity_auto(va, vb)
    } else {
        let dot = dot_product(a, b);
        let norm_a = norm_squared(a).sqrt();
        let norm_b = norm_squared(b).sqrt();

        if norm_a < f32::EPSILON || norm_b < f32::EPSILON {
            0.0
        } else {
            dot / (norm_a * norm_b)
        }
    }
}

/// Computes Euclidean distance between two `VectorData`.
#[must_use]
pub fn euclidean_distance(a: &VectorData, b: &VectorData) -> f32 {
    use crate::simd_avx512::euclidean_auto;

    match (a, b) {
        (VectorData::F32(va), VectorData::F32(vb)) => euclidean_auto(va, vb),
        (VectorData::F32(va), VectorData::F16(vb)) => va
            .iter()
            .zip(vb.iter())
            .map(|(&x, y)| (x - y.to_f32()).powi(2))
            .sum::<f32>()
            .sqrt(),
        (VectorData::F16(va), VectorData::F32(vb)) => va
            .iter()
            .zip(vb.iter())
            .map(|(x, &y)| (x.to_f32() - y).powi(2))
            .sum::<f32>()
            .sqrt(),
        (VectorData::F16(va), VectorData::F16(vb)) => va
            .iter()
            .zip(vb.iter())
            .map(|(x, y)| (x.to_f32() - y.to_f32()).powi(2))
            .sum::<f32>()
            .sqrt(),
        // Fallback for others
        _ => {
            let va = a.to_f32_vec();
            let vb = b.to_f32_vec();
            euclidean_auto(&va, &vb)
        }
    }
}

/// Helper to compute squared L2 norm without allocation
fn norm_squared(v: &VectorData) -> f32 {
    match v {
        VectorData::F32(data) => data.iter().map(|&x| x * x).sum(),
        VectorData::F16(data) => data
            .iter()
            .map(|x| {
                let f = x.to_f32();
                f * f
            })
            .sum(),
        VectorData::BF16(data) => data
            .iter()
            .map(|x| {
                let f = x.to_f32();
                f * f
            })
            .sum(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vector_precision_bytes_per_element() {
        assert_eq!(VectorPrecision::F32.bytes_per_element(), 4);
        assert_eq!(VectorPrecision::F16.bytes_per_element(), 2);
        assert_eq!(VectorPrecision::BF16.bytes_per_element(), 2);
    }

    #[test]
    fn test_vector_precision_memory_size() {
        assert_eq!(VectorPrecision::F32.memory_size(768), 3072);
        assert_eq!(VectorPrecision::F16.memory_size(768), 1536);
    }

    #[test]
    fn test_vector_precision_default() {
        let precision = VectorPrecision::default();
        assert_eq!(precision, VectorPrecision::F32);
    }

    #[test]
    fn test_vector_data_from_f32_slice_f32() {
        let data = vec![0.1, 0.2, 0.3];
        let v = VectorData::from_f32_slice(&data, VectorPrecision::F32);
        assert_eq!(v.len(), 3);
        assert_eq!(v.precision(), VectorPrecision::F32);
    }

    #[test]
    fn test_vector_data_from_f32_slice_f16() {
        let data = vec![0.1, 0.2, 0.3];
        let v = VectorData::from_f32_slice(&data, VectorPrecision::F16);
        assert_eq!(v.len(), 3);
        assert_eq!(v.precision(), VectorPrecision::F16);
    }

    #[test]
    fn test_vector_data_from_f32_slice_bf16() {
        let data = vec![0.1, 0.2, 0.3];
        let v = VectorData::from_f32_slice(&data, VectorPrecision::BF16);
        assert_eq!(v.len(), 3);
        assert_eq!(v.precision(), VectorPrecision::BF16);
    }

    #[test]
    fn test_vector_data_to_f32_vec() {
        let data = vec![1.0, 2.0, 3.0];
        let v = VectorData::from_f32_slice(&data, VectorPrecision::F32);
        let result = v.to_f32_vec();
        assert_eq!(result, data);
    }

    #[test]
    fn test_vector_data_f16_roundtrip() {
        let data = vec![1.0, 2.0, 3.0];
        let v = VectorData::from_f32_slice(&data, VectorPrecision::F16);
        let result = v.to_f32_vec();
        for (a, b) in data.iter().zip(result.iter()) {
            assert!((a - b).abs() < 0.01);
        }
    }

    #[test]
    fn test_vector_data_is_empty() {
        let empty = VectorData::from_f32_slice(&[], VectorPrecision::F32);
        assert!(empty.is_empty());

        let non_empty = VectorData::from_f32_slice(&[1.0], VectorPrecision::F32);
        assert!(!non_empty.is_empty());
    }

    #[test]
    fn test_cosine_similarity_identical() {
        let v1 = VectorData::from_f32_slice(&[1.0, 0.0, 0.0], VectorPrecision::F32);
        let v2 = VectorData::from_f32_slice(&[1.0, 0.0, 0.0], VectorPrecision::F32);
        let sim = cosine_similarity(&v1, &v2);
        assert!((sim - 1.0).abs() < 1e-5);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let v1 = VectorData::from_f32_slice(&[1.0, 0.0, 0.0], VectorPrecision::F32);
        let v2 = VectorData::from_f32_slice(&[0.0, 1.0, 0.0], VectorPrecision::F32);
        let sim = cosine_similarity(&v1, &v2);
        assert!(sim.abs() < 1e-5);
    }

    #[test]
    fn test_euclidean_distance_identical() {
        let v1 = VectorData::from_f32_slice(&[1.0, 2.0, 3.0], VectorPrecision::F32);
        let v2 = VectorData::from_f32_slice(&[1.0, 2.0, 3.0], VectorPrecision::F32);
        let dist = euclidean_distance(&v1, &v2);
        assert!(dist.abs() < 1e-5);
    }

    #[test]
    fn test_euclidean_distance_345() {
        let v1 = VectorData::from_f32_slice(&[0.0, 0.0], VectorPrecision::F32);
        let v2 = VectorData::from_f32_slice(&[3.0, 4.0], VectorPrecision::F32);
        let dist = euclidean_distance(&v1, &v2);
        assert!((dist - 5.0).abs() < 1e-5);
    }

    #[test]
    fn test_norm_squared_f32() {
        let v = VectorData::from_f32_slice(&[3.0, 4.0], VectorPrecision::F32);
        let norm = norm_squared(&v);
        assert!((norm - 25.0).abs() < 1e-5);
    }

    #[test]
    fn test_cosine_similarity_f16_vs_f32() {
        let v1 = VectorData::from_f32_slice(&[1.0, 2.0, 3.0], VectorPrecision::F16);
        let v2 = VectorData::from_f32_slice(&[1.0, 2.0, 3.0], VectorPrecision::F32);
        let sim = cosine_similarity(&v1, &v2);
        assert!((sim - 1.0).abs() < 0.01);
    }
}
