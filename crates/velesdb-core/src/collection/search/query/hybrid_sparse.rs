//! Hybrid dense+sparse search execution with RRF/RSF fusion.
//!
//! Extracts `SparseVectorSearch` from the AST condition tree, executes
//! sparse (optionally filtered) and dense searches in parallel, and fuses
//! results using the requested fusion strategy.

use crate::collection::search::resolve;
use crate::collection::types::Collection;
use crate::error::{Error, Result};
use crate::fusion::FusionStrategy;
use crate::index::sparse::{
    sparse_search, sparse_search_filtered, SparseVector, DEFAULT_SPARSE_INDEX_NAME,
};
use crate::point::{Point, SearchResult};
use crate::storage::{PayloadStorage, VectorStorage};
use crate::velesql::{Condition, SparseVectorExpr, SparseVectorSearch};

impl Collection {
    // ------------------------------------------------------------------
    // Condition extraction
    // ------------------------------------------------------------------

    /// Recursively walks the condition tree to find a `SparseVectorSearch` node.
    ///
    /// **First-wins semantics**: when multiple `SparseVectorSearch` nodes exist
    /// in a compound condition (e.g. `AND`), only the left-most one is returned.
    /// Queries containing more than one `SPARSE_NEAR` clause are currently not
    /// supported; callers that need to detect this case should call
    /// [`Self::validate_single_sparse_search`] before invoking this function.
    pub(crate) fn extract_sparse_vector_search(
        condition: &Condition,
    ) -> Option<&SparseVectorSearch> {
        match condition {
            Condition::SparseVectorSearch(svs) => Some(svs),
            Condition::And(left, right) | Condition::Or(left, right) => {
                Self::extract_sparse_vector_search(left)
                    .or_else(|| Self::extract_sparse_vector_search(right))
            }
            Condition::Group(inner) | Condition::Not(inner) => {
                Self::extract_sparse_vector_search(inner)
            }
            _ => None,
        }
    }

    /// Counts `SparseVectorSearch` nodes in the condition tree.
    ///
    /// Returns an error if more than one `SPARSE_NEAR` clause is found,
    /// because the planner only handles a single sparse branch per query.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the condition contains more than one
    /// `SparseVectorSearch` node (ambiguous multi-sparse query).
    #[allow(dead_code)]
    pub(crate) fn validate_single_sparse_search(condition: &Condition) -> Result<()> {
        fn count(cond: &Condition) -> usize {
            match cond {
                Condition::SparseVectorSearch(_) => 1,
                Condition::And(l, r) | Condition::Or(l, r) => count(l) + count(r),
                Condition::Group(inner) | Condition::Not(inner) => count(inner),
                _ => 0,
            }
        }

        let n = count(condition);
        if n > 1 {
            return Err(Error::Config(format!(
                "Query contains {n} SPARSE_NEAR clauses; only one is supported per query. \
                 Use separate queries for each sparse search."
            )));
        }
        Ok(())
    }

    /// Resolve a `SparseVectorExpr` to a concrete `SparseVector`.
    ///
    /// Accepts two JSON formats:
    /// - Structured: `{"indices": [1,2,3], "values": [0.5, 0.3, 0.1]}`
    /// - Shorthand:  `{"12": 0.8, "45": 0.3}`
    ///
    /// # Errors
    ///
    /// Returns an error if the bind parameter is missing or has the wrong type.
    pub(crate) fn resolve_sparse_vector(
        expr: &SparseVectorExpr,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<SparseVector> {
        match expr {
            SparseVectorExpr::Literal(sv) => Ok(sv.clone()),
            SparseVectorExpr::Parameter(name) => {
                let val = params.get(name).ok_or_else(|| {
                    Error::Config(format!("Missing sparse vector parameter: ${name}"))
                })?;
                let obj = val.as_object().ok_or_else(|| {
                    Error::Config(format!(
                        "Invalid sparse vector parameter ${name}: expected object with \
                         indices/values or {{index: weight}} map"
                    ))
                })?;

                // Try structured format first: {"indices": [...], "values": [...]}
                if let Some(sv) = Self::try_parse_structured_sparse(obj, name)? {
                    return Ok(sv);
                }

                // Shorthand: {"12": 0.8, "45": 0.3}
                Self::parse_shorthand_sparse(obj, name)
            }
        }
    }

    /// Tries to parse a structured sparse vector format with `indices` and `values` arrays.
    fn try_parse_structured_sparse(
        obj: &serde_json::Map<String, serde_json::Value>,
        name: &str,
    ) -> Result<Option<SparseVector>> {
        let (Some(indices_val), Some(values_val)) = (obj.get("indices"), obj.get("values")) else {
            return Ok(None);
        };
        let indices: Vec<u32> = serde_json::from_value(indices_val.clone()).map_err(|e| {
            Error::Config(format!(
                "Invalid sparse vector parameter ${name}.indices: {e}"
            ))
        })?;
        let values: Vec<f32> = serde_json::from_value(values_val.clone()).map_err(|e| {
            Error::Config(format!(
                "Invalid sparse vector parameter ${name}.values: {e}"
            ))
        })?;
        if indices.len() != values.len() {
            return Err(Error::Config(format!(
                "Sparse vector parameter ${name}: indices and values must have equal length"
            )));
        }
        Ok(Some(SparseVector::new(
            indices.into_iter().zip(values).collect(),
        )))
    }

    /// Parses shorthand sparse vector format: `{"12": 0.8, "45": 0.3}`.
    fn parse_shorthand_sparse(
        obj: &serde_json::Map<String, serde_json::Value>,
        name: &str,
    ) -> Result<SparseVector> {
        let mut pairs = Vec::with_capacity(obj.len());
        for (k, v) in obj {
            let idx: u32 = k.parse().map_err(|_| {
                Error::Config(format!(
                    "Invalid sparse vector parameter ${name}: key '{k}' is not a valid u32 index"
                ))
            })?;
            #[allow(clippy::cast_possible_truncation)]
            let weight = v.as_f64().ok_or_else(|| {
                Error::Config(format!(
                    "Invalid sparse vector parameter ${name}: value for key '{k}' is not a number"
                ))
            })? as f32;
            pairs.push((idx, weight));
        }
        Ok(SparseVector::new(pairs))
    }

    // ------------------------------------------------------------------
    // Sparse-only execution
    // ------------------------------------------------------------------

    /// Execute a sparse-only search, optionally filtered by payload conditions.
    ///
    /// # Lock ordering
    ///
    /// `payload_storage(3)` is acquired before `sparse_indexes(9)` to respect
    /// the canonical lock order defined in `docs/CONCURRENCY_MODEL.md`.
    pub(crate) fn execute_sparse_search(
        &self,
        svs: &SparseVectorSearch,
        params: &std::collections::HashMap<String, serde_json::Value>,
        filter_condition: Option<&Condition>,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        let query_vec = Self::resolve_sparse_vector(&svs.vector, params)?;
        let index_name = svs
            .index_name
            .as_deref()
            .unwrap_or(DEFAULT_SPARSE_INDEX_NAME);

        // Build payload filter if there are non-vector metadata conditions.
        let metadata_filter = filter_condition
            .and_then(Self::extract_metadata_filter)
            .map(|cond| crate::filter::Filter::new(crate::filter::Condition::from(cond)));

        // LOCK ORDER: payload_storage(3) before sparse_indexes(9).
        let results = if let Some(ref filter) = metadata_filter {
            let payload_storage = self.payload_storage.read(); // lock 3
            let indexes = self.sparse_indexes.read(); // lock 9
            let index = indexes
                .get(index_name)
                .ok_or_else(|| resolve::sparse_index_not_found(index_name))?;
            let filter_fn = |id: u64| {
                let payload = payload_storage.retrieve(id).ok().flatten();
                let p = payload.as_ref().unwrap_or(&serde_json::Value::Null);
                filter.matches(p)
            };
            let r = sparse_search_filtered(index, &query_vec, limit, Some(&filter_fn));
            drop(indexes);
            drop(payload_storage);
            r
        } else {
            let indexes = self.sparse_indexes.read(); // lock 9 only (no payload needed)
            let index = indexes
                .get(index_name)
                .ok_or_else(|| resolve::sparse_index_not_found(index_name))?;
            let r = sparse_search(index, &query_vec, limit);
            drop(indexes);
            r
        };

        Ok(self.resolve_sparse_results(&results, limit))
    }

    // ------------------------------------------------------------------
    // Hybrid dense+sparse execution
    // ------------------------------------------------------------------

    /// Execute hybrid dense+sparse search with the default RRF strategy.
    ///
    /// Runs both branches (optionally in parallel via `rayon::join`), then
    /// fuses results using RRF with k=60.
    ///
    /// Currently only exercised by integration tests; production callers use
    /// [`Self::execute_hybrid_search_with_strategy`] directly.
    #[allow(dead_code)]
    pub(crate) fn execute_hybrid_search(
        &self,
        dense_vector: &[f32],
        svs: &SparseVectorSearch,
        params: &std::collections::HashMap<String, serde_json::Value>,
        filter_condition: Option<&Condition>,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        self.execute_hybrid_search_with_strategy(
            dense_vector,
            svs,
            params,
            filter_condition,
            limit,
            &FusionStrategy::rrf_default(),
        )
    }

    /// Execute hybrid search with an explicit fusion strategy.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn execute_hybrid_search_with_strategy(
        &self,
        dense_vector: &[f32],
        svs: &SparseVectorSearch,
        params: &std::collections::HashMap<String, serde_json::Value>,
        filter_condition: Option<&Condition>,
        limit: usize,
        strategy: &FusionStrategy,
    ) -> Result<Vec<SearchResult>> {
        let sparse_query = Self::resolve_sparse_vector(&svs.vector, params)?;
        let index_name = svs
            .index_name
            .as_deref()
            .unwrap_or(DEFAULT_SPARSE_INDEX_NAME);
        // Oversampling factor: 2× the requested limit.
        //
        // Both the dense (HNSW) and sparse branches can independently miss the
        // globally optimal result; fetching more candidates from each branch
        // compensates for the blind spots of the other. 2× is chosen to be
        // conservative: `sparse_search_filtered` internally uses a higher
        // oversampling factor (4×–8×) to account for payload-filter selectivity,
        // so the 2× here operates at the fusion level — not the per-branch level.
        // Increasing this beyond 2× improves recall marginally at the cost of
        // more fusion work; it can be tuned per query via strategy configuration.
        let candidate_k = limit.saturating_mul(2).max(limit + 10);

        // Pre-build metadata filter for the sparse branch.
        let metadata_filter = filter_condition
            .and_then(Self::extract_metadata_filter)
            .map(|cond| crate::filter::Filter::new(crate::filter::Condition::from(cond)));

        // Execute both branches.
        let (dense_results, sparse_results) = self.execute_both_branches(
            dense_vector,
            &sparse_query,
            index_name,
            candidate_k,
            metadata_filter.as_ref(),
        );

        // Graceful degradation: if one branch is empty, return the other.
        if dense_results.is_empty() && sparse_results.is_empty() {
            return Ok(Vec::new());
        }
        if dense_results.is_empty() {
            let scored: Vec<(u64, f32)> = sparse_results
                .iter()
                .map(|sd| (sd.doc_id, sd.score))
                .collect();
            return Ok(self.resolve_fused_results(&scored, limit));
        }
        if sparse_results.is_empty() {
            return Ok(self.resolve_fused_results(&dense_results, limit));
        }

        let sparse_tuples: Vec<(u64, f32)> = sparse_results
            .iter()
            .map(|sd| (sd.doc_id, sd.score))
            .collect();

        let fused = strategy
            .fuse(vec![dense_results, sparse_tuples])
            .map_err(|e| Error::Config(format!("Fusion error: {e}")))?;

        Ok(self.resolve_fused_results(&fused, limit))
    }

    /// Execute dense and sparse branches, optionally in parallel.
    ///
    /// # Lock ordering
    ///
    /// When a `metadata_filter` is present, `payload_storage(3)` is acquired
    /// before `sparse_indexes(9)` in both execution paths to respect the
    /// canonical lock order (see `docs/CONCURRENCY_MODEL.md`).
    ///
    /// In the `persistence` path, the two branches run via `rayon::join`; the
    /// dense closure never touches `sparse_indexes`, so there is no ordering
    /// conflict between the two parallel closures.
    pub(crate) fn execute_both_branches(
        &self,
        dense_vector: &[f32],
        sparse_query: &SparseVector,
        index_name: &str,
        candidate_k: usize,
        metadata_filter: Option<&crate::filter::Filter>,
    ) -> (Vec<(u64, f32)>, Vec<crate::index::sparse::ScoredDoc>) {
        #[cfg(feature = "persistence")]
        {
            // Parallel execution via rayon::join.
            // The dense closure uses search_ids() which internally acquires
            // vector_storage(2) only — no conflict with sparse locks.
            // The sparse closure acquires payload_storage(3) before
            // sparse_indexes(9) when filtering is needed.
            let (dense, sparse) = rayon::join(
                || {
                    self.search_ids(dense_vector, candidate_k)
                        .unwrap_or_default()
                        .into_iter()
                        .map(Into::into)
                        .collect()
                },
                || {
                    if let Some(filter) = metadata_filter {
                        // LOCK ORDER: payload_storage(3) before sparse_indexes(9).
                        let payload_storage = self.payload_storage.read();
                        let indexes = self.sparse_indexes.read();
                        let Some(index) = indexes.get(index_name) else {
                            return Vec::new();
                        };
                        let filter_fn = |id: u64| {
                            let payload = payload_storage.retrieve(id).ok().flatten();
                            let p = payload.as_ref().unwrap_or(&serde_json::Value::Null);
                            filter.matches(p)
                        };
                        sparse_search_filtered(index, sparse_query, candidate_k, Some(&filter_fn))
                    } else {
                        let indexes = self.sparse_indexes.read();
                        let Some(index) = indexes.get(index_name) else {
                            return Vec::new();
                        };
                        sparse_search(index, sparse_query, candidate_k)
                    }
                },
            );
            (dense, sparse)
        }

        #[cfg(not(feature = "persistence"))]
        {
            // Sequential fallback (no rayon).
            let dense: Vec<(u64, f32)> = self
                .search_ids(dense_vector, candidate_k)
                .unwrap_or_default()
                .into_iter()
                .map(Into::into)
                .collect();
            let sparse = if let Some(filter) = metadata_filter {
                // LOCK ORDER: payload_storage(3) before sparse_indexes(9).
                let payload_storage = self.payload_storage.read();
                let indexes = self.sparse_indexes.read();
                if let Some(index) = indexes.get(index_name) {
                    let filter_fn = |id: u64| {
                        let payload = payload_storage.retrieve(id).ok().flatten();
                        let p = payload.as_ref().unwrap_or(&serde_json::Value::Null);
                        filter.matches(p)
                    };
                    sparse_search_filtered(index, sparse_query, candidate_k, Some(&filter_fn))
                } else {
                    Vec::new()
                }
            } else {
                let indexes = self.sparse_indexes.read();
                if let Some(index) = indexes.get(index_name) {
                    sparse_search(index, sparse_query, candidate_k)
                } else {
                    Vec::new()
                }
            };
            (dense, sparse)
        }
    }

    // ------------------------------------------------------------------
    // Result resolution helpers
    // ------------------------------------------------------------------

    /// Resolve `ScoredDoc` results to full `SearchResult` with Point data.
    pub(crate) fn resolve_sparse_results(
        &self,
        results: &[crate::index::sparse::ScoredDoc],
        limit: usize,
    ) -> Vec<SearchResult> {
        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();

        let mut out = Vec::with_capacity(results.len().min(limit));
        for sd in results.iter().take(limit) {
            let vector = vector_storage
                .retrieve(sd.doc_id)
                .ok()
                .flatten()
                .unwrap_or_default();
            let payload = payload_storage.retrieve(sd.doc_id).ok().flatten();
            let point = Point {
                id: sd.doc_id,
                vector,
                payload,
                sparse_vectors: None,
            };
            out.push(SearchResult::new(point, sd.score));
        }
        out
    }

    /// Resolve fused `(id, score)` tuples to `SearchResult`.
    pub(crate) fn resolve_fused_results(
        &self,
        fused: &[(u64, f32)],
        limit: usize,
    ) -> Vec<SearchResult> {
        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();

        let mut out = Vec::with_capacity(fused.len().min(limit));
        for &(id, score) in fused.iter().take(limit) {
            let vector = vector_storage
                .retrieve(id)
                .ok()
                .flatten()
                .unwrap_or_default();
            let payload = payload_storage.retrieve(id).ok().flatten();
            let point = Point {
                id,
                vector,
                payload,
                sparse_vectors: None,
            };
            out.push(SearchResult::new(point, score));
        }
        out
    }
}
