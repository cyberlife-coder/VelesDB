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
use std::mem::ManuallyDrop;

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

    /// Creates parameters optimized for high recall (≥97%).
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

    /// Creates parameters optimized for maximum recall (≥99%).
    ///
    /// Uses aggressive M and `ef_construction` values. Best for quality-critical applications.
    /// Trade-off: 2-3x more memory, 3-5x slower indexing.
    #[must_use]
    pub fn max_recall(dimension: usize) -> Self {
        match dimension {
            0..=256 => Self {
                max_connections: 32,
                ef_construction: 500,
                max_elements: 100_000,
            },
            257..=768 => Self {
                max_connections: 48,
                ef_construction: 800,
                max_elements: 100_000,
            },
            _ => Self {
                max_connections: 64,
                ef_construction: 1000,
                max_elements: 100_000,
            },
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
    /// Fast search with `ef_search=64`. ~85% recall, lowest latency.
    Fast,
    /// Balanced search with `ef_search=128`. ~92% recall, good tradeoff.
    #[default]
    Balanced,
    /// Accurate search with `ef_search=256`. ~97% recall, best quality.
    Accurate,
    /// High recall search with `ef_search=512`. ~99%+ recall, highest quality.
    HighRecall,
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
            Self::HighRecall => 512.max(k * 16),
            Self::Custom(ef) => (*ef).max(k),
        }
    }
}

/// ID mappings for HNSW index.
///
/// Groups all mapping-related data under a single lock to reduce
/// lock contention during parallel insertions (WIS-9).
#[derive(Debug, Clone, Default)]
struct HnswMappings {
    /// Mapping from external IDs to internal indices.
    id_to_idx: HashMap<u64, usize>,
    /// Mapping from internal indices to external IDs.
    idx_to_id: HashMap<usize, u64>,
    /// Next available internal index.
    next_idx: usize,
}

impl HnswMappings {
    /// Creates new empty mappings.
    fn new() -> Self {
        Self::default()
    }

    /// Registers an ID and returns its internal index.
    /// Returns `None` if the ID already exists.
    fn register(&mut self, id: u64) -> Option<usize> {
        if self.id_to_idx.contains_key(&id) {
            return None;
        }
        let idx = self.next_idx;
        self.next_idx += 1;
        self.id_to_idx.insert(id, idx);
        self.idx_to_id.insert(idx, id);
        Some(idx)
    }

    /// Removes an ID and returns its internal index if it existed.
    fn remove(&mut self, id: u64) -> Option<usize> {
        if let Some(idx) = self.id_to_idx.remove(&id) {
            self.idx_to_id.remove(&idx);
            Some(idx)
        } else {
            None
        }
    }

    /// Gets the internal index for an external ID.
    fn get_idx(&self, id: u64) -> Option<usize> {
        self.id_to_idx.get(&id).copied()
    }

    /// Gets the external ID for an internal index.
    fn get_id(&self, idx: usize) -> Option<u64> {
        self.idx_to_id.get(&idx).copied()
    }

    /// Returns the number of registered IDs.
    fn len(&self) -> usize {
        self.id_to_idx.len()
    }

    /// Returns true if no IDs are registered.
    #[allow(dead_code)]
    fn is_empty(&self) -> bool {
        self.id_to_idx.is_empty()
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
    /// Wrapped in `ManuallyDrop` to control drop order - must be dropped BEFORE `io_holder`
    inner: RwLock<ManuallyDrop<HnswInner>>,
    /// ID mappings (external ID <-> internal index) under single lock (WIS-9)
    mappings: RwLock<HnswMappings>,
    /// Vector storage for SIMD re-ranking (idx -> vector)
    vectors: RwLock<HashMap<usize, Vec<f32>>>,
    /// Holds the `HnswIo` for loaded indices to prevent memory leak.
    /// The Hnsw borrows from this, so it must be dropped AFTER inner.
    /// Only Some when index was loaded from disk.
    /// Note: This field is intentionally not read - it exists purely for lifetime management.
    #[allow(dead_code)]
    io_holder: Option<Box<HnswIo>>,
}

/// Internal HNSW index wrapper to handle different distance metrics.
enum HnswInner {
    Cosine(Hnsw<'static, f32, DistCosine>),
    Euclidean(Hnsw<'static, f32, DistL2>),
    DotProduct(Hnsw<'static, f32, DistDot>),
    /// Hamming uses L2 internally for graph construction, actual distance computed during re-ranking
    Hamming(Hnsw<'static, f32, DistL2>),
    /// Jaccard uses L2 internally for graph construction, actual similarity computed during re-ranking
    Jaccard(Hnsw<'static, f32, DistL2>),
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
            // Hamming/Jaccard use L2 for graph construction, actual distance computed during re-ranking
            DistanceMetric::Hamming => HnswInner::Hamming(Hnsw::new(
                params.max_connections,
                params.max_elements,
                16,
                params.ef_construction,
                DistL2,
            )),
            DistanceMetric::Jaccard => HnswInner::Jaccard(Hnsw::new(
                params.max_connections,
                params.max_elements,
                16,
                params.ef_construction,
                DistL2,
            )),
        };

        Self {
            dimension,
            metric,
            inner: RwLock::new(ManuallyDrop::new(inner)),
            mappings: RwLock::new(HnswMappings::new()),
            vectors: RwLock::new(HashMap::new()),
            io_holder: None, // No io_holder for newly created indices
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
        match &**inner {
            HnswInner::Cosine(hnsw) => {
                hnsw.file_dump(path, basename)
                    .map_err(std::io::Error::other)?;
            }
            HnswInner::Euclidean(hnsw) | HnswInner::Hamming(hnsw) | HnswInner::Jaccard(hnsw) => {
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

        let mappings = self.mappings.read();

        // Serialize as a tuple to maintain backward compatibility
        bincode::serialize_into(
            writer,
            &(&mappings.id_to_idx, &mappings.idx_to_id, mappings.next_idx),
        )
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
        // Store HnswIo in a Box that we'll keep in the struct.
        // We use unsafe to extend the lifetime to 'static because:
        // - The HnswIo will live as long as the HnswIndex
        // - We implement Drop to ensure proper cleanup order
        let mut io_holder = Box::new(HnswIo::new(path, basename));

        // SAFETY: We're extending the lifetime to 'static, but we guarantee that:
        // 1. io_holder lives in the struct alongside the Hnsw
        // 2. Drop impl ensures inner (which borrows from io_holder) is dropped first
        // 3. io_holder is dropped after inner in the Drop impl
        let io_ref: &'static mut HnswIo =
            unsafe { &mut *std::ptr::from_mut::<HnswIo>(io_holder.as_mut()) };

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
            DistanceMetric::Hamming => {
                let hnsw = io_ref
                    .load_hnsw::<f32, DistL2>()
                    .map_err(std::io::Error::other)?;
                HnswInner::Hamming(hnsw)
            }
            DistanceMetric::Jaccard => {
                let hnsw = io_ref
                    .load_hnsw::<f32, DistL2>()
                    .map_err(std::io::Error::other)?;
                HnswInner::Jaccard(hnsw)
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
            inner: RwLock::new(ManuallyDrop::new(inner)),
            mappings: RwLock::new(HnswMappings {
                id_to_idx,
                idx_to_id,
                next_idx,
            }),
            vectors: RwLock::new(HashMap::new()), // Note: vectors not restored from disk
            io_holder: Some(io_holder),           // Store to prevent memory leak
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
        let mappings = self.mappings.read();

        let mut results: Vec<(u64, f32)> = Vec::with_capacity(k);

        match &**inner {
            HnswInner::Cosine(hnsw) => {
                let neighbours = hnsw.search(query, k, ef_search);
                for n in &neighbours {
                    if let Some(id) = mappings.get_id(n.d_id) {
                        // Clamp to [0,1] to handle float precision issues
                        let score = (1.0 - n.distance).clamp(0.0, 1.0);
                        results.push((id, score));
                    }
                }
            }
            HnswInner::Euclidean(hnsw) | HnswInner::Hamming(hnsw) | HnswInner::Jaccard(hnsw) => {
                let neighbours = hnsw.search(query, k, ef_search);
                for n in &neighbours {
                    if let Some(id) = mappings.get_id(n.d_id) {
                        results.push((id, n.distance));
                    }
                }
            }
            HnswInner::DotProduct(hnsw) => {
                let neighbours = hnsw.search(query, k, ef_search);
                for n in &neighbours {
                    if let Some(id) = mappings.get_id(n.d_id) {
                        results.push((id, -n.distance));
                    }
                }
            }
        }

        results
    }

    /// Searches with SIMD-based re-ranking for improved precision.
    ///
    /// This method first retrieves `rerank_k` candidates using the HNSW index,
    /// then re-ranks them using our SIMD-optimized distance functions for
    /// exact distance computation, returning the top `k` results.
    ///
    /// # Arguments
    ///
    /// * `query` - The query vector
    /// * `k` - Number of nearest neighbors to return
    /// * `rerank_k` - Number of candidates to retrieve before re-ranking (should be > k)
    ///
    /// # Returns
    ///
    /// Vector of (id, distance) tuples, sorted by similarity.
    /// For Cosine/DotProduct: higher is better (descending order).
    /// For Euclidean: lower is better (ascending order).
    ///
    /// # Panics
    ///
    /// Panics if the query dimension doesn't match the index dimension.
    #[must_use]
    pub fn search_with_rerank(&self, query: &[f32], k: usize, rerank_k: usize) -> Vec<(u64, f32)> {
        assert_eq!(
            query.len(),
            self.dimension,
            "Query dimension mismatch: expected {}, got {}",
            self.dimension,
            query.len()
        );

        // 1. Get candidates from HNSW (fast approximate search)
        let candidates = self.search_with_quality(query, rerank_k, SearchQuality::Accurate);

        if candidates.is_empty() {
            return Vec::new();
        }

        // 2. Re-rank using SIMD-optimized exact distance computation
        let inner = self.inner.read();
        let mut reranked: Vec<(u64, f32)> = candidates
            .into_iter()
            .map(|(id, _)| {
                // Get the vector for this ID and compute exact distance
                let exact_dist = self.compute_exact_distance_inner(&inner, id, query);
                (id, exact_dist)
            })
            .collect();

        // 3. Sort by distance (metric-dependent ordering)
        match self.metric {
            DistanceMetric::Cosine | DistanceMetric::DotProduct | DistanceMetric::Jaccard => {
                // Higher is better - sort descending
                reranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            }
            DistanceMetric::Euclidean | DistanceMetric::Hamming => {
                // Lower is better - sort ascending
                reranked.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
            }
        }

        // 4. Return top k
        reranked.truncate(k);
        reranked
    }

    /// Computes exact distance using SIMD-optimized functions.
    fn compute_exact_distance_inner(&self, _inner: &HnswInner, id: u64, query: &[f32]) -> f32 {
        let mappings = self.mappings.read();
        let Some(idx) = mappings.get_idx(id) else {
            return f32::MAX;
        };

        // Get vector from our storage
        let vectors = self.vectors.read();
        let Some(v) = vectors.get(&idx) else {
            return f32::MAX;
        };

        // Use our SIMD-optimized distance functions based on metric
        match self.metric {
            DistanceMetric::Cosine => crate::simd::cosine_similarity_fast(query, v),
            DistanceMetric::Euclidean => crate::simd::euclidean_distance_fast(query, v),
            DistanceMetric::DotProduct => crate::simd::dot_product_fast(query, v),
            DistanceMetric::Hamming => crate::simd::hamming_distance_fast(query, v),
            DistanceMetric::Jaccard => crate::simd::jaccard_similarity_fast(query, v),
        }
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
        // WIS-9: Single lock acquisition instead of 3 separate locks
        let mut registered: Vec<(Vec<f32>, usize)> = Vec::with_capacity(vectors.len());
        {
            let mut mappings = self.mappings.write();

            for (id, vector) in vectors {
                assert_eq!(
                    vector.len(),
                    self.dimension,
                    "Vector dimension mismatch: expected {}, got {}",
                    self.dimension,
                    vector.len()
                );

                // Skip duplicates, register returns None if ID exists
                if let Some(idx) = mappings.register(id) {
                    registered.push((vector, idx));
                }
            }
        }

        let count = registered.len();

        // Store vectors for SIMD re-ranking
        {
            let mut vectors = self.vectors.write();
            for (v, idx) in &registered {
                vectors.insert(*idx, v.clone());
            }
        }

        // Prepare data for hnsw_rs parallel_insert_data: &[(&Vec<T>, usize)]
        let data_refs: Vec<(&Vec<f32>, usize)> =
            registered.iter().map(|(v, idx)| (v, *idx)).collect();

        // Parallel insertion into HNSW graph using hnsw_rs native parallel insert
        let inner = self.inner.read();
        match &**inner {
            HnswInner::Cosine(hnsw) => {
                hnsw.parallel_insert(&data_refs);
            }
            HnswInner::Euclidean(hnsw) | HnswInner::Hamming(hnsw) | HnswInner::Jaccard(hnsw) => {
                hnsw.parallel_insert(&data_refs);
            }
            HnswInner::DotProduct(hnsw) => {
                hnsw.parallel_insert(&data_refs);
            }
        }

        count
    }

    /// Searches multiple queries in parallel using rayon.
    ///
    /// This method is optimized for batch query workloads and can significantly
    /// reduce total search time on multi-core systems.
    ///
    /// # Arguments
    ///
    /// * `queries` - Slice of query vectors
    /// * `k` - Number of nearest neighbors to return per query
    /// * `quality` - Search quality profile
    ///
    /// # Returns
    ///
    /// Vector of results, one per query, in the same order as input.
    ///
    /// # Panics
    ///
    /// Panics if any query dimension doesn't match the index dimension.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let queries: Vec<Vec<f32>> = generate_queries(100);
    /// let query_refs: Vec<&[f32]> = queries.iter().map(|q| q.as_slice()).collect();
    /// let results = index.search_batch_parallel(&query_refs, 10, SearchQuality::Balanced);
    /// ```
    #[must_use]
    pub fn search_batch_parallel(
        &self,
        queries: &[&[f32]],
        k: usize,
        quality: SearchQuality,
    ) -> Vec<Vec<(u64, f32)>> {
        use rayon::prelude::*;

        queries
            .par_iter()
            .map(|query| self.search_with_quality(query, k, quality))
            .collect()
    }

    /// Performs exact brute-force search in parallel using rayon.
    ///
    /// This method computes exact distances to all vectors in the index,
    /// guaranteeing **100% recall**. Uses all available CPU cores.
    ///
    /// # Arguments
    ///
    /// * `query` - The query vector
    /// * `k` - Number of nearest neighbors to return
    ///
    /// # Returns
    ///
    /// Vector of (id, score) tuples, sorted by similarity.
    ///
    /// # Performance
    ///
    /// - **Recall**: 100% (exact)
    /// - **Latency**: O(n/cores) where n = dataset size
    /// - **Best for**: Small datasets (<10k) or when recall is critical
    ///
    /// # Panics
    ///
    /// Panics if the query dimension doesn't match the index dimension.
    #[must_use]
    pub fn brute_force_search_parallel(&self, query: &[f32], k: usize) -> Vec<(u64, f32)> {
        use rayon::prelude::*;

        assert_eq!(
            query.len(),
            self.dimension,
            "Query dimension mismatch: expected {}, got {}",
            self.dimension,
            query.len()
        );

        let vectors = self.vectors.read();
        let mappings = self.mappings.read();

        // Compute distances in parallel
        let mut results: Vec<(u64, f32)> = vectors
            .par_iter()
            .filter_map(|(idx, vec)| {
                let id = mappings.get_id(*idx)?;
                let score = match self.metric {
                    DistanceMetric::Cosine => crate::simd::cosine_similarity_fast(query, vec),
                    DistanceMetric::Euclidean => crate::simd::euclidean_distance_fast(query, vec),
                    DistanceMetric::DotProduct => crate::simd::dot_product_fast(query, vec),
                    DistanceMetric::Hamming => crate::simd::hamming_distance_fast(query, vec),
                    DistanceMetric::Jaccard => crate::simd::jaccard_similarity_fast(query, vec),
                };
                Some((id, score))
            })
            .collect();

        // Sort by similarity (metric-dependent ordering)
        match self.metric {
            DistanceMetric::Cosine | DistanceMetric::DotProduct | DistanceMetric::Jaccard => {
                // Higher is better - sort descending
                results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            }
            DistanceMetric::Euclidean | DistanceMetric::Hamming => {
                // Lower is better - sort ascending
                results.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
            }
        }

        results.truncate(k);
        results
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
        match &mut **inner {
            HnswInner::Cosine(hnsw) => {
                hnsw.set_searching_mode(true);
            }
            HnswInner::Euclidean(hnsw) | HnswInner::Hamming(hnsw) | HnswInner::Jaccard(hnsw) => {
                hnsw.set_searching_mode(true);
            }
            HnswInner::DotProduct(hnsw) => {
                hnsw.set_searching_mode(true);
            }
        }
    }
}

impl Drop for HnswIndex {
    fn drop(&mut self) {
        // SAFETY: We must drop inner BEFORE io_holder because inner (Hnsw)
        // borrows from io_holder (HnswIo). ManuallyDrop lets us control this order.
        //
        // For indices created with new()/with_params(), io_holder is None,
        // so this is just a normal drop of the Hnsw.
        //
        // For indices loaded from disk, we drop the Hnsw first, then io_holder
        // is automatically dropped when Self is dropped (after this fn returns).
        //
        // SAFETY: ManuallyDrop::drop is unsafe because calling it twice is UB.
        // We only call it once here, and Rust won't call it again after Drop::drop.
        unsafe {
            ManuallyDrop::drop(&mut *self.inner.write());
        }
        // io_holder will be dropped automatically after this function returns
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

        // WIS-9: Single lock acquisition for all mappings
        let idx = {
            let mut mappings = self.mappings.write();
            // Check if ID already exists - hnsw_rs doesn't support updates!
            // register() returns None if ID already exists
            match mappings.register(id) {
                Some(idx) => idx,
                None => return, // ID already exists, skip insertion
            }
        };

        // Store vector for SIMD re-ranking
        let mut vectors = self.vectors.write();
        vectors.insert(idx, vector.to_vec());
        drop(vectors);

        // Insert into HNSW index
        let inner = self.inner.write();
        match &**inner {
            HnswInner::Cosine(hnsw) => {
                hnsw.insert((vector, idx));
            }
            HnswInner::Euclidean(hnsw) | HnswInner::Hamming(hnsw) | HnswInner::Jaccard(hnsw) => {
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
        // WIS-9: Single lock for mappings
        let mut mappings = self.mappings.write();
        // Soft delete: vector remains in HNSW graph but is excluded from results
        mappings.remove(id).is_some()
    }

    fn len(&self) -> usize {
        self.mappings.read().len()
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
        // Arrange - use more vectors to make HNSW more stable
        let index = HnswIndex::new(3, DistanceMetric::Cosine);
        index.insert(1, &[1.0, 0.0, 0.0]);
        index.insert(2, &[0.9, 0.1, 0.0]); // Similar to 1
        index.insert(3, &[0.0, 1.0, 0.0]); // Different
        index.insert(4, &[0.8, 0.2, 0.0]); // Similar to 1
        index.insert(5, &[0.0, 0.0, 1.0]); // Different

        // Act
        let results = index.search(&[1.0, 0.0, 0.0], 3);

        // Assert
        assert_eq!(results.len(), 3);
        // First result should be exact match (id=1) - verify it's in top results
        let top_ids: Vec<u64> = results.iter().map(|(id, _)| *id).collect();
        assert!(top_ids.contains(&1), "Exact match should be in top results");
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
        // Arrange - use more vectors to avoid HNSW flakiness with tiny datasets
        let index = HnswIndex::new(3, DistanceMetric::Euclidean);
        index.insert(1, &[0.0, 0.0, 0.0]);
        index.insert(2, &[1.0, 0.0, 0.0]); // Distance 1
        index.insert(3, &[3.0, 4.0, 0.0]); // Distance 5
        index.insert(4, &[2.0, 0.0, 0.0]); // Distance 2
        index.insert(5, &[0.5, 0.5, 0.0]); // Distance ~0.7

        // Act
        let results = index.search(&[0.0, 0.0, 0.0], 3);

        // Assert - at least get some results, first should be closest
        assert!(!results.is_empty(), "Should return results");
        assert_eq!(results[0].0, 1, "Closest should be exact match");
    }

    #[test]
    fn test_hnsw_dot_product_metric() {
        // Arrange - Use normalized positive vectors for dot product
        // DistDot in hnsw_rs requires non-negative dot products
        // Use more vectors to avoid HNSW flakiness with tiny datasets
        let index = HnswIndex::new(3, DistanceMetric::DotProduct);

        // Insert vectors with distinct dot products when queried with [1,0,0]
        index.insert(1, &[1.0, 0.0, 0.0]); // dot=1.0 with query
        index.insert(2, &[0.5, 0.5, 0.5]); // dot=0.5 with query
        index.insert(3, &[0.1, 0.1, 0.1]); // dot=0.1 with query
        index.insert(4, &[0.8, 0.2, 0.0]); // dot=0.8 with query
        index.insert(5, &[0.3, 0.3, 0.3]); // dot=0.3 with query

        // Act - Query with unit vector x
        let query = [1.0, 0.0, 0.0];
        let results = index.search(&query, 3);

        // Assert - at least get some results, first should have highest dot product
        assert!(!results.is_empty(), "Should return results");
        assert_eq!(results[0].0, 1, "Highest dot product should be first");
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
        // ID 1 should be in the top results (exact match)
        // Note: Due to parallel insertion, graph structure may vary
        let result_ids: Vec<u64> = results.iter().map(|r| r.0).collect();
        assert!(result_ids.contains(&1), "ID 1 should be in top 3 results");
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

    // =========================================================================
    // SIMD Re-ranking Tests (TDD - RED phase)
    // =========================================================================

    #[test]
    fn test_search_with_rerank_returns_k_results() {
        // Arrange
        let index = HnswIndex::new(3, DistanceMetric::Cosine);
        index.insert(1, &[1.0, 0.0, 0.0]);
        index.insert(2, &[0.9, 0.1, 0.0]);
        index.insert(3, &[0.8, 0.2, 0.0]);
        index.insert(4, &[0.0, 1.0, 0.0]);
        index.insert(5, &[0.0, 0.0, 1.0]);

        // Act
        let results = index.search_with_rerank(&[1.0, 0.0, 0.0], 3, 5);

        // Assert
        assert_eq!(results.len(), 3, "Should return exactly k results");
    }

    #[test]
    #[allow(clippy::cast_precision_loss)]
    fn test_search_with_rerank_improves_ranking() {
        // Arrange - vectors with subtle differences
        let index = HnswIndex::new(128, DistanceMetric::Cosine);

        // Create vectors with known similarity ordering
        let base: Vec<f32> = (0..128).map(|i| (i as f32 * 0.01).sin()).collect();

        // Slightly modified versions
        let mut v1 = base.clone();
        v1[0] += 0.001; // Very similar

        let mut v2 = base.clone();
        v2[0] += 0.01; // Less similar

        let mut v3 = base.clone();
        v3[0] += 0.1; // Even less similar

        index.insert(1, &v1);
        index.insert(2, &v2);
        index.insert(3, &v3);

        // Act
        let results = index.search_with_rerank(&base, 3, 3);

        // Assert - ID 1 should be closest (highest similarity)
        assert_eq!(results[0].0, 1, "Most similar vector should be first");
    }

    #[test]
    fn test_search_with_rerank_handles_rerank_k_greater_than_index_size() {
        // Arrange - use more vectors to avoid HNSW flakiness
        let index = HnswIndex::new(3, DistanceMetric::Cosine);
        index.insert(1, &[1.0, 0.0, 0.0]);
        index.insert(2, &[0.0, 1.0, 0.0]);
        index.insert(3, &[0.0, 0.0, 1.0]);
        index.insert(4, &[0.5, 0.5, 0.0]);
        index.insert(5, &[0.5, 0.0, 0.5]);

        // Act - rerank_k > index size
        let results = index.search_with_rerank(&[1.0, 0.0, 0.0], 3, 100);

        // Assert - should return at least some results
        assert!(!results.is_empty(), "Should return results");
        assert!(results.len() <= 5, "Should not exceed index size");
    }

    #[test]
    #[allow(clippy::cast_precision_loss, clippy::cast_sign_loss)]
    fn test_search_with_rerank_uses_simd_distances() {
        // Arrange
        let index = HnswIndex::new(768, DistanceMetric::Cosine);

        // Insert 100 vectors
        for i in 0..100_u64 {
            let v: Vec<f32> = (0..768)
                .map(|j| ((i + j as u64) as f32 * 0.01).sin())
                .collect();
            index.insert(i, &v);
        }

        let query: Vec<f32> = (0..768).map(|j| (j as f32 * 0.01).sin()).collect();

        // Act
        let results = index.search_with_rerank(&query, 10, 50);

        // Assert - results should have valid distances (SIMD computed)
        // Note: HNSW may return fewer results if graph not fully connected
        assert!(!results.is_empty(), "Should return at least one result");
        for (_, dist) in &results {
            assert!(*dist >= -1.0 && *dist <= 1.0, "Cosine should be in [-1, 1]");
        }

        // Results should be sorted by similarity (descending for cosine)
        for i in 1..results.len() {
            assert!(
                results[i - 1].1 >= results[i].1,
                "Results should be sorted by similarity descending"
            );
        }
    }

    #[test]
    fn test_search_with_rerank_euclidean_metric() {
        // Arrange
        let index = HnswIndex::new(3, DistanceMetric::Euclidean);
        index.insert(1, &[0.0, 0.0, 0.0]);
        index.insert(2, &[1.0, 0.0, 0.0]);
        index.insert(3, &[2.0, 0.0, 0.0]);

        // Act
        let results = index.search_with_rerank(&[0.0, 0.0, 0.0], 3, 3);

        // Assert - ID 1 should be closest (smallest distance)
        assert_eq!(results[0].0, 1, "Origin should be closest to itself");
        // For euclidean, smaller is better - results sorted ascending
        for i in 1..results.len() {
            assert!(
                results[i - 1].1 <= results[i].1,
                "Euclidean results should be sorted ascending"
            );
        }
    }

    // =========================================================================
    // WIS-8: Memory Leak Fix Tests
    // Tests for multi-tenant scenarios and proper Drop behavior
    // =========================================================================

    #[test]
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_sign_loss,
        clippy::uninlined_format_args
    )]
    fn test_hnsw_multi_tenant_load_unload() {
        // Arrange - Simulate multi-tenant scenario with multiple load/unload cycles
        // This test verifies that indices can be loaded and dropped without memory leak
        use tempfile::tempdir;

        let dir = tempdir().expect("Failed to create temp dir");

        // Create and save an index
        {
            let index = HnswIndex::new(128, DistanceMetric::Cosine);
            for i in 0..100_u64 {
                let v: Vec<f32> = (0..128)
                    .map(|j| ((i + j as u64) as f32 * 0.01).sin())
                    .collect();
                index.insert(i, &v);
            }
            index.save(dir.path()).expect("Failed to save index");
        }

        // Act - Load and drop multiple times (simulates multi-tenant load/unload)
        for iteration in 0..5 {
            let loaded = HnswIndex::load(dir.path(), 128, DistanceMetric::Cosine)
                .expect("Failed to load index");

            // Verify index works correctly
            assert_eq!(
                loaded.len(),
                100,
                "Iteration {}: Should have 100 vectors",
                iteration
            );

            let query: Vec<f32> = (0..128).map(|j| (j as f32 * 0.01).sin()).collect();
            let results = loaded.search(&query, 5);
            assert_eq!(
                results.len(),
                5,
                "Iteration {}: Should return 5 results",
                iteration
            );

            // Index is dropped here, io_holder should be freed
        }

        // If we get here without crash/hang, memory is being managed correctly
    }

    #[test]
    fn test_hnsw_drop_cleans_up_properly() {
        // Arrange - Create index, verify it can be dropped without issues
        use tempfile::tempdir;

        let dir = tempdir().expect("Failed to create temp dir");

        // Create, save, load, and drop
        {
            let index = HnswIndex::new(64, DistanceMetric::Euclidean);
            index.insert(1, &vec![0.5; 64]);
            index.insert(2, &vec![0.3; 64]);
            index.save(dir.path()).expect("Failed to save");
        }

        // Load and immediately drop
        {
            let _loaded =
                HnswIndex::load(dir.path(), 64, DistanceMetric::Euclidean).expect("Failed to load");
            // Dropped here
        }

        // Load again to verify files are still valid after previous drop
        {
            let loaded = HnswIndex::load(dir.path(), 64, DistanceMetric::Euclidean)
                .expect("Failed to load after previous drop");
            assert_eq!(loaded.len(), 2);
        }
    }

    #[test]
    #[allow(clippy::cast_precision_loss, clippy::uninlined_format_args)]
    fn test_hnsw_save_load_preserves_all_metrics() {
        use tempfile::tempdir;

        // Test Cosine and Euclidean metrics
        // Note: DotProduct has numerical precision issues in hnsw_rs with certain vectors
        for metric in [DistanceMetric::Cosine, DistanceMetric::Euclidean] {
            let dir = tempdir().expect("Failed to create temp dir");
            let dim = 32;

            // Create varied vectors (not constant) to avoid numerical issues
            let v1: Vec<f32> = (0..dim).map(|i| (i as f32 * 0.1).sin()).collect();
            let v2: Vec<f32> = (0..dim).map(|i| (i as f32 * 0.2).cos()).collect();
            let query: Vec<f32> = (0..dim).map(|i| (i as f32 * 0.15).sin()).collect();

            // Create and save
            {
                let index = HnswIndex::new(dim, metric);
                index.insert(1, &v1);
                index.insert(2, &v2);
                index.save(dir.path()).expect("Failed to save");
            }

            // Load and verify
            {
                let loaded = HnswIndex::load(dir.path(), dim, metric).expect("Failed to load");
                assert_eq!(
                    loaded.len(),
                    2,
                    "Metric {:?}: Should have 2 vectors",
                    metric
                );
                assert_eq!(loaded.metric(), metric, "Metric should be preserved");
                assert_eq!(loaded.dimension(), dim, "Dimension should be preserved");

                // Verify search works
                let results = loaded.search(&query, 2);
                assert!(
                    !results.is_empty(),
                    "Metric {:?}: Should return results",
                    metric
                );
            }
        }
    }

    // =========================================================================
    // SearchQuality Tests
    // =========================================================================

    #[test]
    fn test_search_quality_fast() {
        let index = HnswIndex::new(3, DistanceMetric::Cosine);
        // Insert more vectors for stable HNSW graph (small graphs are non-deterministic)
        index.insert(1, &[1.0, 0.0, 0.0]);
        index.insert(2, &[0.9, 0.1, 0.0]);
        index.insert(3, &[0.8, 0.2, 0.0]);
        index.insert(4, &[0.7, 0.3, 0.0]);
        index.insert(5, &[0.0, 1.0, 0.0]);

        let results = index.search_with_quality(&[1.0, 0.0, 0.0], 2, SearchQuality::Fast);
        // Fast mode may return fewer results with very small ef_search
        assert!(!results.is_empty(), "Should return at least one result");
        assert!(results.len() <= 2, "Should not exceed requested k");
    }

    #[test]
    fn test_search_quality_accurate() {
        let index = HnswIndex::new(3, DistanceMetric::Cosine);
        index.insert(1, &[1.0, 0.0, 0.0]);
        index.insert(2, &[0.9, 0.1, 0.0]);

        let results = index.search_with_quality(&[1.0, 0.0, 0.0], 2, SearchQuality::Accurate);
        // HNSW may return fewer results for very small indices
        assert!(!results.is_empty(), "Should return at least one result");
        assert_eq!(
            results[0].0, 1,
            "Accurate search should find exact match first"
        );
    }

    #[test]
    fn test_search_quality_custom_ef() {
        // Use more vectors to make HNSW more stable
        let index = HnswIndex::new(3, DistanceMetric::Cosine);
        index.insert(1, &[1.0, 0.0, 0.0]);
        index.insert(2, &[0.9, 0.1, 0.0]);
        index.insert(3, &[0.8, 0.2, 0.0]);
        index.insert(4, &[0.0, 1.0, 0.0]);
        index.insert(5, &[0.0, 0.0, 1.0]);

        let results = index.search_with_quality(&[1.0, 0.0, 0.0], 3, SearchQuality::Custom(512));
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn test_search_quality_ef_search_values() {
        // Verify ef_search values for different quality profiles
        assert_eq!(SearchQuality::Fast.ef_search(10), 64);
        assert_eq!(SearchQuality::Balanced.ef_search(10), 128);
        assert_eq!(SearchQuality::Accurate.ef_search(10), 256);
        assert_eq!(SearchQuality::Custom(100).ef_search(10), 100);

        // ef_search should be at least k * multiplier
        assert_eq!(SearchQuality::Fast.ef_search(100), 200); // k*2
        assert_eq!(SearchQuality::Balanced.ef_search(100), 400); // k*4
        assert_eq!(SearchQuality::Accurate.ef_search(100), 800); // k*8
    }

    // =========================================================================
    // Edge Cases and Error Handling
    // =========================================================================

    #[test]
    fn test_hnsw_load_nonexistent_path() {
        let result = HnswIndex::load("nonexistent_path_12345", 128, DistanceMetric::Cosine);
        assert!(result.is_err(), "Loading from nonexistent path should fail");
    }

    #[test]
    fn test_hnsw_search_with_rerank_empty_index() {
        let index = HnswIndex::new(3, DistanceMetric::Cosine);
        let results = index.search_with_rerank(&[1.0, 0.0, 0.0], 10, 50);
        assert!(
            results.is_empty(),
            "Empty index should return empty results"
        );
    }

    #[test]
    fn test_hnsw_search_with_rerank_dot_product() {
        let index = HnswIndex::new(3, DistanceMetric::DotProduct);
        index.insert(1, &[1.0, 0.0, 0.0]);
        index.insert(2, &[0.5, 0.5, 0.0]);
        index.insert(3, &[0.0, 1.0, 0.0]);

        let results = index.search_with_rerank(&[1.0, 0.0, 0.0], 3, 3);

        // HNSW may return fewer results for very small indices
        assert!(!results.is_empty(), "Should return at least one result");
        // For dot product, ID 1 should have highest score
        assert_eq!(results[0].0, 1, "Highest dot product should be first");
    }

    #[test]
    fn test_hnsw_io_holder_is_none_for_new_index() {
        // For newly created indices, io_holder should be None
        let index = HnswIndex::new(3, DistanceMetric::Cosine);
        // We can't directly access io_holder, but we can verify the index works
        // and drops without issues (no io_holder to manage)
        index.insert(1, &[1.0, 0.0, 0.0]);
        assert_eq!(index.len(), 1);
        // Dropped here without io_holder cleanup needed
    }

    #[test]
    #[allow(clippy::cast_precision_loss, clippy::cast_sign_loss)]
    fn test_hnsw_large_batch_parallel_insert() {
        let index = HnswIndex::new(128, DistanceMetric::Cosine);

        // Create 1000 vectors
        let vectors: Vec<(u64, Vec<f32>)> = (0..1000)
            .map(|i| {
                let v: Vec<f32> = (0..128).map(|j| ((i + j) as f32 * 0.001).sin()).collect();
                (i as u64, v)
            })
            .collect();

        let inserted = index.insert_batch_parallel(vectors);
        index.set_searching_mode();

        assert_eq!(inserted, 1000, "Should insert 1000 vectors");
        assert_eq!(index.len(), 1000);

        // Verify search works
        let query: Vec<f32> = (0..128).map(|j| (j as f32 * 0.001).sin()).collect();
        let results = index.search(&query, 10);
        assert_eq!(results.len(), 10);
    }
}
