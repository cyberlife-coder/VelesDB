//! Batch and multi-query search methods for Collection.

use super::live::Nearest;
use super::pending::PendingRestriction;
use crate::collection::search::query::row_universe::RowUniverse;
use crate::collection::types::Collection;
use crate::error::{Error, Result};
use crate::filter::Filter;
use crate::point::SearchResult;
use crate::pools;
use crate::scored_result::ScoredResult;

impl Collection {
    /// Performs batch search for multiple query vectors in parallel with metadata filtering.
    /// Supports a different filter for each query in the batch.
    ///
    /// # Arguments
    ///
    /// * `queries` - List of query vector slices
    /// * `k` - Maximum number of results per query
    /// * `filters` - List of optional filters (must match queries length)
    ///
    /// # Returns
    ///
    /// Vector of search results for each query, matching its respective filter.
    ///
    /// # Errors
    ///
    /// Returns an error if queries and filters have different lengths or dimension mismatch.
    pub fn search_batch_with_filters(
        &self,
        queries: &[&[f32]],
        k: usize,
        filters: &[Option<Filter>],
    ) -> Result<Vec<Vec<SearchResult>>> {
        let metric = self.validate_filtered_batch(queries, filters)?;
        let candidates_k = k.saturating_mul(4).max(k + 10);
        let restriction =
            |filter: &Option<Filter>| PendingRestriction::NONE.post_filtered_if(filter.is_some());
        let first = self.batch_search_merging_pending(
            queries,
            candidates_k,
            metric,
            filters.iter().map(restriction),
        )?;
        first
            .into_iter()
            .zip(queries)
            .zip(filters)
            .map(|((first, query), filter)| {
                self.nearest_live_where(
                    &Nearest::judging(query, metric, k, candidates_k),
                    |id, payload| filter.as_ref().is_none_or(|f| f.matches_point(id, payload)),
                    self.after_batch(first, query, metric, restriction(filter)),
                )
            })
            .collect()
    }

    /// Checks a filtered batch's shape and query dimensions, returning the
    /// collection's metric.
    fn validate_filtered_batch(
        &self,
        queries: &[&[f32]],
        filters: &[Option<Filter>],
    ) -> Result<crate::DistanceMetric> {
        if queries.len() != filters.len() {
            return Err(Error::Config(format!(
                "Queries count ({}) does not match filters count ({})",
                queries.len(),
                filters.len()
            )));
        }
        self.validate_queries_and_read_metric(queries)
    }

    /// What one query of a batch asks the index for, as a vector read asks
    /// it ([`Self::nearest_live_rows`]): first the candidates the batch's
    /// parallel index search already found for it, then — when dead rows
    /// crowd them — a search of its own for `n`, the pending points merged
    /// under `restriction`.
    fn after_batch<'q>(
        &'q self,
        first: Vec<ScoredResult>,
        query: &'q [f32],
        metric: crate::DistanceMetric,
        restriction: PendingRestriction<'q>,
    ) -> impl Fn(usize) -> Result<Vec<ScoredResult>> + 'q {
        let first = std::cell::RefCell::new(Some(first));
        let quality = self.runtime_search_quality();
        move |n| match first.borrow_mut().take() {
            Some(first) => Ok(first),
            None => self.search_merging_pending(query, n, metric, restriction, |m| {
                self.storage.index.search_with_quality(query, m, quality)
            }),
        }
    }

    /// Performs batch search for multiple query vectors in parallel.
    ///
    /// This method is optimized for high throughput using parallel index traversal.
    /// Each query returns its `k` nearest live rows, as [`Self::search`]
    /// does: a deleted or TTL-expired row takes no place among them.
    ///
    /// # Arguments
    ///
    /// * `queries` - List of query vector slices
    /// * `k` - Maximum number of results per query
    ///
    /// # Returns
    ///
    /// Vector of search results for each query, with full point data.
    ///
    /// # Errors
    ///
    /// Returns an error if any query vector dimension doesn't match the collection.
    pub fn search_batch_parallel(
        &self,
        queries: &[&[f32]],
        k: usize,
    ) -> Result<Vec<Vec<SearchResult>>> {
        // Validate every query first.
        let metric = self.validate_queries_and_read_metric(queries)?;

        // Perf: Use parallel HNSW search (P0 optimization)
        let restrictions = std::iter::repeat(PendingRestriction::NONE);
        let first = self.batch_search_merging_pending(queries, k, metric, restrictions)?;
        first
            .into_iter()
            .zip(queries)
            .map(|(first, query)| {
                self.nearest_live(
                    &Nearest::rows(query, metric, k),
                    self.after_batch(first, query, metric, PendingRestriction::NONE),
                )
            })
            .collect()
    }

    /// Performs multi-query search with result fusion.
    ///
    /// This method executes parallel searches for multiple query vectors and fuses
    /// the results using the specified fusion strategy. Ideal for Multiple Query
    /// Generation (MQG) pipelines where multiple reformulations of a user query
    /// are searched simultaneously.
    ///
    /// # Arguments
    ///
    /// * `vectors` - Slice of query vectors (all must have same dimension)
    /// * `top_k` - Maximum number of results to return after fusion
    /// * `fusion` - Strategy for combining results (Average, Maximum, RRF, Weighted)
    /// * `filter` - Optional metadata filter to apply to all queries
    ///
    /// # Returns
    ///
    /// Vector of `SearchResult` sorted best-first (descending fused score for
    /// similarity metrics, ascending fused distance for Euclidean/Hamming).
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - `vectors` is empty
    /// - Any vector has incorrect dimension
    /// - More than 10 vectors are provided (configurable limit)
    #[allow(clippy::needless_pass_by_value)]
    pub fn multi_query_search(
        &self,
        vectors: &[&[f32]],
        top_k: usize,
        fusion: crate::fusion::FusionStrategy,
        filter: Option<&crate::filter::Filter>,
    ) -> Result<Vec<SearchResult>> {
        let metric = self.validate_multi_query_inputs(vectors)?;
        let depth = crate::fusion::branch_depth(top_k);

        // Each branch holds the `depth` best points that pass the filter:
        // `search_with_filter` oversamples by the filter's selectivity and
        // prefilters through the secondary indexes. Cutting each branch to
        // its `depth` nearest points first and filtering after would leave a
        // selective filter almost nothing to fuse.
        let filtered = match filter {
            None => self.nearest_live_branches(vectors, depth, metric)?,
            Some(filter) => vectors
                .iter()
                .map(|vector| {
                    self.search_with_filter(vector, depth, filter)
                        .map(|results| results.into_iter().map(|r| (r.point.id, r.score)).collect())
                })
                .collect::<Result<_>>()?,
        };

        // Every branch carries this collection's metric scores, so score-level
        // strategies must honor the metric's polarity (#2102).
        let directions = vec![metric.score_direction(); filtered.len()];
        let fused = fusion.fuse_with_directions(filtered, &directions)?;

        Ok(self.first_rows(fused, top_k, RowUniverse::Vectors))
    }

    /// Validates inputs for `multi_query_search` and returns the distance metric.
    fn validate_multi_query_inputs(&self, vectors: &[&[f32]]) -> Result<crate::DistanceMetric> {
        crate::fusion::validate_fused_vector_count(vectors.len())?;
        self.validate_queries_and_read_metric(vectors)
    }

    /// Each query's `depth` nearest live rows, as `(id, score)`: the
    /// branches a multi-query search fuses, so a deleted or TTL-expired row
    /// takes no place in one ([`Self::nearest_live_rows`]).
    ///
    /// # Errors
    ///
    /// What the index search returns.
    fn nearest_live_branches(
        &self,
        vectors: &[&[f32]],
        depth: usize,
        metric: crate::DistanceMetric,
    ) -> Result<Vec<Vec<(u64, f32)>>> {
        let restrictions = std::iter::repeat(PendingRestriction::NONE);
        let first = self.batch_search_merging_pending(vectors, depth, metric, restrictions)?;
        first
            .into_iter()
            .zip(vectors)
            .map(|(first, vector)| {
                let live = self.nearest_live_ids(
                    &Nearest::rows(vector, metric, depth),
                    self.after_batch(first, vector, metric, PendingRestriction::NONE),
                )?;
                Ok(live.into_iter().map(Into::into).collect())
            })
            .collect()
    }

    /// The batch form of [`search_merging_pending`](Self::search_merging_pending):
    /// one pending snapshot before the parallel index search, then, per query,
    /// a refetch while unindexed ids crowd its `k` (in parallel) and the merge
    /// under that query's restriction (on the caller; one restriction per
    /// query, in order).
    ///
    /// # Errors
    ///
    /// Returns what the index search returns.
    fn batch_search_merging_pending<'r>(
        &self,
        queries: &[&[f32]],
        k: usize,
        metric: crate::DistanceMetric,
        restrictions: impl Iterator<Item = PendingRestriction<'r>>,
    ) -> Result<Vec<Vec<ScoredResult>>> {
        let quality = self.runtime_search_quality();
        let pending = self.snapshot_pending();
        let index_results = self
            .storage
            .index
            .search_batch_parallel(queries, k, quality)?;
        // Only the refetches run in parallel: each is an index search, which
        // takes no collection lock. The merge scores pending points under the
        // `vector_storage` read guard, so it runs here, on the caller: no job
        // on a rayon pool takes a collection lock.
        let refetched: Vec<Vec<ScoredResult>> = pools::map_owned(
            index_results.into_iter().zip(queries).collect(),
            |(first, query)| {
                Self::index_results_past_pending(&pending, k, first, |n| {
                    self.storage.index.search_with_quality(query, n, quality)
                })
            },
        )
        .into_iter()
        .collect::<Result<_>>()?;
        Ok(refetched
            .into_iter()
            .zip(queries)
            .zip(restrictions)
            .map(|((results, query), restriction)| {
                self.merge_pending(results, &pending, query, k, metric, restriction)
            })
            .collect())
    }

    /// Performs multi-query search returning only IDs and fused scores.
    ///
    /// This is a faster variant of `multi_query_search` that skips fetching
    /// vector and payload data. Use when you only need document IDs.
    ///
    /// Reuses [`validate_multi_query_inputs`](Self::validate_multi_query_inputs),
    /// [`crate::fusion::branch_depth`], and
    /// [`nearest_live_branches`](Self::nearest_live_branches) to eliminate
    /// duplication with `multi_query_search`: every branch holds live rows
    /// only.
    ///
    /// # Arguments
    ///
    /// * `vectors` - Slice of query vectors
    /// * `top_k` - Maximum number of results
    /// * `fusion` - Fusion strategy
    ///
    /// # Returns
    ///
    /// Vector of `(id, fused_score)` tuples sorted by score descending.
    ///
    /// # Errors
    ///
    /// Returns an error if vectors is empty, exceeds max limit, or has dimension mismatch.
    #[allow(clippy::needless_pass_by_value)]
    pub fn multi_query_search_ids(
        &self,
        vectors: &[&[f32]],
        top_k: usize,
        fusion: crate::fusion::FusionStrategy,
    ) -> Result<Vec<(u64, f32)>> {
        let metric = self.validate_multi_query_inputs(vectors)?;
        let batch_results =
            self.nearest_live_branches(vectors, crate::fusion::branch_depth(top_k), metric)?;

        // Same polarity contract as `multi_query_search` (#2102).
        let directions = vec![metric.score_direction(); batch_results.len()];
        let fused = fusion.fuse_with_directions(batch_results, &directions)?;

        Ok(fused.into_iter().take(top_k).collect())
    }
}
