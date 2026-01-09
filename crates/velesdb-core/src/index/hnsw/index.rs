//! HNSW (Hierarchical Navigable Small World) index implementation.
//!
//! This module provides a high-performance approximate nearest neighbor
//! search index based on the HNSW algorithm.
//!
//! # Quality Profiles
//!
//! The index supports different quality profiles for search:
//! - `Fast`: `ef_search=64`, ~92% recall, lowest latency
//! - `Balanced`: `ef_search=128`, ~99% recall, good tradeoff (default)
//! - `Accurate`: `ef_search=256`, ~100% recall, high precision
//! - `Perfect`: `ef_search=2048`, 100% recall, maximum accuracy
//!
//! # Recommended Parameters by Vector Dimension
//!
//! | Dimension   | M     | ef_construction | ef_search |
//! |-------------|-------|-----------------|-----------|
//! | d ≤ 256     | 12-16 | 100-200         | 64-128    |
//! | 256 < d ≤768| 16-24 | 200-400         | 128-256   |
//! | d > 768     | 24-32 | 300-600         | 256-512   |

use super::native_inner::NativeHnswInner as HnswInner;
use super::params::{HnswParams, SearchQuality};
use super::sharded_mappings::ShardedMappings;
use super::sharded_vectors::ShardedVectors;
use crate::distance::DistanceMetric;
use crate::index::VectorIndex;
use parking_lot::RwLock;
use std::mem::ManuallyDrop;

// Native persistence - no HnswIo needed (v1.0+)
type HnswIo = ();

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
/// HNSW index for efficient approximate nearest neighbor search.
///
/// # Safety Invariants (Self-Referential Pattern)
///
/// When loaded from disk via [`HnswIndex::load`], this struct uses a
/// self-referential pattern where `inner` (the HNSW graph) borrows from
/// `io_holder` (the memory-mapped file). This requires careful lifetime
/// management:
///
/// 1. **Field Order**: `io_holder` must be declared AFTER `inner` so Rust's
///    default drop order drops `inner` first (fields drop in declaration order).
///
/// 2. **`ManuallyDrop`**: `inner` is wrapped in `ManuallyDrop` so we can
///    explicitly control when it's dropped in our `Drop` impl.
///
/// 3. **Custom Drop**: Our `Drop` impl explicitly drops `inner` before
///    returning, ensuring `io_holder` (dropped automatically after) outlives it.
///
/// 4. **Lifetime Extension**: We use `'static` lifetime in `HnswInner` which is
///    technically a lie - the actual lifetime is tied to `io_holder`. This is
///    safe because we guarantee `io_holder` outlives `inner` via the above.
///
/// **Note**: The `ouroboros` crate cannot be used here because `hnsw_rs::Hnsw`
/// has an invariant lifetime parameter, which is incompatible with self-referential
/// struct crates that require covariant lifetimes.
///
/// # Feature Flags (v0.8.12+)
///
/// - `native-hnsw` (default): Uses native HNSW implementation (faster, no deps)
/// - `legacy-hnsw`: Uses `hnsw_rs` library for compatibility
///
/// # Why Not Unsafe Alternatives?
///
/// - `ouroboros`/`self_cell`: Require covariant lifetimes (Hnsw is invariant)
/// - `rental`: Deprecated and unmaintained
/// - `owning_ref`: Doesn't support this pattern
///
/// The current approach is a well-documented Rust pattern for handling libraries
/// that return borrowed data from owned resources.
pub struct HnswIndex {
    /// Vector dimension
    dimension: usize,
    /// Distance metric
    metric: DistanceMetric,
    /// Internal HNSW index (type-erased for flexibility).
    ///
    /// # Safety
    ///
    /// Wrapped in `ManuallyDrop` to control drop order. MUST be dropped
    /// BEFORE `io_holder` because it contains references into `io_holder`'s
    /// memory-mapped data (when loaded from disk).
    inner: RwLock<ManuallyDrop<HnswInner>>,
    /// ID mappings (external ID <-> internal index) - lock-free via `DashMap` (EPIC-A.1)
    mappings: ShardedMappings,
    /// Vector storage for SIMD re-ranking - sharded for parallel writes (EPIC-A.2)
    vectors: ShardedVectors,
    /// Whether to store vectors in `ShardedVectors` for re-ranking.
    ///
    /// When `false`, vectors are only stored in HNSW graph, providing:
    /// - ~2x faster insert throughput
    /// - ~50% less memory usage
    /// - No SIMD re-ranking or brute-force search support
    ///
    /// Default: `true` (full functionality)
    enable_vector_storage: bool,
    /// Holds the `HnswIo` for loaded indices.
    ///
    /// # Safety
    ///
    /// This field MUST be declared AFTER `inner` and MUST outlive `inner`.
    /// The `Hnsw` in `inner` borrows from the memory-mapped data owned by `HnswIo`.
    /// Our `Drop` impl ensures `inner` is dropped first.
    ///
    /// - `Some(Box<HnswIo>)`: Index was loaded from disk, `inner` borrows from this
    /// - `None`: Index was created in memory, no borrowing relationship
    #[allow(dead_code)] // Read implicitly via lifetime - dropped after inner
    io_holder: Option<Box<HnswIo>>,
}

// RF-2: HnswInner enum and its impl blocks moved to inner.rs

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
        // RF-2.6: Use HnswInner factory method to eliminate code duplication
        let inner = HnswInner::new(
            metric,
            params.max_connections,
            params.max_elements,
            params.ef_construction,
        );

        Self {
            dimension,
            metric,
            inner: RwLock::new(ManuallyDrop::new(inner)),
            mappings: ShardedMappings::new(),
            vectors: ShardedVectors::new(dimension),
            enable_vector_storage: true, // Default: full functionality
            io_holder: None,             // No io_holder for newly created indices
        }
    }

    /// Creates a new HNSW index optimized for fast inserts.
    ///
    /// This disables vector storage in `ShardedVectors`, providing:
    /// - ~2x faster insert throughput
    /// - ~50% less memory usage
    ///
    /// **Trade-off**: SIMD re-ranking and brute-force search are disabled.
    /// Use this when you only need approximate HNSW search.
    ///
    /// # Arguments
    ///
    /// * `dimension` - The dimension of vectors to index
    /// * `metric` - The distance metric to use
    #[must_use]
    pub fn new_fast_insert(dimension: usize, metric: DistanceMetric) -> Self {
        let mut index = Self::new(dimension, metric);
        index.enable_vector_storage = false;
        index
    }

    /// Creates a new HNSW index with custom parameters, optimized for fast inserts.
    ///
    /// Same as [`Self::new_fast_insert`] but with custom HNSW parameters.
    #[must_use]
    pub fn with_params_fast_insert(
        dimension: usize,
        metric: DistanceMetric,
        params: HnswParams,
    ) -> Self {
        let mut index = Self::with_params(dimension, metric, params);
        index.enable_vector_storage = false;
        index
    }

    /// Creates a new HNSW index in turbo mode for maximum insert throughput.
    ///
    /// **Target**: 5k+ vec/s (vs ~2k/s with standard `new()`)
    ///
    /// # Trade-offs
    ///
    /// - **Recall**: ~85% (vs ≥95% with standard params)
    /// - **Best for**: Bulk loading, development, benchmarking
    /// - **Not recommended for**: Production search workloads requiring high recall
    ///
    /// # Example
    ///
    /// ```rust
    /// use velesdb_core::{HnswIndex, DistanceMetric};
    ///
    /// // Create turbo index for fast bulk loading
    /// let index = HnswIndex::new_turbo(768, DistanceMetric::Cosine);
    /// ```
    #[must_use]
    pub fn new_turbo(dimension: usize, metric: DistanceMetric) -> Self {
        Self::with_params(dimension, metric, HnswParams::turbo())
    }

    /// Saves the HNSW index and ID mappings to the specified directory.
    ///
    /// # Errors
    ///
    /// Returns an error if saving fails.
    pub fn save<P: AsRef<std::path::Path>>(&self, path: P) -> std::io::Result<()> {
        use std::fs;
        use std::io::BufWriter;
        let path = path.as_ref();
        fs::create_dir_all(path)?;

        // Save native HNSW index
        let inner = self.inner.read();
        inner.file_dump(path, "hnsw")?;

        // Save mappings using bincode
        let mappings_path = path.join("id_mappings.bin");
        let file = fs::File::create(mappings_path)?;
        let writer = BufWriter::new(file);
        let (id_to_idx, idx_to_id, next_idx) = self.mappings.as_parts();
        bincode::serialize_into(writer, &(id_to_idx, idx_to_id, next_idx))
            .map_err(std::io::Error::other)?;

        Ok(())
    }

    /// Loads the HNSW index and ID mappings from the specified directory.
    ///
    /// # Errors
    ///
    /// Returns an error if loading fails (missing files, corrupted data, etc.).
    pub fn load<P: AsRef<std::path::Path>>(
        path: P,
        dimension: usize,
        metric: DistanceMetric,
    ) -> std::io::Result<Self> {
        use std::collections::HashMap;
        use std::fs;
        use std::io::BufReader;
        let path = path.as_ref();

        // Load native HNSW index
        let inner = HnswInner::file_load(path, "hnsw", metric)?;

        // Load mappings using bincode
        let mappings_path = path.join("id_mappings.bin");
        let file = fs::File::open(mappings_path)?;
        let reader = BufReader::new(file);
        let (id_to_idx, idx_to_id, next_idx): (HashMap<u64, usize>, HashMap<usize, u64>, usize) =
            bincode::deserialize_from(reader).map_err(std::io::Error::other)?;
        let mappings = ShardedMappings::from_parts(id_to_idx, idx_to_id, next_idx);

        Ok(Self {
            dimension,
            metric,
            inner: RwLock::new(ManuallyDrop::new(inner)),
            mappings,
            vectors: ShardedVectors::new(dimension),
            enable_vector_storage: true,
            io_holder: None,
        })
    }

    /// Validates that the query/vector dimension matches the index dimension.
    ///
    /// RF-2.7: Helper to eliminate 7x duplicated validation pattern.
    ///
    /// # Panics
    ///
    /// Panics if the dimension doesn't match.
    #[inline]
    fn validate_dimension(&self, data: &[f32], data_type: &str) {
        assert_eq!(
            data.len(),
            self.dimension,
            "{data_type} dimension mismatch: expected {}, got {}",
            self.dimension,
            data.len()
        );
    }

    /// Computes exact SIMD distance between query and vector based on metric.
    ///
    /// This helper eliminates code duplication across search methods.
    #[inline]
    fn compute_distance(&self, query: &[f32], vector: &[f32]) -> f32 {
        match self.metric {
            DistanceMetric::Cosine => crate::simd::cosine_similarity_fast(query, vector),
            DistanceMetric::Euclidean => crate::simd::euclidean_distance_fast(query, vector),
            DistanceMetric::DotProduct => crate::simd::dot_product_fast(query, vector),
            DistanceMetric::Hamming => crate::simd::hamming_distance_fast(query, vector),
            DistanceMetric::Jaccard => crate::simd::jaccard_similarity_fast(query, vector),
        }
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
    /// - `Fast`: ~92% recall, lowest latency
    /// - `Balanced`: ~99% recall, good tradeoff (default)
    /// - `Accurate`: ~100% recall, high precision
    /// - `Perfect`: 100% recall guaranteed via SIMD re-ranking
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
        self.validate_dimension(query, "Query");

        // Perfect mode uses brute-force SIMD for guaranteed 100% recall
        if matches!(quality, SearchQuality::Perfect) {
            return self.search_brute_force(query, k);
        }

        // For very small collections (≤100 vectors), use brute-force to guarantee 100% recall
        // HNSW graph may not be fully connected with so few nodes, causing missed results
        // Only use brute-force if vector storage is enabled (not in fast-insert mode)
        if self.len() <= 100 && self.enable_vector_storage && !self.vectors.is_empty() {
            return self.search_brute_force(query, k);
        }

        let ef_search = quality.ef_search(k);
        let inner = self.inner.read();

        // RF-1: Using HnswInner methods for search and score transformation
        let neighbours = inner.search(query, k, ef_search);
        let mut results: Vec<(u64, f32)> = Vec::with_capacity(neighbours.len());

        for n in &neighbours {
            if let Some(id) = self.mappings.get_id(n.d_id) {
                let score = inner.transform_score(n.distance);
                results.push((id, score));
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
        self.validate_dimension(query, "Query");

        // 1. Get candidates from HNSW (fast approximate search)
        let candidates = self.search_with_quality(query, rerank_k, SearchQuality::Accurate);

        if candidates.is_empty() {
            return Vec::new();
        }

        // 2. Re-rank using SIMD-optimized exact distance computation
        // EPIC-A.2: Collect candidate vectors from ShardedVectors for re-ranking
        let candidate_vectors: Vec<(u64, usize, Vec<f32>)> = candidates
            .iter()
            .filter_map(|(id, _)| {
                let idx = self.mappings.get_idx(*id)?;
                let vec = self.vectors.get(idx)?;
                Some((*id, idx, vec))
            })
            .collect();

        // Perf TS-CORE-001: Adaptive prefetch distance based on vector size
        let prefetch_distance = crate::simd::calculate_prefetch_distance(self.dimension);
        let mut reranked: Vec<(u64, f32)> = Vec::with_capacity(candidate_vectors.len());

        for (i, (id, _idx, v)) in candidate_vectors.iter().enumerate() {
            // Prefetch upcoming vectors (P1 optimization on local snapshot)
            if i + prefetch_distance < candidate_vectors.len() {
                crate::simd::prefetch_vector(&candidate_vectors[i + prefetch_distance].2);
            }

            // Compute exact distance for current vector
            let exact_dist = self.compute_distance(query, v);

            reranked.push((*id, exact_dist));
        }

        // 3. Sort by distance (metric-dependent ordering)
        self.metric.sort_results(&mut reranked);

        // 4. Return top k
        reranked.truncate(k);
        reranked
    }

    /// Brute-force search using SIMD for guaranteed 100% recall.
    ///
    /// Computes exact distance to ALL vectors in the index and returns the top k.
    /// Use only for small datasets or when 100% recall is critical.
    ///
    /// # Arguments
    ///
    /// * `query` - The query vector
    /// * `k` - Number of nearest neighbors to return
    #[must_use]
    pub fn search_brute_force(&self, query: &[f32], k: usize) -> Vec<(u64, f32)> {
        if self.vectors.is_empty() {
            return Vec::new();
        }

        // EPIC-A.2: Use collect_for_parallel for ShardedVectors iteration
        let vectors_snapshot = self.vectors.collect_for_parallel();

        // Compute distance to all vectors using SIMD
        let mut all_distances: Vec<(u64, f32)> = Vec::with_capacity(vectors_snapshot.len());

        for (idx, vec) in &vectors_snapshot {
            if let Some(id) = self.mappings.get_id(*idx) {
                let dist = self.compute_distance(query, vec);
                all_distances.push((id, dist));
            }
        }

        // Sort by distance (metric-dependent ordering)
        self.metric.sort_results(&mut all_distances);

        all_distances.truncate(k);
        all_distances
    }

    /// Brute-force search with thread-local buffer reuse (RF-3 optimization).
    ///
    /// This method uses a thread-local buffer to avoid repeated allocations
    /// when performing multiple brute-force searches. Ideal for hot paths
    /// where brute-force is called repeatedly.
    ///
    /// # Performance
    ///
    /// - First call per thread: Normal allocation
    /// - Subsequent calls: ~40% fewer allocations (buffer reuse)
    ///
    /// # Arguments
    ///
    /// * `query` - The query vector
    /// * `k` - Number of nearest neighbors to return
    #[must_use]
    pub fn search_brute_force_buffered(&self, query: &[f32], k: usize) -> Vec<(u64, f32)> {
        use std::cell::RefCell;

        thread_local! {
            static BUFFER: RefCell<Vec<(usize, Vec<f32>)>> = const { RefCell::new(Vec::new()) };
        }

        if self.vectors.is_empty() {
            return Vec::new();
        }

        BUFFER.with(|buf| {
            let mut buffer = buf.borrow_mut();
            self.vectors.collect_into(&mut buffer);

            // Compute distance to all vectors using SIMD
            let mut all_distances: Vec<(u64, f32)> = Vec::with_capacity(buffer.len());

            for (idx, vec) in buffer.iter() {
                if let Some(id) = self.mappings.get_id(*idx) {
                    let dist = self.compute_distance(query, vec);
                    all_distances.push((id, dist));
                }
            }

            // Sort by distance (metric-dependent ordering)
            self.metric.sort_results(&mut all_distances);

            all_distances.truncate(k);
            all_distances
        })
    }

    /// GPU-accelerated brute-force search for large datasets.
    ///
    /// Uses GPU compute shaders for batch distance calculation when available.
    /// Falls back to `None` if GPU is not available or not supported.
    ///
    /// # Performance (P1-GPU-1)
    ///
    /// - **When to use**: Datasets >10K vectors, batch queries
    /// - **Speedup**: 5-10x for large batches on discrete GPU
    /// - **Fallback**: Returns `None` if GPU unavailable, caller should use CPU
    ///
    /// # Arguments
    ///
    /// * `query` - The query vector
    /// * `k` - Number of nearest neighbors to return
    ///
    /// # Returns
    ///
    /// `Some(results)` if GPU available, `None` otherwise.
    /// Caller should fallback to `search_brute_force` if `None`.
    #[must_use]
    pub fn search_brute_force_gpu(&self, query: &[f32], k: usize) -> Option<Vec<(u64, f32)>> {
        #[cfg(feature = "gpu")]
        {
            use crate::gpu::GpuAccelerator;

            // Only use GPU for Cosine metric (others not yet implemented)
            if self.metric != DistanceMetric::Cosine {
                return None;
            }

            // Try to get GPU accelerator
            let gpu = GpuAccelerator::new()?;

            // Collect all vectors into contiguous buffer for GPU
            let vectors_snapshot = self.vectors.collect_for_parallel();
            if vectors_snapshot.is_empty() {
                return Some(Vec::new());
            }

            // Build contiguous vector buffer and ID mapping
            let mut flat_vectors: Vec<f32> =
                Vec::with_capacity(vectors_snapshot.len() * self.dimension);
            let mut id_map: Vec<u64> = Vec::with_capacity(vectors_snapshot.len());

            for (idx, vec) in &vectors_snapshot {
                if let Some(id) = self.mappings.get_id(*idx) {
                    flat_vectors.extend_from_slice(vec);
                    id_map.push(id);
                }
            }

            if id_map.is_empty() {
                return Some(Vec::new());
            }

            // GPU batch cosine similarity
            let similarities = gpu.batch_cosine_similarity(&flat_vectors, query, self.dimension);

            // Combine IDs with similarities
            let mut results: Vec<(u64, f32)> = id_map.into_iter().zip(similarities).collect();

            // Sort by similarity (descending for cosine)
            self.metric.sort_results(&mut results);

            results.truncate(k);
            Some(results)
        }

        #[cfg(not(feature = "gpu"))]
        {
            let _ = (query, k); // Suppress unused warnings
            None
        }
    }

    /// Searches with SIMD-based re-ranking using a custom quality for initial search.
    ///
    /// Similar to `search_with_rerank` but allows specifying the quality profile
    /// for the initial HNSW search phase.
    ///
    /// # Arguments
    ///
    /// * `query` - The query vector
    /// * `k` - Number of nearest neighbors to return
    /// * `rerank_k` - Number of candidates to retrieve before re-ranking
    /// * `initial_quality` - Quality profile for initial HNSW search
    ///
    /// # Panics
    ///
    /// Panics if the query dimension doesn't match the index dimension.
    #[must_use]
    pub fn search_with_rerank_quality(
        &self,
        query: &[f32],
        k: usize,
        rerank_k: usize,
        initial_quality: SearchQuality,
    ) -> Vec<(u64, f32)> {
        self.validate_dimension(query, "Query");

        // 1. Get candidates from HNSW with specified quality
        // Avoid recursion if initial_quality is Perfect
        let actual_quality = if matches!(initial_quality, SearchQuality::Perfect) {
            SearchQuality::Accurate
        } else {
            initial_quality
        };
        let candidates = self.search_with_quality(query, rerank_k, actual_quality);

        if candidates.is_empty() {
            return Vec::new();
        }

        // 2. Re-rank using SIMD-optimized exact distance computation
        // EPIC-A.2: Collect candidate vectors from ShardedVectors
        let candidate_vectors: Vec<(u64, usize, Vec<f32>)> = candidates
            .iter()
            .filter_map(|(id, _)| {
                let idx = self.mappings.get_idx(*id)?;
                let vec = self.vectors.get(idx)?;
                Some((*id, idx, vec))
            })
            .collect();

        let prefetch_distance = crate::simd::calculate_prefetch_distance(self.dimension);
        let mut reranked: Vec<(u64, f32)> = Vec::with_capacity(candidate_vectors.len());

        for (i, (id, _idx, v)) in candidate_vectors.iter().enumerate() {
            // Prefetch upcoming vectors
            if i + prefetch_distance < candidate_vectors.len() {
                crate::simd::prefetch_vector(&candidate_vectors[i + prefetch_distance].2);
            }

            // Compute exact distance
            let exact_dist = self.compute_distance(query, v);

            reranked.push((*id, exact_dist));
        }

        // 3. Sort by distance (metric-dependent ordering)
        self.metric.sort_results(&mut reranked);

        reranked.truncate(k);
        reranked
    }

    /// Prepares vectors for batch insertion: validates dimensions and registers IDs.
    ///
    /// Returns a vector of (`internal_index`, vector) pairs ready for insertion.
    /// Duplicates are automatically skipped.
    ///
    /// # Performance
    ///
    /// - Single pass over input (no intermediate collection)
    /// - Pre-allocated output vector
    /// - Inline dimension validation
    #[inline]
    fn prepare_batch_insert<I>(&self, vectors: I) -> Vec<(usize, Vec<f32>)>
    where
        I: IntoIterator<Item = (u64, Vec<f32>)>,
    {
        let iter = vectors.into_iter();
        let (lower, upper) = iter.size_hint();
        let capacity = upper.unwrap_or(lower);
        let mut to_insert: Vec<(usize, Vec<f32>)> = Vec::with_capacity(capacity);

        for (id, vector) in iter {
            // Inline validation for hot path
            assert_eq!(
                vector.len(),
                self.dimension,
                "Vector dimension mismatch: expected {}, got {}",
                self.dimension,
                vector.len()
            );
            if let Some(idx) = self.mappings.register(id) {
                to_insert.push((idx, vector));
            }
        }

        to_insert
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
    /// Batch insert using sequential insertion (more reliable than `parallel_insert`).
    ///
    /// # Why sequential?
    ///
    /// The `hnsw_rs::parallel_insert` can cause issues:
    /// - Rayon thread pool conflicts with async runtimes
    /// - Potential deadlocks on Windows with `parking_lot`
    /// - Less predictable behavior with high-dimensional vectors
    ///
    /// Sequential insertion is fast enough for most use cases and more reliable.
    pub fn insert_batch_parallel<I>(&self, vectors: I) -> usize
    where
        I: IntoIterator<Item = (u64, Vec<f32>)>,
    {
        // RF-2.5: Use helper for validation and ID registration
        let to_insert = self.prepare_batch_insert(vectors);

        // Prepare references for hnsw_rs parallel_insert: &[(&Vec<T>, usize)]
        let data_refs: Vec<(&Vec<f32>, usize)> =
            to_insert.iter().map(|(idx, v)| (v, *idx)).collect();

        let count = data_refs.len();

        // Insert into HNSW graph using native parallel_insert (uses rayon internally)
        // RF-1: Using HnswInner method
        {
            let inner = self.inner.write();
            inner.parallel_insert(&data_refs);
        }

        // Perf: Conditionally store vectors for SIMD re-ranking
        if self.enable_vector_storage {
            self.vectors.insert_batch(to_insert);
        }

        count
    }

    /// Inserts multiple vectors sequentially (DEPRECATED).
    ///
    /// # Deprecated
    ///
    /// **Use [`Self::insert_batch_parallel`] instead** - it's 15x faster (29k/s vs 1.9k/s).
    ///
    /// This method exists for backward compatibility only. The theoretical use cases
    /// (rayon/tokio conflicts) have not materialized in practice.
    ///
    /// # Performance Comparison
    ///
    /// | Method | Throughput | Recommendation |
    /// |--------|------------|----------------|
    /// | `insert_batch_parallel` | **29.3k/s** | ✅ Use this |
    /// | `insert_batch_sequential` | 1.9k/s | ❌ Deprecated |
    ///
    /// # Arguments
    ///
    /// * `vectors` - Iterator of (id, vector) pairs to insert
    ///
    /// # Returns
    ///
    /// Number of vectors successfully inserted (duplicates are skipped).
    #[deprecated(
        since = "0.8.5",
        note = "Use insert_batch_parallel instead - 15x faster (29k/s vs 1.9k/s)"
    )]
    pub fn insert_batch_sequential<I>(&self, vectors: I) -> usize
    where
        I: IntoIterator<Item = (u64, Vec<f32>)>,
    {
        // RF-2.5: Use helper for validation and ID registration
        let to_insert = self.prepare_batch_insert(vectors);
        let count = to_insert.len();
        if count == 0 {
            return 0;
        }

        // Perf: Insert into HNSW FIRST (using references), then move to ShardedVectors
        // This avoids unnecessary clone() that was causing 2x allocation overhead
        {
            let inner = self.inner.write();
            for (idx, vector) in &to_insert {
                inner.insert((vector.as_slice(), *idx));
            }
        }

        // Perf: Conditionally store vectors for SIMD re-ranking
        if self.enable_vector_storage {
            self.vectors.insert_batch(to_insert);
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

        // Perf TS-CORE-002: Acquire locks ONCE for entire batch to reduce contention
        // Before: N lock acquire/release cycles for N queries
        // After: 1 lock acquire, N searches, 1 release
        let ef_search = quality.ef_search(k);
        let inner = self.inner.read();

        queries
            .par_iter()
            .map(|query| {
                self.validate_dimension(query, "Query");

                // RF-1: Using HnswInner methods for search and score transformation
                let neighbours = inner.search(query, k, ef_search);
                let mut results: Vec<(u64, f32)> = Vec::with_capacity(neighbours.len());

                for n in &neighbours {
                    if let Some(id) = self.mappings.get_id(n.d_id) {
                        let score = inner.transform_score(n.distance);
                        results.push((id, score));
                    }
                }

                results
            })
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

        self.validate_dimension(query, "Query");

        // EPIC-A.2: Use collect_for_parallel for rayon par_iter support
        let vectors_snapshot = self.vectors.collect_for_parallel();

        // Compute distances in parallel using rayon
        let mut results: Vec<(u64, f32)> = vectors_snapshot
            .par_iter()
            .filter_map(|(idx, vec)| {
                let id = self.mappings.get_id(*idx)?;
                let score = self.compute_distance(query, vec);
                Some((id, score))
            })
            .collect();

        // Sort by distance (metric-dependent ordering)
        self.metric.sort_results(&mut results);

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
        // RF-1: Using HnswInner method
        let mut inner = self.inner.write();
        inner.set_searching_mode(true);
    }

    // =========================================================================
    // Vacuum / Maintenance Operations
    // =========================================================================

    /// Returns the number of tombstones (soft-deleted entries) in the index.
    ///
    /// Tombstones are entries that have been removed from mappings but still
    /// exist in the underlying HNSW graph. High tombstone count degrades
    /// search performance.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let index = HnswIndex::new(128, DistanceMetric::Cosine);
    /// // Insert and delete some vectors...
    /// if index.tombstone_ratio() > 0.2 {
    ///     index.needs_vacuum(); // Consider rebuilding
    /// }
    /// ```
    #[must_use]
    pub fn tombstone_count(&self) -> usize {
        // Total inserted = next_idx in mappings (monotonic counter)
        // Active = mappings.len()
        // Tombstones = Total - Active
        let total_inserted = self.mappings.next_idx();
        let active = self.mappings.len();
        total_inserted.saturating_sub(active)
    }

    /// Returns the tombstone ratio (0.0 = clean, 1.0 = 100% deleted).
    ///
    /// Use this to decide when to trigger a vacuum/rebuild operation.
    /// A ratio > 0.2 (20%) is a reasonable threshold for considering vacuum.
    #[must_use]
    #[allow(clippy::cast_precision_loss)] // Acceptable precision loss for ratio calculation
    pub fn tombstone_ratio(&self) -> f64 {
        let total = self.mappings.next_idx();
        if total == 0 {
            return 0.0;
        }
        let tombstones = self.tombstone_count();
        tombstones as f64 / total as f64
    }

    /// Returns true if the index has significant fragmentation and would
    /// benefit from a vacuum/rebuild operation.
    ///
    /// Current threshold: 20% tombstones
    #[must_use]
    pub fn needs_vacuum(&self) -> bool {
        self.tombstone_ratio() > 0.2
    }

    /// Rebuilds the HNSW index, removing all tombstones.
    ///
    /// This creates a new HNSW graph containing only the active vectors,
    /// eliminating fragmentation and improving search performance.
    ///
    /// # Important
    ///
    /// - This operation is **blocking** and may take significant time for large indices
    /// - The index remains readable during rebuild (copy-on-write pattern)
    /// - Requires `enable_vector_storage = true` (vectors must be stored)
    ///
    /// # Returns
    ///
    /// - `Ok(count)` - Number of vectors in the rebuilt index
    /// - `Err` - If vector storage is disabled or rebuild fails
    ///
    /// # Errors
    ///
    /// Returns `VacuumError::VectorStorageDisabled` if the index was created
    /// with `new_fast_insert()` mode, which disables vector storage.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let index = HnswIndex::new(128, DistanceMetric::Cosine);
    /// // ... insert and delete many vectors ...
    ///
    /// if index.needs_vacuum() {
    ///     let count = index.vacuum()?;
    ///     println!("Rebuilt index with {} vectors", count);
    /// }
    /// ```
    pub fn vacuum(&self) -> Result<usize, VacuumError> {
        if !self.enable_vector_storage {
            return Err(VacuumError::VectorStorageDisabled);
        }

        // 1. Collect all active vectors (copy-on-write snapshot)
        let active_vectors: Vec<(u64, Vec<f32>)> = self
            .mappings
            .iter()
            .filter_map(|(id, idx)| self.vectors.get(idx).map(|vec| (id, vec)))
            .collect();

        let count = active_vectors.len();

        if count == 0 {
            return Ok(0);
        }

        // 2. Create new HNSW graph with auto-tuned parameters
        let params = HnswParams::auto(self.dimension);
        let new_inner = HnswInner::new(
            self.metric,
            params.max_connections,
            count.max(1000), // max_elements with reasonable minimum
            params.ef_construction,
        );

        // 3. Create new mappings and vectors
        let new_mappings = ShardedMappings::with_capacity(count);
        let new_vectors = ShardedVectors::new(self.dimension);

        // 4. Bulk insert into new structures
        let refs_for_hnsw: Vec<(&Vec<f32>, usize)> = active_vectors
            .iter()
            .enumerate()
            .map(|(idx, (id, vec))| {
                // Register in new mappings
                new_mappings.register(*id);
                // Store in new vectors
                new_vectors.insert(idx, vec);
                (vec, idx)
            })
            .collect();

        // 5. Parallel insert into new HNSW
        new_inner.parallel_insert(&refs_for_hnsw);

        // 6. Atomic swap (replace old with new)
        {
            let mut inner_guard = self.inner.write();
            // Drop old inner safely
            unsafe {
                ManuallyDrop::drop(&mut *inner_guard);
            }
            // Replace with new
            *inner_guard = ManuallyDrop::new(new_inner);
        }

        // 7. Swap mappings and vectors
        // Note: ShardedMappings/ShardedVectors use interior mutability,
        // so we need to clear and repopulate
        self.mappings.clear();
        self.vectors.clear();

        for (id, vec) in active_vectors {
            if let Some(idx) = self.mappings.register(id) {
                self.vectors.insert(idx, &vec);
            }
        }

        Ok(count)
    }
}

/// Errors that can occur during vacuum operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VacuumError {
    /// Vector storage is disabled, cannot rebuild index
    VectorStorageDisabled,
}

impl std::fmt::Display for VacuumError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::VectorStorageDisabled => {
                write!(f, "Cannot vacuum: vector storage is disabled (use new() instead of new_fast_insert())")
            }
        }
    }
}

impl std::error::Error for VacuumError {}

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
    #[inline]
    fn insert(&self, id: u64, vector: &[f32]) {
        // Inline validation for hot path performance
        assert_eq!(
            vector.len(),
            self.dimension,
            "Vector dimension mismatch: expected {}, got {}",
            self.dimension,
            vector.len()
        );

        // Register the ID and get internal index with ShardedMappings
        // Check if ID already exists - hnsw_rs doesn't support updates!
        // register() returns None if ID already exists
        let Some(idx) = self.mappings.register(id) else {
            return; // ID already exists, skip insertion
        };

        // Insert into HNSW index (RF-1: using HnswInner method)
        // Perf: Minimize lock hold time by not explicitly dropping
        self.inner.write().insert((vector, idx));

        // Perf: Conditionally store vector for SIMD re-ranking
        // When disabled, saves ~50% memory and ~2x insert speed
        if self.enable_vector_storage {
            self.vectors.insert(idx, vector);
        }
    }

    fn search(&self, query: &[f32], k: usize) -> Vec<(u64, f32)> {
        // Perf: Use Balanced quality for best latency/recall tradeoff
        // ef_search=128 provides ~95% recall with minimal latency
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
        // EPIC-A.1: Lock-free removal with ShardedMappings
        // Soft delete: vector remains in HNSW graph but is excluded from results
        self.mappings.remove(id).is_some()
    }

    fn len(&self) -> usize {
        self.mappings.len()
    }

    fn dimension(&self) -> usize {
        self.dimension
    }

    fn metric(&self) -> DistanceMetric {
        self.metric
    }
}

// ============================================================================
// Safety tests - must stay in this file (require private field access)
// ============================================================================
#[cfg(test)]
mod safety_tests {
    use super::*;

    /// Compile-time assertion that `io_holder` field is declared AFTER `inner`.
    #[test]
    fn test_field_order_io_holder_after_inner() {
        use std::mem::offset_of;

        let inner_offset = offset_of!(HnswIndex, inner);
        let io_holder_offset = offset_of!(HnswIndex, io_holder);

        assert!(
            inner_offset < io_holder_offset,
            "CRITICAL: io_holder must be declared AFTER inner for correct drop order"
        );
    }
}
