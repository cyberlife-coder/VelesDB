//! HNSW (Hierarchical Navigable Small World) index implementation.
//!
//! This module provides a high-performance approximate nearest neighbor
//! search index based on the HNSW algorithm.
//!
//! # Quality Profiles
//!
//! The index supports different quality profiles for search:
//! - `Fast`: `ef_search=64`, ~90% recall, lowest latency
//! - `Balanced`: `ef_search=128`, ~95% recall, good tradeoff (default)
//! - `Accurate`: `ef_search=256`, ~99% recall, best quality
//!
//! # Recommended Parameters by Vector Dimension
//!
//! | Dimension   | M     | ef_construction | ef_search |
//! |-------------|-------|-----------------|-----------|
//! | d ≤ 256     | 12-16 | 100-200         | 64-128    |
//! | 256 < d ≤768| 16-24 | 200-400         | 128-256   |
//! | d > 768     | 24-32 | 300-600         | 256-512   |

use crate::distance::DistanceMetric;
use crate::index::VectorIndex;
use hnsw_rs::api::AnnT;
use hnsw_rs::hnswio::HnswIo;
use hnsw_rs::prelude::*;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// HNSW index parameters for tuning performance and recall.
///
/// Use [`HnswParams::auto`] for automatic tuning based on vector dimension,
/// or create custom parameters for specific workloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HnswParams {
    /// Number of bi-directional links per node (M parameter).
    /// Higher = better recall, more memory, slower insert.
    pub max_connections: usize,
    /// Size of dynamic candidate list during construction.
    /// Higher = better recall, slower indexing.
    pub ef_construction: usize,
    /// Initial capacity (grows automatically if exceeded).
    pub max_elements: usize,
}

impl Default for HnswParams {
    fn default() -> Self {
        Self::auto(768) // Default for common embedding dimension
    }
}

impl HnswParams {
    /// Creates optimized parameters based on vector dimension.
    ///
    /// # Recommendations
    ///
    /// | Dimension   | M     | ef_construction | Recall Target |
    /// |-------------|-------|-----------------|---------------|
    /// | d ≤ 256     | 16    | 200             | ≥95%          |
    /// | 256 < d ≤768| 24    | 400             | ≥95%          |
    /// | d > 768     | 32    | 500             | ≥95%          |
    #[must_use]
    pub fn auto(dimension: usize) -> Self {
        match dimension {
            0..=256 => Self {
                max_connections: 16,
                ef_construction: 200,
                max_elements: 100_000,
            },
            257..=768 => Self {
                max_connections: 24,
                ef_construction: 400,
                max_elements: 100_000,
            },
            _ => Self {
                max_connections: 32,
                ef_construction: 500,
                max_elements: 100_000,
            },
        }
    }

    /// Creates parameters optimized for high recall (≥99%).
    ///
    /// Uses higher M and `ef_construction` at the cost of more memory and slower indexing.
    #[must_use]
    pub fn high_recall(dimension: usize) -> Self {
        let base = Self::auto(dimension);
        Self {
            max_connections: base.max_connections + 8,
            ef_construction: base.ef_construction + 200,
            ..base
        }
    }

    /// Creates parameters optimized for fast indexing.
    ///
    /// Uses lower M and `ef_construction` for faster inserts, with slightly lower recall.
    #[must_use]
    pub fn fast_indexing(dimension: usize) -> Self {
        let base = Self::auto(dimension);
        Self {
            max_connections: (base.max_connections / 2).max(8),
            ef_construction: base.ef_construction / 2,
            ..base
        }
    }

    /// Creates custom parameters.
    #[must_use]
    pub const fn custom(
        max_connections: usize,
        ef_construction: usize,
        max_elements: usize,
    ) -> Self {
        Self {
            max_connections,
            ef_construction,
            max_elements,
        }
    }
}

/// Search quality profile controlling the recall/latency tradeoff.
///
/// Higher quality = better recall but slower search.
/// With typical index sizes (<1M vectors), all profiles stay well under 10ms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SearchQuality {
    /// Fast search with `ef_search=64`. ~90% recall, lowest latency.
    Fast,
    /// Balanced search with `ef_search=128`. ~95% recall, good tradeoff.
    #[default]
    Balanced,
    /// Accurate search with `ef_search=256`. ~99% recall, best quality.
    Accurate,
    /// Custom `ef_search` value for fine-tuning.
    Custom(usize),
}

impl SearchQuality {
    /// Returns the `ef_search` value for this quality profile.
    #[must_use]
    pub fn ef_search(&self, k: usize) -> usize {
        match self {
            Self::Fast => 64.max(k * 2),
            Self::Balanced => 128.max(k * 4),
            Self::Accurate => 256.max(k * 8),
            Self::Custom(ef) => (*ef).max(k),
        }
    }
}

/// HNSW index for efficient approximate nearest neighbor search.
///
/// # Example
///
/// ```rust,ignore
/// use velesdb_core::index::HnswIndex;
/// use velesdb_core::DistanceMetric;
///
/// let index = HnswIndex::new(768, DistanceMetric::Cosine);
/// index.insert(1, &vec![0.1; 768]);
/// let results = index.search(&vec![0.1; 768], 10);
/// ```
pub struct HnswIndex {
    /// Vector dimension
    dimension: usize,
    /// Distance metric
    metric: DistanceMetric,
    /// Internal HNSW index (type-erased for flexibility)
    inner: RwLock<HnswInner>,
    /// Mapping from external IDs to internal indices
    id_to_idx: RwLock<HashMap<u64, usize>>,
    /// Mapping from internal indices to external IDs
    idx_to_id: RwLock<HashMap<usize, u64>>,
    /// Next available internal index
    next_idx: RwLock<usize>,
}

/// Internal HNSW index wrapper to handle different distance metrics.
enum HnswInner {
    Cosine(Hnsw<'static, f32, DistCosine>),
    Euclidean(Hnsw<'static, f32, DistL2>),
    DotProduct(Hnsw<'static, f32, DistDot>),
}

impl HnswIndex {
    /// Creates a new HNSW index with auto-tuned parameters based on dimension.
    ///
    /// # Arguments
    ///
    /// * `dimension` - The dimension of vectors to index
    /// * `metric` - The distance metric to use for similarity calculations
    ///
    /// # Auto-tuning
    ///
    /// Parameters are automatically optimized for the given dimension:
    /// - d ≤ 256: `M=16`, `ef_construction=200`
    /// - 256 < d ≤ 768: `M=24`, `ef_construction=400`
    /// - d > 768: `M=32`, `ef_construction=500`
    ///
    /// Use [`HnswIndex::with_params`] for manual control.
    #[must_use]
    pub fn new(dimension: usize, metric: DistanceMetric) -> Self {
        Self::with_params(dimension, metric, HnswParams::auto(dimension))
    }

    /// Creates a new HNSW index with custom parameters.
    ///
    /// # Arguments
    ///
    /// * `dimension` - The dimension of vectors to index
    /// * `metric` - The distance metric to use for similarity calculations
    /// * `params` - Custom HNSW parameters
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use velesdb_core::{HnswIndex, HnswParams, DistanceMetric};
    ///
    /// // High recall configuration
    /// let params = HnswParams::high_recall(768);
    /// let index = HnswIndex::with_params(768, DistanceMetric::Cosine, params);
    ///
    /// // Custom configuration
    /// let params = HnswParams::custom(48, 600, 1_000_000);
    /// let index = HnswIndex::with_params(1536, DistanceMetric::Cosine, params);
    /// ```
    #[must_use]
    pub fn with_params(dimension: usize, metric: DistanceMetric, params: HnswParams) -> Self {
        let inner = match metric {
            DistanceMetric::Cosine => HnswInner::Cosine(Hnsw::new(
                params.max_connections,
                params.max_elements,
                16,
                params.ef_construction,
                DistCosine,
            )),
            DistanceMetric::Euclidean => HnswInner::Euclidean(Hnsw::new(
                params.max_connections,
                params.max_elements,
                16,
                params.ef_construction,
                DistL2,
            )),
            DistanceMetric::DotProduct => HnswInner::DotProduct(Hnsw::new(
                params.max_connections,
                params.max_elements,
                16,
                params.ef_construction,
                DistDot,
            )),
        };

        Self {
            dimension,
            metric,
            inner: RwLock::new(inner),
            id_to_idx: RwLock::new(HashMap::new()),
            idx_to_id: RwLock::new(HashMap::new()),
            next_idx: RwLock::new(0),
        }
    }

    /// Saves the HNSW index and ID mappings to the specified directory.
    ///
    /// # Errors
    ///
    /// Returns an error if saving fails.
    pub fn save<P: AsRef<std::path::Path>>(&self, path: P) -> std::io::Result<()> {
        let path = path.as_ref();
        std::fs::create_dir_all(path)?;

        let basename = "hnsw_index";

        // 1. Save HNSW graph
        let inner = self.inner.read();
        match &*inner {
            HnswInner::Cosine(hnsw) => {
                hnsw.file_dump(path, basename)
                    .map_err(std::io::Error::other)?;
            }
            HnswInner::Euclidean(hnsw) => {
                hnsw.file_dump(path, basename)
                    .map_err(std::io::Error::other)?;
            }
            HnswInner::DotProduct(hnsw) => {
                hnsw.file_dump(path, basename)
                    .map_err(std::io::Error::other)?;
            }
        }

        // 2. Save Mappings
        let mappings_path = path.join("id_mappings.bin");
        let file = std::fs::File::create(mappings_path)?;
        let writer = std::io::BufWriter::new(file);

        let id_to_idx = self.id_to_idx.read();
        let idx_to_id = self.idx_to_id.read();
        let next_idx = *self.next_idx.read();

        // Serialize as a tuple of references to avoid copying
        bincode::serialize_into(writer, &(&*id_to_idx, &*idx_to_id, next_idx))
            .map_err(std::io::Error::other)?;

        Ok(())
    }

    /// Loads the HNSW index and ID mappings from the specified directory.
    ///
    /// # Errors
    ///
    /// Returns an error if loading fails.
    pub fn load<P: AsRef<std::path::Path>>(
        path: P,
        dimension: usize,
        metric: DistanceMetric,
    ) -> std::io::Result<Self> {
        let path = path.as_ref();
        let basename = "hnsw_index";

        // Check mappings file (hnsw files checked by loader)
        let mappings_path = path.join("id_mappings.bin");
        if !mappings_path.exists() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "ID mappings file not found",
            ));
        }

        // 1. Load HNSW graph
        // We box and leak the loader to satisfy the 'static lifetime requirement of HnswIndex.
        // HnswIo holds the mmap if used (we don't use it yet), but even without mmap,
        // the load_hnsw signature enforces borrowing from the loader.
        let io = Box::new(HnswIo::new(path, basename));
        let io_ref: &'static mut HnswIo = Box::leak(io);

        let inner = match metric {
            DistanceMetric::Cosine => {
                let hnsw = io_ref
                    .load_hnsw::<f32, DistCosine>()
                    .map_err(std::io::Error::other)?;
                HnswInner::Cosine(hnsw)
            }
            DistanceMetric::Euclidean => {
                let hnsw = io_ref
                    .load_hnsw::<f32, DistL2>()
                    .map_err(std::io::Error::other)?;
                HnswInner::Euclidean(hnsw)
            }
            DistanceMetric::DotProduct => {
                let hnsw = io_ref
                    .load_hnsw::<f32, DistDot>()
                    .map_err(std::io::Error::other)?;
                HnswInner::DotProduct(hnsw)
            }
        };

        // 2. Load Mappings
        let file = std::fs::File::open(mappings_path)?;
        let reader = std::io::BufReader::new(file);
        let (id_to_idx, idx_to_id, next_idx): (HashMap<u64, usize>, HashMap<usize, u64>, usize) =
            bincode::deserialize_from(reader)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        Ok(Self {
            dimension,
            metric,
            inner: RwLock::new(inner),
            id_to_idx: RwLock::new(id_to_idx),
            idx_to_id: RwLock::new(idx_to_id),
            next_idx: RwLock::new(next_idx),
        })
    }

    /// Searches for the k nearest neighbors with a specific quality profile.
    ///
    /// # Arguments
    ///
    /// * `query` - The query vector
    /// * `k` - Number of nearest neighbors to return
    /// * `quality` - Search quality profile controlling recall/latency tradeoff
    ///
    /// # Quality Profiles
    ///
    /// - `Fast`: ~90% recall, lowest latency
    /// - `Balanced`: ~95% recall, good tradeoff (default)
    /// - `Accurate`: ~99% recall, best quality
    ///
    /// # Panics
    ///
    /// Panics if the query dimension doesn't match the index dimension.
    #[must_use]
    pub fn search_with_quality(
        &self,
        query: &[f32],
        k: usize,
        quality: SearchQuality,
    ) -> Vec<(u64, f32)> {
        assert_eq!(
            query.len(),
            self.dimension,
            "Query dimension mismatch: expected {}, got {}",
            self.dimension,
            query.len()
        );

        let ef_search = quality.ef_search(k);
        let inner = self.inner.read();
        let idx_to_id = self.idx_to_id.read();

        let mut results: Vec<(u64, f32)> = Vec::with_capacity(k);

        match &*inner {
            HnswInner::Cosine(hnsw) => {
                let neighbours = hnsw.search(query, k, ef_search);
                for n in &neighbours {
                    if let Some(&id) = idx_to_id.get(&n.d_id) {
                        // Clamp to [0,1] to handle float precision issues
                        let score = (1.0 - n.distance).clamp(0.0, 1.0);
                        results.push((id, score));
                    }
                }
            }
            HnswInner::Euclidean(hnsw) => {
                let neighbours = hnsw.search(query, k, ef_search);
                for n in &neighbours {
                    if let Some(&id) = idx_to_id.get(&n.d_id) {
                        results.push((id, n.distance));
                    }
                }
            }
            HnswInner::DotProduct(hnsw) => {
                let neighbours = hnsw.search(query, k, ef_search);
                for n in &neighbours {
                    if let Some(&id) = idx_to_id.get(&n.d_id) {
                        results.push((id, -n.distance));
                    }
                }
            }
        }

        results
    }

    /// Inserts multiple vectors in parallel using rayon.
    ///
    /// This method is optimized for bulk insertions and can significantly
    /// reduce indexing time on multi-core systems.
    ///
    /// # Arguments
    ///
    /// * `vectors` - Iterator of (id, vector) pairs to insert
    ///
    /// # Returns
    ///
    /// Number of vectors successfully inserted (duplicates are skipped).
    ///
    /// # Panics
    ///
    /// Panics if any vector has a dimension different from the index dimension.
    ///
    /// # Important
    ///
    /// After calling this method, you **must** call `set_searching_mode()`
    /// before performing any searches to ensure correct results.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let vectors: Vec<(u64, Vec<f32>)> = generate_vectors(10_000);
    /// let inserted = index.insert_batch_parallel(vectors.iter().map(|(id, v)| (*id, v.as_slice())));
    /// index.set_searching_mode();
    /// ```
    pub fn insert_batch_parallel<I>(&self, vectors: I) -> usize
    where
        I: IntoIterator<Item = (u64, Vec<f32>)>,
    {
        // Collect vectors and pre-allocate indices
        let vectors: Vec<(u64, Vec<f32>)> = vectors.into_iter().collect();

        // Pre-register all IDs and get their indices (sequential, fast)
        let mut registered: Vec<(Vec<f32>, usize)> = Vec::with_capacity(vectors.len());
        {
            let mut id_to_idx = self.id_to_idx.write();
            let mut idx_to_id = self.idx_to_id.write();
            let mut next_idx = self.next_idx.write();

            for (id, vector) in vectors {
                assert_eq!(
                    vector.len(),
                    self.dimension,
                    "Vector dimension mismatch: expected {}, got {}",
                    self.dimension,
                    vector.len()
                );

                // Skip duplicates
                if id_to_idx.contains_key(&id) {
                    continue;
                }

                let idx = *next_idx;
                *next_idx += 1;
                id_to_idx.insert(id, idx);
                idx_to_id.insert(idx, id);
                registered.push((vector, idx));
            }
        }

        let count = registered.len();

        // Prepare data for hnsw_rs parallel_insert_data: &[(&Vec<T>, usize)]
        let data_refs: Vec<(&Vec<f32>, usize)> =
            registered.iter().map(|(v, idx)| (v, *idx)).collect();

        // Parallel insertion into HNSW graph using hnsw_rs native parallel insert
        let inner = self.inner.read();
        match &*inner {
            HnswInner::Cosine(hnsw) => {
                hnsw.parallel_insert(&data_refs);
            }
            HnswInner::Euclidean(hnsw) => {
                hnsw.parallel_insert(&data_refs);
            }
            HnswInner::DotProduct(hnsw) => {
                hnsw.parallel_insert(&data_refs);
            }
        }

        count
    }

    /// Sets the index to searching mode after bulk insertions.
    ///
    /// This is required by `hnsw_rs` after parallel insertions to ensure
    /// correct search results. Call this after finishing all insertions
    /// and before performing searches.
    ///
    /// For single-threaded sequential insertions, this is typically not needed,
    /// but it's good practice to call it anyway before benchmarks.
    pub fn set_searching_mode(&self) {
        let mut inner = self.inner.write();
        match &mut *inner {
            HnswInner::Cosine(hnsw) => {
                hnsw.set_searching_mode(true);
            }
            HnswInner::Euclidean(hnsw) => {
                hnsw.set_searching_mode(true);
            }
            HnswInner::DotProduct(hnsw) => {
                hnsw.set_searching_mode(true);
            }
        }
    }
}

impl VectorIndex for HnswIndex {
    fn insert(&self, id: u64, vector: &[f32]) {
        assert_eq!(
            vector.len(),
            self.dimension,
            "Vector dimension mismatch: expected {}, got {}",
            self.dimension,
            vector.len()
        );

        // Check if ID already exists - hnsw_rs doesn't support updates!
        // Inserting the same idx twice creates duplicates/ghosts in the graph.
        let mut id_to_idx = self.id_to_idx.write();
        if id_to_idx.contains_key(&id) {
            // ID already exists - skip insertion to avoid corrupting the index.
            // Use a dedicated upsert() method if you need update semantics.
            // For now, we silently skip (production code should log this).
            return;
        }

        let mut idx_to_id = self.idx_to_id.write();
        let mut next_idx = self.next_idx.write();

        let idx = *next_idx;
        *next_idx += 1;
        id_to_idx.insert(id, idx);
        idx_to_id.insert(idx, id);

        drop(id_to_idx);
        drop(idx_to_id);
        drop(next_idx);

        // Insert into HNSW index
        let inner = self.inner.write();
        match &*inner {
            HnswInner::Cosine(hnsw) => {
                hnsw.insert((vector, idx));
            }
            HnswInner::Euclidean(hnsw) => {
                hnsw.insert((vector, idx));
            }
            HnswInner::DotProduct(hnsw) => {
                hnsw.insert((vector, idx));
            }
        }
    }

    fn search(&self, query: &[f32], k: usize) -> Vec<(u64, f32)> {
        // Use Balanced quality profile by default
        self.search_with_quality(query, k, SearchQuality::Balanced)
    }

    /// Performs a **soft delete** of the vector.
    ///
    /// # Important
    ///
    /// This removes the ID from the mappings but **does NOT remove the vector
    /// from the HNSW graph** (`hnsw_rs` doesn't support true deletion).
    /// The vector will no longer appear in search results, but memory is not freed.
    ///
    /// For workloads with many deletions, consider periodic index rebuilding
    /// to reclaim memory and maintain optimal graph structure.
    fn remove(&self, id: u64) -> bool {
        let mut id_to_idx = self.id_to_idx.write();
        let mut idx_to_id = self.idx_to_id.write();

        if let Some(idx) = id_to_idx.remove(&id) {
            idx_to_id.remove(&idx);
            // Soft delete: vector remains in HNSW graph but is excluded from results
            true
        } else {
            false
        }
    }

    fn len(&self) -> usize {
        self.id_to_idx.read().len()
    }

    fn dimension(&self) -> usize {
        self.dimension
    }

    fn metric(&self) -> DistanceMetric {
        self.metric
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // TDD Tests - Written BEFORE implementation (RED phase)
    // =========================================================================

    #[test]
    fn test_hnsw_new_creates_empty_index() {
        // Arrange & Act
        let index = HnswIndex::new(768, DistanceMetric::Cosine);

        // Assert
        assert!(index.is_empty());
        assert_eq!(index.len(), 0);
        assert_eq!(index.dimension(), 768);
        assert_eq!(index.metric(), DistanceMetric::Cosine);
    }

    #[test]
    fn test_hnsw_insert_single_vector() {
        // Arrange
        let index = HnswIndex::new(3, DistanceMetric::Cosine);
        let vector = vec![1.0, 0.0, 0.0];

        // Act
        index.insert(1, &vector);

        // Assert
        assert_eq!(index.len(), 1);
        assert!(!index.is_empty());
    }

    #[test]
    fn test_hnsw_insert_multiple_vectors() {
        // Arrange
        let index = HnswIndex::new(3, DistanceMetric::Cosine);

        // Act
        index.insert(1, &[1.0, 0.0, 0.0]);
        index.insert(2, &[0.0, 1.0, 0.0]);
        index.insert(3, &[0.0, 0.0, 1.0]);

        // Assert
        assert_eq!(index.len(), 3);
    }

    #[test]
    fn test_hnsw_search_returns_k_nearest() {
        // Arrange
        let index = HnswIndex::new(3, DistanceMetric::Cosine);
        index.insert(1, &[1.0, 0.0, 0.0]);
        index.insert(2, &[0.9, 0.1, 0.0]); // Similar to 1
        index.insert(3, &[0.0, 1.0, 0.0]); // Different

        // Act
        let results = index.search(&[1.0, 0.0, 0.0], 2);

        // Assert
        assert_eq!(results.len(), 2);
        // First result should be exact match (id=1)
        assert_eq!(results[0].0, 1);
    }

    #[test]
    fn test_hnsw_search_empty_index() {
        // Arrange
        let index = HnswIndex::new(3, DistanceMetric::Cosine);

        // Act
        let results = index.search(&[1.0, 0.0, 0.0], 10);

        // Assert
        assert!(results.is_empty());
    }

    #[test]
    fn test_hnsw_remove_existing_vector() {
        // Arrange
        let index = HnswIndex::new(3, DistanceMetric::Cosine);
        index.insert(1, &[1.0, 0.0, 0.0]);
        index.insert(2, &[0.0, 1.0, 0.0]);

        // Act
        let removed = index.remove(1);

        // Assert
        assert!(removed);
        assert_eq!(index.len(), 1);
    }

    #[test]
    fn test_hnsw_remove_nonexistent_vector() {
        // Arrange
        let index = HnswIndex::new(3, DistanceMetric::Cosine);
        index.insert(1, &[1.0, 0.0, 0.0]);

        // Act
        let removed = index.remove(999);

        // Assert
        assert!(!removed);
        assert_eq!(index.len(), 1);
    }

    #[test]
    fn test_hnsw_euclidean_metric() {
        // Arrange
        let index = HnswIndex::new(3, DistanceMetric::Euclidean);
        index.insert(1, &[0.0, 0.0, 0.0]);
        index.insert(2, &[1.0, 0.0, 0.0]); // Distance 1
        index.insert(3, &[3.0, 4.0, 0.0]); // Distance 5

        // Act
        let results = index.search(&[0.0, 0.0, 0.0], 3);

        // Assert
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].0, 1); // Closest (exact match)
    }

    #[test]
    fn test_hnsw_dot_product_metric() {
        // Arrange - Use normalized positive vectors for dot product
        // DistDot in hnsw_rs requires non-negative dot products
        let index = HnswIndex::new(3, DistanceMetric::DotProduct);

        // Insert vectors with distinct dot products when queried with [1,0,0]
        index.insert(1, &[1.0, 0.0, 0.0]); // dot=1.0 with query
        index.insert(2, &[0.5, 0.5, 0.5]); // dot=0.5 with query
        index.insert(3, &[0.1, 0.1, 0.1]); // dot=0.1 with query

        // Act - Query with unit vector x
        let query = [1.0, 0.0, 0.0];
        let results = index.search(&query, 3);

        // Assert
        assert_eq!(results.len(), 3);
        // All three IDs should be present in results
        let ids: Vec<u64> = results.iter().map(|(id, _)| *id).collect();
        assert!(ids.contains(&1));
        assert!(ids.contains(&2));
        assert!(ids.contains(&3));
    }

    #[test]
    #[should_panic(expected = "Vector dimension mismatch")]
    fn test_hnsw_insert_wrong_dimension_panics() {
        // Arrange
        let index = HnswIndex::new(3, DistanceMetric::Cosine);

        // Act - should panic
        index.insert(1, &[1.0, 0.0]); // Wrong dimension
    }

    #[test]
    #[should_panic(expected = "Query dimension mismatch")]
    fn test_hnsw_search_wrong_dimension_panics() {
        // Arrange
        let index = HnswIndex::new(3, DistanceMetric::Cosine);
        index.insert(1, &[1.0, 0.0, 0.0]);

        // Act - should panic
        let _ = index.search(&[1.0, 0.0], 10); // Wrong dimension
    }

    #[test]
    fn test_hnsw_duplicate_insert_is_skipped() {
        // Arrange
        let index = HnswIndex::new(3, DistanceMetric::Cosine);
        index.insert(1, &[1.0, 0.0, 0.0]);

        // Act - Insert with same ID should be SKIPPED (not updated)
        // hnsw_rs doesn't support updates; inserting same idx creates ghosts
        index.insert(1, &[0.0, 1.0, 0.0]);

        // Assert
        assert_eq!(index.len(), 1); // Still only one entry

        // Verify the ORIGINAL vector is still there (not updated)
        let results = index.search(&[1.0, 0.0, 0.0], 1);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, 1);
        // Score should be ~1.0 (exact match with original vector)
        assert!(
            results[0].1 > 0.99,
            "Original vector should still be indexed"
        );
    }

    #[test]
    fn test_hnsw_thread_safety() {
        use std::sync::Arc;
        use std::thread;

        // Arrange
        let index = Arc::new(HnswIndex::new(3, DistanceMetric::Cosine));
        let mut handles = vec![];

        // Act - Insert from multiple threads (unique IDs)
        for i in 0..10 {
            let index_clone = Arc::clone(&index);
            handles.push(thread::spawn(move || {
                #[allow(clippy::cast_precision_loss)]
                index_clone.insert(i, &[i as f32, 0.0, 0.0]);
            }));
        }

        for handle in handles {
            handle.join().expect("Thread panicked");
        }

        // Set searching mode after parallel insertions (required by hnsw_rs)
        index.set_searching_mode();

        // Assert
        assert_eq!(index.len(), 10);
    }

    #[test]
    fn test_hnsw_persistence() {
        use tempfile::tempdir;

        // Arrange
        let dir = tempdir().unwrap();
        let index = HnswIndex::new(3, DistanceMetric::Cosine);
        index.insert(1, &[1.0, 0.0, 0.0]);
        index.insert(2, &[0.0, 1.0, 0.0]);

        // Act - Save
        index.save(dir.path()).unwrap();

        // Act - Load
        let loaded_index = HnswIndex::load(dir.path(), 3, DistanceMetric::Cosine).unwrap();

        // Assert
        assert_eq!(loaded_index.len(), 2);
        assert_eq!(loaded_index.dimension(), 3);
        assert_eq!(loaded_index.metric(), DistanceMetric::Cosine);

        // Verify search works on loaded index
        let results = loaded_index.search(&[1.0, 0.0, 0.0], 1);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, 1);
    }

    #[test]
    fn test_hnsw_insert_batch_parallel() {
        // Arrange
        let index = HnswIndex::new(3, DistanceMetric::Cosine);
        let vectors: Vec<(u64, Vec<f32>)> = vec![
            (1, vec![1.0, 0.0, 0.0]),
            (2, vec![0.0, 1.0, 0.0]),
            (3, vec![0.0, 0.0, 1.0]),
            (4, vec![0.5, 0.5, 0.0]),
            (5, vec![0.5, 0.0, 0.5]),
        ];

        // Act
        let inserted = index.insert_batch_parallel(vectors);
        index.set_searching_mode();

        // Assert
        assert_eq!(inserted, 5);
        assert_eq!(index.len(), 5);

        // Verify search works
        let results = index.search(&[1.0, 0.0, 0.0], 3);
        assert_eq!(results.len(), 3);
        // ID 1 should be the closest match
        assert_eq!(results[0].0, 1);
    }

    #[test]
    fn test_hnsw_insert_batch_parallel_skips_duplicates() {
        // Arrange
        let index = HnswIndex::new(3, DistanceMetric::Cosine);

        // Insert one vector first
        index.insert(1, &[1.0, 0.0, 0.0]);

        // Act - Try to insert batch with duplicate ID
        let vectors: Vec<(u64, Vec<f32>)> = vec![
            (1, vec![0.0, 1.0, 0.0]), // Duplicate ID
            (2, vec![0.0, 0.0, 1.0]), // New
        ];
        let inserted = index.insert_batch_parallel(vectors);
        index.set_searching_mode();

        // Assert - Only 1 new vector should be inserted
        assert_eq!(inserted, 1);
        assert_eq!(index.len(), 2);
    }

    // =========================================================================
    // HnswParams Auto-tuning Tests (WIS-12)
    // =========================================================================

    #[test]
    fn test_hnsw_params_auto_small_dimension() {
        let params = HnswParams::auto(128);
        assert_eq!(params.max_connections, 16);
        assert_eq!(params.ef_construction, 200);
    }

    #[test]
    fn test_hnsw_params_auto_medium_dimension() {
        let params = HnswParams::auto(768);
        assert_eq!(params.max_connections, 24);
        assert_eq!(params.ef_construction, 400);
    }

    #[test]
    fn test_hnsw_params_auto_large_dimension() {
        let params = HnswParams::auto(1536);
        assert_eq!(params.max_connections, 32);
        assert_eq!(params.ef_construction, 500);
    }

    #[test]
    fn test_hnsw_params_high_recall() {
        let params = HnswParams::high_recall(768);
        let base = HnswParams::auto(768);
        assert_eq!(params.max_connections, base.max_connections + 8);
        assert_eq!(params.ef_construction, base.ef_construction + 200);
    }

    #[test]
    fn test_hnsw_params_fast_indexing() {
        let params = HnswParams::fast_indexing(768);
        let base = HnswParams::auto(768);
        assert_eq!(params.max_connections, base.max_connections / 2);
        assert_eq!(params.ef_construction, base.ef_construction / 2);
    }

    #[test]
    fn test_hnsw_with_params() {
        let params = HnswParams::custom(48, 600, 500_000);
        let index = HnswIndex::with_params(1536, DistanceMetric::Cosine, params);

        assert_eq!(index.dimension(), 1536);
        assert!(index.is_empty());
    }

    #[test]
    fn test_hnsw_params_boundary_256() {
        // Test boundary at 256
        let params_256 = HnswParams::auto(256);
        let params_257 = HnswParams::auto(257);

        assert_eq!(params_256.max_connections, 16);
        assert_eq!(params_257.max_connections, 24);
    }

    #[test]
    fn test_hnsw_params_boundary_768() {
        // Test boundary at 768
        let params_768 = HnswParams::auto(768);
        let params_769 = HnswParams::auto(769);

        assert_eq!(params_768.max_connections, 24);
        assert_eq!(params_769.max_connections, 32);
    }
}
