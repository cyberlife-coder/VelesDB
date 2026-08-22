//! Scalar Quantization (SQ8) for fast HNSW traversal.
//!
//! Based on VSAG paper (arXiv:2503.17911): dual-precision architecture
//! using int8 for graph traversal and float32 for final re-ranking.
//!
//! # Performance Benefits
//!
//! - **4x memory bandwidth reduction** during traversal
//! - **SIMD-friendly**: 32 int8 values fit in 256-bit register (vs 8 float32)
//! - **Cache efficiency**: More vectors fit in L1/L2 cache
//!
//! # Algorithm
//!
//! For each dimension:
//! - Compute min/max from training data
//! - Scale to [0, 255] range: `q = round((x - min) / (max - min) * 255)`
//! - Store scale and offset for reconstruction
//!
//! # Safety (EPIC-032/US-007)
//!
//! All `as u32` casts in distance computation are proven safe:
//! - Input: u8 values in [0, 255]
//! - Difference: i32 in [-255, 255]
//! - Squared: i32 in [0, 65025] (always non-negative, fits in u32)

use std::sync::Arc;

// =============================================================================
// SIMD-optimized distance computation for int8 quantized vectors
// =============================================================================

/// Computes L2 squared distance between two quantized vectors using SIMD.
///
/// Uses 8-wide unrolling for better instruction-level parallelism.
/// On x86_64 with AVX2, processes 32 bytes per iteration.
///
/// # Performance
///
/// - **4x memory bandwidth reduction** vs float32
/// - **Better SIMD utilization**: 32 int8 fit in 256-bit register vs 8 float32
#[inline]
fn distance_l2_quantized_simd(a: &[u8], b: &[u8]) -> u32 {
    debug_assert_eq!(a.len(), b.len());

    // Process in chunks of 8 for better ILP (Instruction Level Parallelism)
    let chunks = a.len() / 8;
    let remainder = a.len() % 8;

    let mut sum0: u32 = 0;
    let mut sum1: u32 = 0;
    let mut sum2: u32 = 0;
    let mut sum3: u32 = 0;

    // Main loop: 8-wide unrolling
    for i in 0..chunks {
        let base = i * 8;

        // Unroll 8 iterations with 4 accumulators
        let d0 = i32::from(a[base]) - i32::from(b[base]);
        let d1 = i32::from(a[base + 1]) - i32::from(b[base + 1]);
        let d2 = i32::from(a[base + 2]) - i32::from(b[base + 2]);
        let d3 = i32::from(a[base + 3]) - i32::from(b[base + 3]);
        let d4 = i32::from(a[base + 4]) - i32::from(b[base + 4]);
        let d5 = i32::from(a[base + 5]) - i32::from(b[base + 5]);
        let d6 = i32::from(a[base + 6]) - i32::from(b[base + 6]);
        let d7 = i32::from(a[base + 7]) - i32::from(b[base + 7]);

        // SAFETY (EPIC-032/US-007): d_i in [-255, 255], so d_i*d_i in [0, 65025]
        // This is always non-negative and fits in u32 (max 4,294,967,295)
        #[allow(clippy::cast_sign_loss)] // Proven non-negative: square of integer
        {
            sum0 += (d0 * d0) as u32 + (d4 * d4) as u32;
            sum1 += (d1 * d1) as u32 + (d5 * d5) as u32;
            sum2 += (d2 * d2) as u32 + (d6 * d6) as u32;
            sum3 += (d3 * d3) as u32 + (d7 * d7) as u32;
        }
    }

    // Handle remainder
    let base = chunks * 8;
    for i in 0..remainder {
        let diff = i32::from(a[base + i]) - i32::from(b[base + i]);
        // SAFETY (EPIC-032/US-007): diff in [-255, 255], diff*diff in [0, 65025]
        #[allow(clippy::cast_sign_loss)]
        {
            sum0 += (diff * diff) as u32;
        }
    }

    sum0 + sum1 + sum2 + sum3
}

/// Computes asymmetric L2 distance: float32 query vs quantized candidate.
///
/// Uses precomputed lookup tables for efficient SIMD execution.
/// Based on VSAG paper's ADT (Asymmetric Distance Table) approach.
#[inline]
fn distance_l2_asymmetric_simd(
    query: &[f32],
    quantized: &[u8],
    min_vals: &[f32],
    inv_scales: &[f32],
) -> f32 {
    debug_assert_eq!(query.len(), quantized.len());
    debug_assert_eq!(query.len(), min_vals.len());
    debug_assert_eq!(query.len(), inv_scales.len());

    let chunks = query.len() / 4;
    let remainder = query.len() % 4;

    let (sum0, sum1, sum2, sum3) =
        asymmetric_chunked_sum(query, quantized, min_vals, inv_scales, chunks);

    let remainder_sum = asymmetric_remainder_sum(
        query,
        quantized,
        min_vals,
        inv_scales,
        chunks * 4,
        remainder,
    );

    (sum0 + sum1 + sum2 + sum3 + remainder_sum).sqrt()
}

/// Computes the main chunked (4-wide) sum for asymmetric L2 distance.
#[inline]
fn asymmetric_chunked_sum(
    query: &[f32],
    quantized: &[u8],
    min_vals: &[f32],
    inv_scales: &[f32],
    chunks: usize,
) -> (f32, f32, f32, f32) {
    let mut sum0: f32 = 0.0;
    let mut sum1: f32 = 0.0;
    let mut sum2: f32 = 0.0;
    let mut sum3: f32 = 0.0;

    for i in 0..chunks {
        let base = i * 4;

        let dq0 = f32::from(quantized[base]) * inv_scales[base] + min_vals[base];
        let dq1 = f32::from(quantized[base + 1]) * inv_scales[base + 1] + min_vals[base + 1];
        let dq2 = f32::from(quantized[base + 2]) * inv_scales[base + 2] + min_vals[base + 2];
        let dq3 = f32::from(quantized[base + 3]) * inv_scales[base + 3] + min_vals[base + 3];

        let d0 = query[base] - dq0;
        let d1 = query[base + 1] - dq1;
        let d2 = query[base + 2] - dq2;
        let d3 = query[base + 3] - dq3;

        sum0 += d0 * d0;
        sum1 += d1 * d1;
        sum2 += d2 * d2;
        sum3 += d3 * d3;
    }

    (sum0, sum1, sum2, sum3)
}

/// Computes the remainder sum for asymmetric L2 distance (elements not covered by 4-wide chunks).
#[inline]
fn asymmetric_remainder_sum(
    query: &[f32],
    quantized: &[u8],
    min_vals: &[f32],
    inv_scales: &[f32],
    base: usize,
    remainder: usize,
) -> f32 {
    let mut sum = 0.0_f32;
    for i in 0..remainder {
        let idx = base + i;
        let dq = f32::from(quantized[idx]) * inv_scales[idx] + min_vals[idx];
        let diff = query[idx] - dq;
        sum += diff * diff;
    }
    sum
}

/// Quantization parameters learned from training data.
///
/// This is the TRAINED per-dimension quantizer the SQ8 HNSW backend runs
/// on — not to be confused with [`crate::quantization::QuantizedVector`],
/// the standalone per-vector min/max codec (each vector carries its own
/// range, no training, no shared parameters).
#[derive(Debug, Clone)]
pub struct ScalarQuantizer {
    /// Minimum value per dimension
    pub min_vals: Vec<f32>,
    /// Scale factor per dimension: 255 / (max - min)
    pub scales: Vec<f32>,
    /// Inverse scale factor: 1 / scale (precomputed for fast dequantization)
    pub inv_scales: Vec<f32>,
    /// Vector dimension
    pub dimension: usize,
}

/// Quantized vector storage (int8 per dimension).
///
/// Codes only — the shared per-dimension parameters live in the
/// [`ScalarQuantizer`] that produced them. Distinct from the
/// self-describing [`crate::quantization::QuantizedVector`] primitive,
/// which embeds a per-vector min/max instead.
#[derive(Debug, Clone)]
pub struct QuantizedVector {
    /// Quantized values [0, 255]
    pub data: Vec<u8>,
}

/// Quantized vector storage with shared quantizer reference.
#[derive(Debug, Clone)]
pub struct QuantizedVectorStore {
    /// Shared quantizer parameters
    quantizer: Arc<ScalarQuantizer>,
    /// Quantized vectors (flattened: node_id * dimension + dim_idx)
    data: Vec<u8>,
    /// Number of vectors stored
    count: usize,
}

impl ScalarQuantizer {
    /// Creates a new quantizer from training vectors.
    ///
    /// # Arguments
    ///
    /// * `vectors` - Training vectors to compute min/max per dimension
    ///
    /// # Errors
    ///
    /// Returns `Error::InvalidQuantizerConfig` if `vectors` is empty or
    /// vectors have inconsistent dimensions.
    pub fn train(vectors: &[&[f32]]) -> crate::error::Result<Self> {
        if vectors.is_empty() {
            return Err(crate::error::Error::InvalidQuantizerConfig(
                "cannot train on empty vectors".to_string(),
            ));
        }
        let dimension = vectors[0].len();
        if !vectors.iter().all(|v| v.len() == dimension) {
            return Err(crate::error::Error::InvalidQuantizerConfig(
                "all vectors must have same dimension".to_string(),
            ));
        }

        let mut min_vals = vec![f32::MAX; dimension];
        let mut max_vals = vec![f32::MIN; dimension];

        // Find min/max per dimension
        for vec in vectors {
            for (i, &val) in vec.iter().enumerate() {
                min_vals[i] = min_vals[i].min(val);
                max_vals[i] = max_vals[i].max(val);
            }
        }

        // Compute scales (avoid division by zero)
        let scales: Vec<f32> = min_vals
            .iter()
            .zip(max_vals.iter())
            .map(|(&min, &max)| {
                let range = max - min;
                if range.abs() < 1e-10 {
                    1.0 // Constant dimension, scale doesn't matter
                } else {
                    255.0 / range
                }
            })
            .collect();

        // Precompute inverse scales for fast dequantization
        let inv_scales: Vec<f32> = scales.iter().map(|&s| 1.0 / s).collect();

        Ok(Self {
            min_vals,
            scales,
            inv_scales,
            dimension,
        })
    }

    /// Quantizes a float32 vector to int8.
    #[must_use]
    pub fn quantize(&self, vector: &[f32]) -> QuantizedVector {
        debug_assert_eq!(vector.len(), self.dimension);

        let data: Vec<u8> = vector
            .iter()
            .zip(self.min_vals.iter())
            .zip(self.scales.iter())
            .map(|((&val, &min), &scale)| {
                let q = ((val - min) * scale).round();
                q.clamp(0.0, 255.0) as u8
            })
            .collect();

        QuantizedVector { data }
    }

    /// Dequantizes an int8 vector back to float32.
    #[must_use]
    pub fn dequantize(&self, quantized: &QuantizedVector) -> Vec<f32> {
        debug_assert_eq!(quantized.data.len(), self.dimension);

        quantized
            .data
            .iter()
            .zip(self.min_vals.iter())
            .zip(self.inv_scales.iter())
            .map(|((&q, &min), &inv_scale)| {
                // x = q * inv_scale + min (multiplication is faster than division)
                f32::from(q) * inv_scale + min
            })
            .collect()
    }

    /// Computes approximate L2 distance between quantized vectors.
    ///
    /// This is ~4x faster than float32 due to SIMD efficiency.
    #[inline]
    #[must_use]
    pub fn distance_l2_quantized(&self, a: &QuantizedVector, b: &QuantizedVector) -> u32 {
        debug_assert_eq!(a.data.len(), b.data.len());
        distance_l2_quantized_simd(&a.data, &b.data)
    }

    /// Computes approximate L2 distance using raw slices (zero-copy).
    ///
    /// Useful for QuantizedVectorStore.get_slice() access pattern.
    #[inline]
    #[must_use]
    pub fn distance_l2_quantized_slice(&self, a: &[u8], b: &[u8]) -> u32 {
        debug_assert_eq!(a.len(), b.len());
        distance_l2_quantized_simd(a, b)
    }

    /// Computes approximate L2 distance: quantized vs float32 query.
    ///
    /// Asymmetric distance: query stays in float32, candidates in int8.
    /// This is the VSAG "ADT" (Asymmetric Distance Table) approach.
    #[inline]
    #[must_use]
    pub fn distance_l2_asymmetric(&self, query: &[f32], quantized: &QuantizedVector) -> f32 {
        debug_assert_eq!(query.len(), self.dimension);
        debug_assert_eq!(quantized.data.len(), self.dimension);

        distance_l2_asymmetric_simd(query, &quantized.data, &self.min_vals, &self.inv_scales)
    }

    /// Computes asymmetric L2 distance using raw slice (zero-copy).
    #[inline]
    #[must_use]
    pub fn distance_l2_asymmetric_slice(&self, query: &[f32], quantized: &[u8]) -> f32 {
        debug_assert_eq!(query.len(), self.dimension);
        debug_assert_eq!(quantized.len(), self.dimension);

        distance_l2_asymmetric_simd(query, quantized, &self.min_vals, &self.inv_scales)
    }
}

/// On-disk form of [`ScalarQuantizer`]: `inv_scales` is derived, so only the
/// trained per-dimension `min_vals`/`scales` are persisted.
#[cfg(feature = "persistence")]
#[derive(serde::Serialize, serde::Deserialize)]
struct PersistedScalarQuantizer {
    dimension: usize,
    min_vals: Vec<f32>,
    scales: Vec<f32>,
}

/// Size cap for `sq8.idx`: 2 f32 vectors per dimension — even a 65 536-dim
/// quantizer is ~512 KiB, so anything past the cap is corruption, not data.
#[cfg(feature = "persistence")]
const MAX_SQ8_INDEX_BYTES: u64 = 16 * 1024 * 1024;

#[cfg(feature = "persistence")]
impl ScalarQuantizer {
    /// Saves the trained quantizer to `<dir>/sq8.idx` using postcard with
    /// atomic write (parity with `RaBitQIndex::save` / `rabitq.idx`).
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::Error::Io`] if serialization or file I/O fails.
    pub fn save(&self, dir: &std::path::Path) -> Result<(), crate::error::Error> {
        let persisted = PersistedScalarQuantizer {
            dimension: self.dimension,
            min_vals: self.min_vals.clone(),
            scales: self.scales.clone(),
        };
        let data = postcard::to_allocvec(&persisted).map_err(|e| {
            crate::error::Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("failed to serialize SQ8 quantizer: {e}"),
            ))
        })?;
        let final_path = dir.join("sq8.idx");
        crate::storage::atomic_write::atomic_write(&final_path, &data).map_err(|e| {
            crate::error::Error::Io(std::io::Error::new(
                e.kind(),
                format!("failed to write SQ8 quantizer: {e}"),
            ))
        })
    }

    /// Loads a trained quantizer from `<dir>/sq8.idx`. Returns `None` if the
    /// file doesn't exist.
    ///
    /// The decoded quantizer is validated so a corrupt artifact is rejected
    /// here rather than producing mismatched per-dimension indexing in the
    /// distance kernels.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::Error::Io`] if deserialization or file I/O
    /// fails, or [`crate::error::Error::IndexCorrupted`] if the file exceeds
    /// the size cap or the decoded shape is inconsistent with `dimension`.
    pub fn load(dir: &std::path::Path) -> Result<Option<Self>, crate::error::Error> {
        let path = dir.join("sq8.idx");
        if !path.exists() {
            return Ok(None);
        }
        let file_len = std::fs::metadata(&path)?.len();
        if file_len > MAX_SQ8_INDEX_BYTES {
            return Err(crate::error::Error::IndexCorrupted(format!(
                "SQ8 quantizer file is {file_len} bytes, exceeds cap {MAX_SQ8_INDEX_BYTES}"
            )));
        }
        let data = std::fs::read(&path)?;
        let persisted: PersistedScalarQuantizer = postcard::from_bytes(&data).map_err(|e| {
            crate::error::Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("failed to deserialize SQ8 quantizer: {e}"),
            ))
        })?;
        Self::from_persisted(persisted).map(Some)
    }

    /// Validates a decoded artifact and rebuilds the derived `inv_scales`.
    fn from_persisted(persisted: PersistedScalarQuantizer) -> Result<Self, crate::error::Error> {
        let PersistedScalarQuantizer {
            dimension,
            min_vals,
            scales,
        } = persisted;
        if dimension == 0 {
            return Err(crate::error::Error::IndexCorrupted(
                "SQ8 quantizer has zero dimension".into(),
            ));
        }
        if min_vals.len() != dimension || scales.len() != dimension {
            return Err(crate::error::Error::IndexCorrupted(format!(
                "SQ8 quantizer shape mismatch: {} min_vals / {} scales for dimension {dimension}",
                min_vals.len(),
                scales.len()
            )));
        }
        // A zero, negative, or non-finite scale cannot come from `train`
        // (which floors the range at 255/epsilon or pins scale to 1.0) —
        // reject rather than let `1.0 / scale` poison every distance.
        if min_vals.iter().any(|v| !v.is_finite())
            || scales.iter().any(|s| !s.is_finite() || *s <= 0.0)
        {
            return Err(crate::error::Error::IndexCorrupted(
                "SQ8 quantizer contains non-finite or non-positive parameters".into(),
            ));
        }
        let inv_scales: Vec<f32> = scales.iter().map(|&s| 1.0 / s).collect();
        Ok(Self {
            min_vals,
            scales,
            inv_scales,
            dimension,
        })
    }
}

impl QuantizedVectorStore {
    /// Creates a new quantized vector store.
    #[must_use]
    pub fn new(quantizer: Arc<ScalarQuantizer>, capacity: usize) -> Self {
        let dimension = quantizer.dimension;
        Self {
            quantizer,
            data: Vec::with_capacity(capacity * dimension),
            count: 0,
        }
    }

    /// Adds a vector to the store (quantizes it first).
    pub fn push(&mut self, vector: &[f32]) {
        let quantized = self.quantizer.quantize(vector);
        self.data.extend(quantized.data);
        self.count += 1;
    }

    /// Gets a quantized vector by index.
    #[must_use]
    pub fn get(&self, index: usize) -> Option<QuantizedVector> {
        if index >= self.count {
            return None;
        }
        let start = index * self.quantizer.dimension;
        let end = start + self.quantizer.dimension;
        Some(QuantizedVector {
            data: self.data[start..end].to_vec(),
        })
    }

    /// Gets raw slice for a quantized vector (zero-copy).
    #[must_use]
    pub fn get_slice(&self, index: usize) -> Option<&[u8]> {
        if index >= self.count {
            return None;
        }
        let start = index * self.quantizer.dimension;
        let end = start + self.quantizer.dimension;
        Some(&self.data[start..end])
    }

    /// Returns the number of vectors.
    #[must_use]
    pub fn len(&self) -> usize {
        self.count
    }

    /// Returns true if empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Returns reference to quantizer.
    #[must_use]
    pub fn quantizer(&self) -> &ScalarQuantizer {
        &self.quantizer
    }
}
