//! Batch operations for HnswIndex.

use super::HnswIndex;
use crate::index::hnsw::params::SearchQuality;
use rayon::prelude::*;

impl HnswIndex {
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
    pub(crate) fn prepare_batch_insert<I>(&self, vectors: I) -> Vec<(usize, Vec<f32>)>
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
    /// # Performance (v0.8.5+)
    ///
    /// - **~15x faster** than sequential insertion (29k/s vs 1.9k/s on 8-core CPU)
    /// - Automatically scales with available CPU cores
    /// - Lock-free ID mapping via `DashMap`
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use velesdb_core::index::HnswIndex;
    /// use velesdb_core::DistanceMetric;
    ///
    /// let index = HnswIndex::new(128, DistanceMetric::Cosine);
    /// let vectors: Vec<_> = (0..1000)
    ///     .map(|i| (i as u64, vec![i as f32 / 1000.0; 128]))
    ///     .collect();
    ///
    /// let inserted = index.insert_batch_parallel(vectors);
    /// println!("Inserted {} vectors", inserted);
    /// ```
    pub fn insert_batch_parallel<I>(&self, vectors: I) -> usize
    where
        I: IntoIterator<Item = (u64, Vec<f32>)>,
    {
        let to_insert = self.prepare_batch_insert(vectors);
        let count = to_insert.len();

        if count == 0 {
            return 0;
        }

        // Perf: Store vectors first (parallel-friendly) before HNSW insertion
        if self.enable_vector_storage {
            to_insert.par_iter().for_each(|(idx, vec)| {
                self.vectors.insert(*idx, vec);
            });
        }

        // Prepare references for HNSW batch insertion
        let refs_for_hnsw: Vec<(&Vec<f32>, usize)> =
            to_insert.iter().map(|(idx, vec)| (vec, *idx)).collect();

        // RF-1: Using HnswInner parallel_insert method
        self.inner.read().parallel_insert(&refs_for_hnsw);

        count
    }

    /// Sequential batch insertion (deprecated in favor of `insert_batch_parallel`).
    ///
    /// # Performance
    ///
    /// Significantly slower than `insert_batch_parallel`. Use only if you need
    /// deterministic insertion order for debugging.
    #[deprecated(
        since = "0.8.5",
        note = "Use insert_batch_parallel instead - 15x faster (29k/s vs 1.9k/s)"
    )]
    pub fn insert_batch_sequential<I>(&self, vectors: I) -> usize
    where
        I: IntoIterator<Item = (u64, Vec<f32>)>,
    {
        let to_insert = self.prepare_batch_insert(vectors);
        let count = to_insert.len();

        for (idx, vec) in &to_insert {
            self.inner.write().insert((vec.as_slice(), *idx));
            if self.enable_vector_storage {
                self.vectors.insert(*idx, vec);
            }
        }

        count
    }

    /// Performs batch search for multiple queries in parallel.
    ///
    /// # Arguments
    ///
    /// * `queries` - Slice of query vectors (as slices)
    /// * `k` - Number of nearest neighbors per query
    /// * `quality` - Search quality profile
    ///
    /// # Returns
    ///
    /// Vector of results, one per query, each containing (id, score) tuples.
    ///
    /// # Performance
    ///
    /// - Uses rayon for parallel query processing
    /// - Scales with available CPU cores
    /// - Each query is independent, enabling high parallelism
    ///
    /// # Panics
    ///
    /// Panics if any query dimension doesn't match the index dimension.
    #[must_use]
    pub fn search_batch_parallel(
        &self,
        queries: &[&[f32]],
        k: usize,
        quality: SearchQuality,
    ) -> Vec<Vec<(u64, f32)>> {
        // Validate all query dimensions first
        for (i, query) in queries.iter().enumerate() {
            assert_eq!(
                query.len(),
                self.dimension,
                "Query {} dimension mismatch: expected {}, got {}",
                i,
                self.dimension,
                query.len()
            );
        }

        // Process queries in parallel
        queries
            .par_iter()
            .map(|query| {
                let ef_search = quality.ef_search(k);
                let inner = self.inner.read();

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
}
