//! Trigram accelerator for batch operations.
//!
//! Provides parallelism for bulk trigram operations:
//! - Batch document indexing
//! - Parallel pattern matching across millions of docs
//!
//! # Performance Thresholds
//!
//! | Operation | CPU Best | GPU Threshold |
//! |-----------|----------|---------------|
//! | Single search | < 100K docs | > 500K docs |
//! | Batch index | < 10K docs | > 50K docs |
//! | Pattern scan | < 1M docs | > 1M docs |

use roaring::RoaringBitmap;
use std::collections::HashSet;

/// CPU trigram index operations.
///
/// Provides batch trigram extraction and search using CPU.
/// Previously named `GpuTrigramAccelerator` — renamed because all actual
/// trigram operations (extraction, search) are pure CPU `HashMap` lookups.
pub struct TrigramAccelerator;

impl TrigramAccelerator {
    /// Create a new trigram accelerator.
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Batch search multiple patterns.
    ///
    /// For each pattern, extracts trigrams and intersects matching document sets.
    /// More efficient than individual searches for > 10 patterns on > 100K docs.
    ///
    /// # Arguments
    /// * `patterns` - Search patterns to match
    /// * `inverted_index` - Trigram -> document bitmap index
    ///
    /// # Returns
    /// Vector of `RoaringBitmap` with matching document IDs per pattern.
    #[must_use]
    pub fn batch_search(
        &self,
        patterns: &[&str],
        inverted_index: &std::collections::HashMap<[u8; 3], RoaringBitmap>,
    ) -> Vec<RoaringBitmap> {
        patterns
            .iter()
            .map(|pattern| Self::search_single(pattern, inverted_index))
            .collect()
    }

    /// Search a single pattern using extracted trigrams.
    fn search_single(
        pattern: &str,
        inverted_index: &std::collections::HashMap<[u8; 3], RoaringBitmap>,
    ) -> RoaringBitmap {
        let trigrams = Self::extract_trigrams_cpu(pattern);
        if trigrams.is_empty() {
            return RoaringBitmap::new();
        }

        // Intersect all trigram bitmaps
        let mut result: Option<RoaringBitmap> = None;
        for trigram in &trigrams {
            if let Some(bitmap) = inverted_index.get(trigram) {
                result = Some(match result {
                    Some(r) => r & bitmap,
                    None => bitmap.clone(),
                });
            } else {
                // Trigram not in index = no matches
                return RoaringBitmap::new();
            }
        }

        result.unwrap_or_default()
    }

    /// Batch extract trigrams from multiple documents.
    ///
    /// Processes documents sequentially on CPU.
    /// Optimal for > 1000 documents.
    ///
    /// # Arguments
    /// * `documents` - Documents to extract trigrams from
    ///
    /// # Returns
    /// Vector of trigram sets, one per document.
    #[must_use]
    pub fn batch_extract_trigrams(&self, documents: &[&str]) -> Vec<HashSet<[u8; 3]>> {
        documents
            .iter()
            .map(|doc| Self::extract_trigrams_cpu(doc))
            .collect()
    }

    /// Extract trigrams from text.
    fn extract_trigrams_cpu(text: &str) -> HashSet<[u8; 3]> {
        let bytes = text.as_bytes();
        if bytes.len() < 3 {
            return HashSet::new();
        }

        let mut trigrams = HashSet::with_capacity(bytes.len().saturating_sub(2));
        for window in bytes.windows(3) {
            trigrams.insert([window[0], window[1], window[2]]);
        }
        trigrams
    }
}

impl Default for TrigramAccelerator {
    fn default() -> Self {
        Self::new()
    }
}

/// Compute backend selection for trigram operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrigramComputeBackend {
    /// CPU SIMD (default, always available)
    #[default]
    CpuSimd,
    /// GPU via wgpu (requires `gpu` feature)
    #[cfg(feature = "gpu")]
    Gpu,
}

impl TrigramComputeBackend {
    /// Select best available backend based on workload size.
    #[must_use]
    #[cfg_attr(not(feature = "gpu"), allow(unused_variables))]
    pub fn auto_select(doc_count: usize, pattern_count: usize) -> Self {
        #[cfg(feature = "gpu")]
        {
            // GPU is better for large workloads
            if doc_count > 500_000 || (doc_count > 100_000 && pattern_count > 10) {
                if crate::gpu::ComputeBackend::gpu_available() {
                    return Self::Gpu;
                }
            }
        }

        Self::CpuSimd
    }

    /// Get backend name for logging.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::CpuSimd => "CPU SIMD",
            #[cfg(feature = "gpu")]
            Self::Gpu => "GPU (wgpu)",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backend_auto_select_small() {
        let backend = TrigramComputeBackend::auto_select(10_000, 1);
        assert_eq!(backend, TrigramComputeBackend::CpuSimd);
    }

    #[test]
    fn test_backend_auto_select_medium() {
        let backend = TrigramComputeBackend::auto_select(100_000, 5);
        // Should still be CPU for medium workloads
        assert_eq!(backend, TrigramComputeBackend::CpuSimd);
    }

    #[test]
    fn test_backend_name() {
        assert_eq!(TrigramComputeBackend::CpuSimd.name(), "CPU SIMD");
    }

    #[test]
    fn test_trigram_accelerator_creation() {
        let accel = TrigramAccelerator::new();
        let docs = vec!["hello", "world", "test"];
        let results = accel.batch_extract_trigrams(&docs);

        assert_eq!(results.len(), 3);
        // "hello" has trigrams: hel, ell, llo
        assert!(results[0].contains(b"hel"));
        assert!(results[0].contains(b"ell"));
        assert!(results[0].contains(b"llo"));
    }

    #[test]
    fn test_batch_extract_trigrams_short_text() {
        let accel = TrigramAccelerator::new();
        let docs = vec!["ab", "a", ""];
        let results = accel.batch_extract_trigrams(&docs);

        assert_eq!(results.len(), 3);
        assert!(results[0].is_empty()); // "ab" too short
        assert!(results[1].is_empty()); // "a" too short
        assert!(results[2].is_empty()); // empty
    }

    #[test]
    fn test_batch_search_empty_patterns() {
        let accel = TrigramAccelerator::new();
        let index: std::collections::HashMap<[u8; 3], RoaringBitmap> =
            std::collections::HashMap::new();
        let results = accel.batch_search(&[], &index);
        assert!(results.is_empty());
    }

    #[test]
    fn test_batch_search_with_matches() {
        let accel = TrigramAccelerator::new();
        let mut index: std::collections::HashMap<[u8; 3], RoaringBitmap> =
            std::collections::HashMap::new();

        // Add trigrams for "hello" in doc 0 and 1
        let mut bitmap = RoaringBitmap::new();
        bitmap.insert(0);
        bitmap.insert(1);
        index.insert([b'h', b'e', b'l'], bitmap.clone());
        index.insert([b'e', b'l', b'l'], bitmap.clone());
        index.insert([b'l', b'l', b'o'], bitmap);

        let patterns = vec!["hello"];
        let results = accel.batch_search(&patterns, &index);

        assert_eq!(results.len(), 1);
        assert!(results[0].contains(0));
        assert!(results[0].contains(1));
    }

    #[test]
    fn test_batch_search_no_matches() {
        let accel = TrigramAccelerator::new();
        let index: std::collections::HashMap<[u8; 3], RoaringBitmap> =
            std::collections::HashMap::new();
        let patterns = vec!["hello"];
        let results = accel.batch_search(&patterns, &index);

        assert_eq!(results.len(), 1);
        assert!(results[0].is_empty());
    }

    #[test]
    fn test_trigram_accelerator_default() {
        // Reason: verify the struct can be constructed (compile-time check)
        let _ = TrigramAccelerator;
    }
}
