//! Batch operations for HnswIndex.

use super::HnswIndex;
use crate::index::hnsw::native::PARALLEL_BATCH_MIN;
use crate::index::hnsw::params::SearchQuality;
use crate::pools;
use crate::scored_result::ScoredResult;
use crate::validation::validate_dimension_match;

/// What [`HnswIndex::insert_batch_unless_indexed`] did with a batch.
#[derive(Debug)]
pub(crate) struct UnlessIndexed {
    /// The ids the graph held already as they are, and those it inserted.
    pub(crate) settled: Vec<u64>,
    /// The ids of the pairs a refused batch did not insert: every pair the
    /// graph lacked, the batch being all-or-nothing.
    pub(crate) refused: Vec<u64>,
    /// Why the graph refused the batch.
    pub(crate) error: Option<crate::error::Error>,
}

impl HnswIndex {
    /// Inserts multiple vectors in parallel using rayon.
    ///
    /// This method is optimized for bulk insertions and can significantly
    /// reduce indexing time on multi-core systems.
    ///
    /// # Ordering
    ///
    /// Every dimension is checked before the graph sees any vector. The graph
    /// then places the batch and returns each vector's slot in input order,
    /// and only then is each id mapped to its slot (#2246) — see
    /// `docs/SOUNDNESS.md` "HNSW Slot Allocation". An id repeated within the
    /// batch ends on its last occurrence. Because `parallel_insert` uses
    /// rayon, the HNSW graph construction order is non-deterministic across
    /// runs (see v1.7.2 CHANGELOG note).
    ///
    /// # Arguments
    ///
    /// * `vectors` - Iterator of (id, vector) pairs to insert
    ///
    /// # Returns
    ///
    /// Number of vectors inserted or updated (upsert semantics). A batch
    /// holding a vector of the wrong dimension, or one the graph refuses,
    /// maps nothing and returns 0, with the cause logged; nodes the graph had
    /// already placed stay unmapped, as tombstones.
    ///
    /// # Called from a rayon worker
    ///
    /// A batch large enough for the graph pool is placed in sub-threshold
    /// slices on the calling thread instead (`pools::on_graph_pool`): a worker
    /// waiting on another pool keeps executing its own pool's jobs
    /// (`rayon-core`'s `Registry::in_worker_cross`), and one of them taking
    /// the index read guard, or a collection lock its caller holds, behind a
    /// queued writer would stop the process (\#2343). It is slower there, and
    /// never refused.
    ///
    /// # Performance (v0.8.5+)
    ///
    /// - **Faster** than sequential insertion on multi-core CPUs
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
    pub fn insert_batch_parallel<'a, I>(&self, vectors: I) -> usize
    where
        I: IntoIterator<Item = (u64, &'a [f32])>,
    {
        let items: Vec<(u64, &'a [f32])> = vectors.into_iter().collect();
        match self.try_insert_batch(&items) {
            Ok(inserted) => inserted,
            Err(e) => {
                tracing::error!("insert_batch_parallel: {e}");
                0
            }
        }
    }

    /// [`Self::insert_batch_parallel`], returning why the graph refused a
    /// batch instead of logging it: a caller retrying the batch's pairs one
    /// by one reports their refusals once, together.
    ///
    /// # Errors
    ///
    /// Returns an error if a vector has the wrong dimension, if the dedicated
    /// pool is unavailable, or if the graph refuses the batch; it inserts
    /// nothing then.
    fn try_insert_batch(&self, items: &[(u64, &[f32])]) -> crate::error::Result<usize> {
        if let Some(e) = items
            .iter()
            .find_map(|(_, vector)| validate_dimension_match(self.dimension, vector.len()).err())
        {
            return Err(e);
        }
        if items.is_empty() {
            return Ok(0);
        }

        // A batch large enough for `place_batch` to connect in parallel joins
        // on rayon while the guard below is held, so it runs on the graph pool
        // and never on the global pool: a global worker can take
        // `inner.read()`, and under a pending `vacuum` writer that closes the
        // deadlock cycle of #2343. A caller that is itself a rayon worker
        // waits on no pool at all (`pools::on_graph_pool`): it places the
        // batch in sub-threshold slices, node by node, on its own thread.
        //
        // A smaller batch places node by node and never enters rayon, so it
        // needs no pool and must not pay for one. `install` costs a flat
        // ~32 us of thread hand-off whatever the work inside, and what that
        // is worth depends entirely on the batch: measured on this call at
        // 16 dimensions, release, medians of 20 runs — n=1 places in 250 ns,
        // so the hand-off would be 128x the work; n=10 in 7.96 us, 4x; n=50
        // in 161 us, 20%; n=99 in 1.28 ms, 2.5%. So the skip earns its keep
        // on tiny batches and almost nothing at the threshold, and it is
        // worth stating that way rather than as one dramatic ratio. At a
        // larger dimension the same batch costs more and the hand-off matters
        // less still (`docs/BENCHMARKS.md` measures ~300 us per vector at
        // 768D against ~6.5 us here). The threshold read here is
        // `place_batch`'s own, so the two cannot drift apart.
        // Standard is the only backend whose `place_batch` reaches rayon:
        // RaBitQ and Sq8 insert one vector at a time at every length, to keep
        // their positional code store consistent with NodeId order
        // (`native_inner::parallel_insert`). They therefore never need the
        // pool; the length test below still installs it for them, which costs
        // the hand-off above and nothing else. Left as is rather than adding a
        // backend test here: `place_batch` owns that decision, and duplicating
        // it is how the two drift apart.
        let vectors: Vec<&[f32]> = items.iter().map(|(_, vector)| *vector).collect();
        // Held across the placement and the mappings: a sealed `vacuum` is
        // settling a frozen remainder, and this batch must land either wholly
        // before its seal or wholly after (#2335). Taken before `inner`,
        // always.
        let _publishing = self.publishing.read();
        // Held until every id is mapped: `reorder_for_locality` and `vacuum`
        // renumber slots under the write lock, so each slot placed here is still
        // its vector's when the mapping names it.
        let inner = self.inner.read();
        let placed = if vectors.len() >= PARALLEL_BATCH_MIN {
            let graph = &*inner;
            pools::on_graph_pool(
                || graph.place_parallel(&vectors),
                || {
                    let mut slots = Vec::with_capacity(vectors.len());
                    for slice in vectors.chunks(PARALLEL_BATCH_MIN - 1) {
                        slots.extend(graph.place_parallel(slice)?);
                    }
                    Ok(slots)
                },
            )
            .and_then(|placed| placed)
        } else {
            inner.place_parallel(&vectors)
        };
        let outcome = placed.map(|placements| {
            for ((id, _), placed) in items.iter().zip(placements) {
                self.mappings.assign(*id, placed);
            }
        });
        drop(inner);
        outcome.map(|()| items.len())
    }

    /// [`Self::insert_batch_parallel`] for the pairs whose id the graph does
    /// not already hold as it is: an id mapped to a linked node holding
    /// exactly that vector is left alone. Returns the ids it settled — those
    /// the graph held already and those it inserted — and, when the graph
    /// refused the batch, the ids of the pairs it did not insert and why.
    /// The batch is all-or-nothing, so one refused pair refuses every pair
    /// the graph lacked: a caller retries them one by one, each under its own
    /// hold of whatever lock it reads them under, and logs the refusals once.
    ///
    /// A merge into the index (the async builder's flush, the deferred
    /// indexer's merge) reads each id's vector from
    /// storage, and the graph may already hold it: an upsert that went
    /// straight to the graph after the id was queued. Inserting it again
    /// would only leave the node it has behind, unmapped, for every search
    /// to walk past until `vacuum`.
    pub(crate) fn insert_batch_unless_indexed(&self, pairs: &[(u64, Vec<f32>)]) -> UnlessIndexed {
        let held = self.slots_holding(pairs);
        let mut settled = Vec::with_capacity(pairs.len());
        let mut fresh: Vec<(u64, &[f32])> = Vec::with_capacity(pairs.len());
        for ((id, vector), held) in pairs.iter().zip(held) {
            if held {
                settled.push(*id);
            } else {
                fresh.push((*id, vector.as_slice()));
            }
        }
        let fresh_ids = fresh.iter().map(|&(id, _)| id);
        match self.try_insert_batch(&fresh) {
            Ok(_) => {
                settled.extend(fresh_ids);
                UnlessIndexed {
                    settled,
                    refused: Vec::new(),
                    error: None,
                }
            }
            Err(e) => UnlessIndexed {
                settled,
                refused: fresh_ids.collect(),
                error: Some(e),
            },
        }
    }

    /// For each pair, whether its id maps to a linked node holding exactly
    /// its vector. Read under one graph guard, so no renumber moves a slot
    /// between the lookups.
    fn slots_holding(&self, pairs: &[(u64, Vec<f32>)]) -> Vec<bool> {
        let inner = self.inner.read();
        // The graph keeps a Cosine vector normalized (its distance engine is
        // built pre-normalized), so a stored vector is compared in that form,
        // as crash recovery's stale-vector pass compares it.
        let normalize = self.metric == crate::DistanceMetric::Cosine;
        let same_vector: Vec<Option<usize>> = inner.with_contiguous_vectors(|arena| {
            pairs
                .iter()
                .map(|(id, vector)| {
                    let slot = self.mappings.get_idx(*id)?;
                    let held = arena.get(slot)?;
                    let same = if normalize {
                        let mut normalized = vector.clone();
                        crate::simd_native::normalize_inplace_native(&mut normalized);
                        held == normalized.as_slice()
                    } else {
                        held == vector.as_slice()
                    };
                    same.then_some(slot)
                })
                .collect()
        });
        if same_vector.len() != pairs.len() {
            // No arena to compare against: hold nothing, insert everything.
            return vec![false; pairs.len()];
        }
        let unlinked: rustc_hash::FxHashSet<usize> = inner
            .unlinked_nodes(same_vector.iter().flatten().copied())
            .into_iter()
            .collect();
        drop(inner);
        same_vector
            .into_iter()
            .map(|slot| slot.is_some_and(|slot| !unlinked.contains(&slot)))
            .collect()
    }

    /// Links into the graph, where they are, the nodes `upsert_bulk`'s direct
    /// writer placed for `ids`: the async builder's drain (#2264). Returns the
    /// number of nodes linked.
    ///
    /// Each id resolves to its slot now, under the read guard that keeps
    /// `reorder_for_locality` and `vacuum` from renumbering it. An id no longer
    /// mapped (deleted since it was queued) is skipped, and so is a slot
    /// already linked: the id was upserted through the graph since, or a
    /// `vacuum` rebuilt the graph with it. An id queued twice is linked once,
    /// at the slot of its last write.
    ///
    /// The guard is held across the connect phase, so that phase runs on the
    /// graph pool (`pools::on_graph_pool`) and never on the global rayon pool:
    /// a global worker can take `inner.read()`, and under a pending `vacuum`
    /// writer that closes the deadlock cycle of \#2343. Called from a rayon
    /// worker, it links in sub-threshold slices on the calling thread, as
    /// [`Self::insert_batch_parallel`] places.
    ///
    /// # Errors
    ///
    /// Returns an error if the arena does not hold a slot a mapping names, or
    /// if the dedicated pool cannot be built. It links nothing then, except
    /// from a rayon worker, where the slices before the failing one stay
    /// linked; a later call skips them.
    #[cfg(feature = "persistence")]
    pub(crate) fn link_placed(&self, ids: &[u64]) -> crate::error::Result<usize> {
        if ids.is_empty() {
            return Ok(0);
        }
        let inner = self.inner.read();
        let mut slots: Vec<usize> = ids
            .iter()
            .filter_map(|&id| self.mappings.get_idx(id))
            .collect();
        slots.sort_unstable();
        slots.dedup();
        let unlinked = inner.unlinked_nodes(slots);
        let graph: &crate::index::hnsw::native_inner::NativeHnswInner = &inner;
        // A rayon worker waits on no pool (`pools::on_graph_pool`): it links
        // the nodes in sub-threshold slices, one by one, on its own thread.
        pools::on_graph_pool(
            || graph.link_placed(&unlinked),
            || {
                unlinked
                    .chunks(PARALLEL_BATCH_MIN - 1)
                    .try_for_each(|slice| graph.link_placed(slice))
            },
        )??;
        drop(inner);
        Ok(unlinked.len())
    }

    /// Performs batch search for multiple queries in parallel.
    ///
    /// `Perfect`, `Adaptive`, `AutoTune`, and an index of at most 100 vectors whose graph holds at
    /// least one slot and whose exact-distance features are on, run [`Self::search_with_quality`]
    /// per query. Otherwise, when the quality calls for two-stage reranking and the index's
    /// exact-distance features are on, the method runs HNSW search for all queries (rayon), then
    /// reranks each query's candidates on GPU or SIMD; failing that, it runs HNSW-only search.
    ///
    /// # Arguments
    ///
    /// * `queries` - Slice of query vectors (as slices)
    /// * `k` - Number of nearest neighbors per query
    /// * `quality` - Search quality profile
    ///
    /// # Returns
    ///
    /// Vector of results, one per query, each containing scored results.
    ///
    /// # Performance
    ///
    /// - Uses rayon for parallel HNSW search across all queries
    /// - GPU reranking batches all candidates per query for efficient dispatch
    /// - Falls back to SIMD reranking below GPU threshold
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::Error::DimensionMismatch`] if any query dimension does not
    /// match the index dimension.
    pub fn search_batch_parallel(
        &self,
        queries: &[&[f32]],
        k: usize,
        quality: SearchQuality,
    ) -> crate::error::Result<Vec<Vec<ScoredResult>>> {
        self.validate_batch_dimensions(queries)?;

        // Perfect, Adaptive, AutoTune, or very small collections: delegate to
        // search_with_quality per-query to match single-query behavior.
        // - Perfect: brute force (the exact top-k) unless exact-distance
        //   features are off
        // - Adaptive: uses spread-based two-phase escalation (not batch-compatible)
        // - AutoTune: computes auto-ef range per dataset/dim/k (issue #699 follow-up)
        // - Small (<=100): uses brute-force for fully-connected graph safety
        //
        // Without AutoTune in this list, batch + AutoTune would fall through to the
        // standard ef_search_for_scale HNSW path below — same fixed quality for every
        // query — which silently disables the adaptive ef-range mechanism that the
        // single-query path applies via try_search_special_quality.
        if matches!(
            quality,
            SearchQuality::Perfect | SearchQuality::Adaptive { .. } | SearchQuality::AutoTune
        ) || (self.len() <= 100 && self.enable_vector_storage && self.graph_vector_count() > 0)
        {
            return pools::map(queries, |query| self.search_with_quality(query, k, quality))
                .into_iter()
                .collect();
        }

        // Aligned with single-query path (search.rs:339): use ef_search_for_scale
        // so batch and single-query produce the same ef value for the same quality
        // profile when dataset_size > 10K. Without this, batch queries on large
        // datasets would silently use a lower ef than equivalent single queries,
        // breaking the implicit contract that single and batch with the same
        // quality return the same results. See issue #694.
        let ef_search = quality.ef_search_for_scale(k, self.len());

        // Two-stage GPU/SIMD reranking for Balanced/Accurate/Custom qualities.
        if let Some(rerank_k) = self.should_two_stage_rerank(quality, k, ef_search) {
            return Ok(self.search_batch_with_rerank(queries, k, rerank_k, ef_search));
        }

        // Fast path: HNSW-only search for each query (rayon parallel)
        Ok(pools::map(queries, |query| {
            self.search_hnsw_only(query, k, ef_search)
        }))
    }

    /// Validates that all query vectors have the correct dimension.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::Error::DimensionMismatch`] on the first query whose dimension
    /// does not match the index dimension.
    fn validate_batch_dimensions(&self, queries: &[&[f32]]) -> crate::error::Result<()> {
        for query in queries {
            validate_dimension_match(self.dimension, query.len())?;
        }
        Ok(())
    }

    /// Batch search with two-stage reranking for all queries.
    ///
    /// Phase 1: HNSW search with oversampled `rerank_k` candidates (rayon).
    /// Phase 2: Rerank each query's candidates via GPU or SIMD.
    fn search_batch_with_rerank(
        &self,
        queries: &[&[f32]],
        k: usize,
        rerank_k: usize,
        ef_search: usize,
    ) -> Vec<Vec<ScoredResult>> {
        let all_candidates: Vec<Vec<ScoredResult>> = pools::map(queries, |query| {
            self.search_hnsw_only_needing(query, rerank_k, k, ef_search)
        });

        // Rerank in parallel, collecting per-query latencies for aggregated EMA update.
        let timed_results: Vec<(Vec<ScoredResult>, u64)> =
            pools::map_enumerated(queries, |i, query| {
                self.rerank_sort_and_truncate_timed(query, &all_candidates[i], k)
            });

        let total_us: u64 = timed_results.iter().map(|(_, e)| e).sum();
        if let Some(mean_us) = total_us.checked_div(timed_results.len() as u64) {
            if mean_us > 0 {
                self.update_rerank_latency_ema(mean_us);
            }
        }

        timed_results.into_iter().map(|(r, _)| r).collect()
    }

    /// Performs exact brute-force search in parallel using rayon.
    ///
    /// For large datasets (>100K vectors), automatically attempts GPU-accelerated
    /// search via `search_brute_force_gpu_inner` before falling back to rayon.
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
    /// - **Result**: the exact top-k under the index's own distance, ties aside;
    ///   nothing when the index's exact-distance features are off
    /// - **Latency**: O(n/cores) on CPU, O(n/GPU-threads) on GPU
    /// - **GPU threshold**: 100K vectors (below this, rayon is faster)
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::Error::DimensionMismatch`] if the query dimension does not
    /// match the index dimension.
    pub fn brute_force_search_parallel(
        &self,
        query: &[f32],
        k: usize,
    ) -> crate::error::Result<Vec<ScoredResult>> {
        self.validate_dimension(query)?;

        // Try GPU path for large datasets where GPU upload overhead is amortized
        #[cfg(feature = "gpu")]
        if self.len() > Self::GPU_BRUTE_FORCE_THRESHOLD {
            if let Some(results) = self.search_brute_force_gpu_inner(query, k) {
                return Ok(results);
            }
        }

        Ok(self.brute_force_search_rayon(query, k))
    }

    /// GPU brute-force dispatch accessible from tests.
    ///
    /// Delegates to `search_brute_force_gpu_inner` without the 100K threshold
    /// gate, so tests can exercise the GPU path with smaller datasets.
    ///
    /// Returns `None` in every case `search_brute_force_gpu_inner` lists.
    #[cfg(all(test, feature = "gpu"))]
    #[must_use]
    pub(crate) fn brute_force_search_gpu_dispatch(
        &self,
        query: &[f32],
        k: usize,
    ) -> Option<Vec<ScoredResult>> {
        self.search_brute_force_gpu_inner(query, k)
    }

    /// Rayon-based brute-force search over the graph's `ContiguousVectors`.
    ///
    /// Extracted from `brute_force_search_parallel` so the GPU gate in that
    /// method stays compact. Also used by `search_brute_force` as the default
    /// compute path (RF-DEDUP: single parallel implementation).
    ///
    /// Snapshots the contiguous slab under a brief read lock (one memcpy),
    /// then releases it BEFORE the rayon scan: parallel inserts acquire
    /// `vectors.write()` from inside rayon tasks, so running parallel work
    /// while holding the vectors lock could park every worker on that write
    /// and deadlock the pool (same discipline as the GPU snapshot paths).
    ///
    /// Internal indices `0..count` are filtered through `mappings.get_id`,
    /// so tombstoned slots (deleted/upserted vectors that remain in the
    /// graph store) are never returned.
    pub(super) fn brute_force_search_rayon(&self, query: &[f32], k: usize) -> Vec<ScoredResult> {
        // Preserve fast-insert semantics: exact-distance features are gated
        // on `enable_vector_storage` even though the graph stores vectors.
        if !self.enable_vector_storage {
            return Vec::new();
        }

        let (flat, dimension) = {
            let inner = self.inner.read();
            inner.with_contiguous_vectors(|vectors| {
                (vectors.as_flat_slice().to_vec(), vectors.dimension())
            })
        };
        if flat.is_empty() || dimension == 0 {
            return Vec::new();
        }

        let mut results: Vec<ScoredResult> =
            pools::filter_map_chunks_enumerated(&flat, dimension, |idx, vec| {
                let id = self.mappings.get_id(idx)?;
                let score = self.compute_distance(query, vec);
                Some(ScoredResult::new(id, score))
            });

        self.metric.sort_scored_results(&mut results);

        results.truncate(k);
        results
    }

    /// Minimum dataset size for GPU brute-force dispatch.
    ///
    /// wgpu pays a fixed overhead per dispatch, which no recorded run measures.
    /// Below 100K vectors, rayon parallel SIMD is faster due to zero
    /// GPU buffer upload overhead.
    #[cfg(feature = "gpu")]
    const GPU_BRUTE_FORCE_THRESHOLD: usize = 100_000;
}
