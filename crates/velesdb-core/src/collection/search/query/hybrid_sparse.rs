//! Hybrid dense+sparse search execution with RRF/RSF fusion.
//!
//! Extracts `SparseVectorSearch` from the AST condition tree, executes
//! sparse (optionally filtered) and dense searches in parallel, and fuses
//! results using the requested fusion strategy.

use crate::collection::expiry::{is_payload_expired, now_unix_secs};
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

/// The two ranked branches of a hybrid search, before fusion: the dense hits
/// as `(id, score)` pairs, the sparse hits as scored documents.
type HybridBranches = (Vec<(u64, f32)>, Vec<crate::index::sparse::ScoredDoc>);

impl Collection {
    // ------------------------------------------------------------------
    // Condition extraction
    // ------------------------------------------------------------------

    /// Recursively walks the condition tree to find a `SparseVectorSearch` node.
    ///
    /// **First-wins semantics**: when multiple `SparseVectorSearch` nodes exist
    /// in a compound condition (e.g. `AND`), only the left-most one is
    /// returned, and it is the only sparse branch the query runs. The VelesQL
    /// spec defines one sparse branch per hybrid query but does not reject a
    /// second `SPARSE_NEAR`, so nothing upstream refuses one either.
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
                    Error::Query(format!("Missing sparse vector parameter: ${name}"))
                })?;
                let obj = val.as_object().ok_or_else(|| {
                    Error::Query(format!(
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
            Error::Query(format!(
                "Invalid sparse vector parameter ${name}.indices: {e}"
            ))
        })?;
        let values: Vec<f32> = serde_json::from_value(values_val.clone()).map_err(|e| {
            Error::Query(format!(
                "Invalid sparse vector parameter ${name}.values: {e}"
            ))
        })?;
        if indices.len() != values.len() {
            return Err(Error::Query(format!(
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
                Error::Query(format!(
                    "Invalid sparse vector parameter ${name}: key '{k}' is not a valid u32 index"
                ))
            })?;
            #[allow(clippy::cast_possible_truncation)]
            let weight = v.as_f64().ok_or_else(|| {
                Error::Query(format!(
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
    pub(crate) fn execute_sparse_search(
        &self,
        svs: &SparseVectorSearch,
        params: &std::collections::HashMap<String, serde_json::Value>,
        filter_condition: Option<&Condition>,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        self.execute_sparse_search_in_anchors(svs, params, filter_condition, limit, None)
    }

    /// Execute a sparse-only search, optionally filtered by payload
    /// conditions and/or restricted to a GraphFirst anchor-id set.
    ///
    /// With `anchors`, the sparse index's per-id filter makes the fetch
    /// exact at `limit` *within* the graph matches — no over-fetch window.
    ///
    /// # Lock ordering
    ///
    /// `payload_storage(3)` is acquired before `sparse_indexes(9)` to respect
    /// the canonical lock order defined in `docs/CONCURRENCY_MODEL.md`.
    pub(crate) fn execute_sparse_search_in_anchors(
        &self,
        svs: &SparseVectorSearch,
        params: &std::collections::HashMap<String, serde_json::Value>,
        filter_condition: Option<&Condition>,
        limit: usize,
        anchors: Option<&std::collections::HashSet<u64>>,
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

        let results = if metadata_filter.is_some() || anchors.is_some() {
            self.sparse_search_with_id_filter(
                index_name,
                &query_vec,
                limit,
                metadata_filter.as_ref(),
                anchors,
            )?
        } else {
            let indexes = self.query.sparse_indexes.read(); // lock 9 only (no payload needed)
            let index = indexes
                .get(index_name)
                .ok_or_else(|| resolve::sparse_index_not_found(index_name))?;
            let r = sparse_search(index, &query_vec, limit);
            drop(indexes);
            r
        };

        Ok(self.resolve_sparse_results(&results, limit))
    }

    /// Runs a sparse search with a per-id filter combining anchor membership
    /// and the metadata payload filter.
    ///
    /// # Lock ordering
    ///
    /// `payload_storage(3)` is acquired before `sparse_indexes(9)` to respect
    /// the canonical lock order defined in `docs/CONCURRENCY_MODEL.md`.
    // `payload_storage`(3) and `sparse_indexes`(9) are both held across the
    // search because `filter_fn` closes over them and runs per candidate
    // inside it. Releasing either early would leave the closure reading a
    // guard that is gone; the numbered order is the documented one.
    #[expect(clippy::significant_drop_tightening)]
    fn sparse_search_with_id_filter(
        &self,
        index_name: &str,
        query_vec: &SparseVector,
        limit: usize,
        metadata_filter: Option<&crate::filter::Filter>,
        anchors: Option<&std::collections::HashSet<u64>>,
    ) -> Result<Vec<crate::sparse_index::ScoredDoc>> {
        let payload_storage = metadata_filter.map(|_| self.storage.payload_storage.read()); // lock 3
        let indexes = self.query.sparse_indexes.read(); // lock 9
        let index = indexes
            .get(index_name)
            .ok_or_else(|| resolve::sparse_index_not_found(index_name))?;
        let filter_fn = |id: u64| {
            if anchors.is_some_and(|a| !a.contains(&id)) {
                return false;
            }
            match (metadata_filter, &payload_storage) {
                (Some(filter), Some(storage)) => {
                    let payload = storage.retrieve(id).ok().flatten();
                    let p = payload.as_ref().unwrap_or(&serde_json::Value::Null);
                    filter.matches(p)
                }
                _ => true,
            }
        };
        Ok(sparse_search_filtered(
            index,
            query_vec,
            limit,
            Some(&filter_fn),
        ))
    }

    // ------------------------------------------------------------------
    // Hybrid dense+sparse execution
    // ------------------------------------------------------------------

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
        )?;

        self.fuse_dense_sparse(dense_results, &sparse_results, limit, strategy)
    }

    /// Execute dense and sparse branches, optionally in parallel.
    ///
    /// Either branch's failure fails the hybrid search — a dense query of the
    /// wrong dimension, an unknown sparse index — instead of leaving that
    /// branch empty for [`fuse_dense_sparse`](Self::fuse_dense_sparse) to
    /// degrade over. A branch that ran and matched nothing is still an empty,
    /// successful branch. When both fail, the dense error is reported.
    ///
    /// # Lock ordering
    ///
    /// When a `metadata_filter` is present, `payload_storage(3)` is acquired
    /// before `sparse_indexes(9)` (see `sparse_branch` and
    /// `docs/CONCURRENCY_MODEL.md`).
    ///
    /// In the `persistence` path, the two branches run via `rayon::join`; the
    /// dense closure never touches `sparse_indexes`, so there is no ordering
    /// conflict between the two parallel closures.
    ///
    /// # Errors
    ///
    /// Returns the dense branch's error (e.g. a dimension mismatch), else the
    /// sparse branch's (e.g. the named sparse index does not exist).
    pub(crate) fn execute_both_branches(
        &self,
        dense_vector: &[f32],
        sparse_query: &SparseVector,
        index_name: &str,
        candidate_k: usize,
        metadata_filter: Option<&crate::filter::Filter>,
    ) -> Result<HybridBranches> {
        let dense = || self.dense_branch(dense_vector, candidate_k, metadata_filter);
        let sparse = || self.sparse_branch(sparse_query, index_name, candidate_k, metadata_filter);
        #[cfg(feature = "persistence")]
        let (dense, sparse) = rayon::join(dense, sparse);
        #[cfg(not(feature = "persistence"))]
        let (dense, sparse) = (dense(), sparse());
        Ok((dense?, sparse?))
    }

    /// The dense (HNSW) branch of a hybrid search, as `(id, score)` pairs.
    ///
    /// The metadata filter bounds BOTH branches. The dense branch previously
    /// ignored it entirely (only sparse filtered), so dense-only candidates the
    /// filter excludes leaked into fusion. `search_ids` acquires
    /// `vector_storage(2)` only — no conflict with the sparse branch's locks.
    fn dense_branch(
        &self,
        dense_vector: &[f32],
        candidate_k: usize,
        metadata_filter: Option<&crate::filter::Filter>,
    ) -> Result<Vec<(u64, f32)>> {
        Ok(match metadata_filter {
            Some(filter) => self
                .search_with_filter(dense_vector, candidate_k, filter)?
                .into_iter()
                .map(|r| (r.point.id, r.score))
                .collect(),
            None => self
                .search_ids(dense_vector, candidate_k)?
                .into_iter()
                .map(Into::into)
                .collect(),
        })
    }

    /// The sparse branch of a hybrid search on the named sparse index.
    ///
    /// # Errors
    ///
    /// Returns an error if the named sparse index does not exist, as the
    /// sparse-only search does.
    // With a filter, payload_storage(3) is taken before sparse_indexes(9) and
    // both are held for the whole filtered search, because the filter closure
    // reads through them per candidate.
    #[expect(clippy::significant_drop_tightening)]
    fn sparse_branch(
        &self,
        sparse_query: &SparseVector,
        index_name: &str,
        candidate_k: usize,
        metadata_filter: Option<&crate::filter::Filter>,
    ) -> Result<Vec<crate::index::sparse::ScoredDoc>> {
        // LOCK ORDER: payload_storage(3) before sparse_indexes(9).
        let payload_storage = metadata_filter.map(|_| self.storage.payload_storage.read());
        let indexes = self.query.sparse_indexes.read();
        let index = indexes
            .get(index_name)
            .ok_or_else(|| resolve::sparse_index_not_found(index_name))?;
        let (Some(filter), Some(payload_storage)) = (metadata_filter, payload_storage.as_ref())
        else {
            return Ok(sparse_search(index, sparse_query, candidate_k));
        };
        let filter_fn = |id: u64| {
            let payload = payload_storage.retrieve(id).ok().flatten();
            let p = payload.as_ref().unwrap_or(&serde_json::Value::Null);
            filter.matches(p)
        };
        Ok(sparse_search_filtered(
            index,
            sparse_query,
            candidate_k,
            Some(&filter_fn),
        ))
    }

    /// Fuse dense and sparse branch results into ranked `SearchResult`s.
    ///
    /// A branch that came back empty is still fused, as an empty list: the
    /// scores returned always have the direction
    /// [`FusionStrategy::fused_direction`] states for a dense and a sparse
    /// branch, so a caller reading them as fused scores never meets the dense
    /// branch's raw distances instead. Shared by the VelesQL hybrid path and
    /// the raw `SparseVector` SDK entry points.
    ///
    /// # Errors
    ///
    /// Returns an error if the fusion strategy itself fails.
    pub(crate) fn fuse_dense_sparse(
        &self,
        dense_results: Vec<(u64, f32)>,
        sparse_results: &[crate::index::sparse::ScoredDoc],
        limit: usize,
        strategy: &FusionStrategy,
    ) -> Result<Vec<SearchResult>> {
        if dense_results.is_empty() && sparse_results.is_empty() {
            return Ok(Vec::new());
        }

        let sparse_tuples: Vec<(u64, f32)> = sparse_results
            .iter()
            .map(|sd| (sd.doc_id, sd.score))
            .collect();

        // Dense scores carry the collection metric's polarity (distances for
        // Euclidean/Hamming); sparse scores are always higher-is-better
        // relevance. Score-level strategies must align them (#2102).
        let directions = [
            self.storage.config.read().metric.score_direction(),
            crate::fusion::ScoreDirection::HigherIsBetter,
        ];
        let fused = strategy
            .fuse_with_directions(vec![dense_results, sparse_tuples], &directions)
            .map_err(|e| Error::Config(format!("Fusion error: {e}")))?;

        Ok(self.resolve_fused_results(&fused, limit))
    }

    // ------------------------------------------------------------------
    // Result resolution helpers
    // ------------------------------------------------------------------

    /// Hydrate a sequence of `(id, score)` pairs into full `SearchResult`s.
    ///
    /// Shared by both sparse-only and fused search paths. Acquires
    /// `vector_storage(2)` and `payload_storage(3)` read locks.
    fn resolve_id_score_pairs(
        &self,
        pairs: impl Iterator<Item = (u64, f32)>,
        limit: usize,
        capacity_hint: usize,
    ) -> Vec<SearchResult> {
        let vector_storage = self.storage.vector_storage.read();
        let payload_storage = self.storage.payload_storage.read();
        let now_secs = now_unix_secs();

        let mut out = Vec::with_capacity(capacity_hint.min(limit));
        for (id, score) in pairs.take(limit) {
            let vector = vector_storage
                .retrieve(id)
                .ok()
                .flatten()
                .unwrap_or_default();
            let payload = payload_storage.retrieve(id).ok().flatten();
            if is_payload_expired(payload.as_ref(), now_secs) {
                continue;
            }
            let point = Point {
                id,
                vector,
                payload,
                sparse_vectors: None,
            };
            out.push(SearchResult::new(point, score));
        }
        // The loop is both guards' last use; nothing below reads storage.
        drop(payload_storage);
        drop(vector_storage);
        out
    }

    /// Resolve `ScoredDoc` results to full `SearchResult` with Point data.
    pub(crate) fn resolve_sparse_results(
        &self,
        results: &[crate::index::sparse::ScoredDoc],
        limit: usize,
    ) -> Vec<SearchResult> {
        self.resolve_id_score_pairs(
            results.iter().map(|sd| (sd.doc_id, sd.score)),
            limit,
            results.len(),
        )
    }

    /// Resolve fused `(id, score)` tuples to `SearchResult`.
    pub(crate) fn resolve_fused_results(
        &self,
        fused: &[(u64, f32)],
        limit: usize,
    ) -> Vec<SearchResult> {
        self.resolve_id_score_pairs(fused.iter().copied(), limit, fused.len())
    }
}
