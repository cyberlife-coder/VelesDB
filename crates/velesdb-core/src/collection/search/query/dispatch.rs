//! Query dispatch logic extracted from mod.rs (VP-010, Plan 06-03 Task 6).
//!
//! Contains the main dispatch functions that route `VelesQL` queries to the
//! appropriate search backend based on extracted conditions.

use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::SearchResult;

/// Extracted query context passed from `execute_query()` to dispatch functions.
pub(super) struct QueryContext<'a> {
    pub vector_search: Option<&'a [f32]>,
    pub first_similarity: Option<(String, Vec<f32>, crate::velesql::CompareOp, f64)>,
    pub all_similarity_conditions: &'a [(String, Vec<f32>, crate::velesql::CompareOp, f64)],
    pub filter_condition: Option<&'a crate::velesql::Condition>,
    pub limit: usize,
    pub ef_search: Option<usize>,
    pub overfetch_base: usize,
}

/// Maximum allowed LIMIT value to prevent overflow in over-fetch calculations.
const MAX_LIMIT: usize = 100_000;

impl Collection {
    /// VP-012: Dispatch `NEAR_FUSED` multi-vector fused search.
    ///
    /// Returns `Ok(Some(results))` if `NEAR_FUSED` was found and dispatched,
    /// `Ok(None)` if no `NEAR_FUSED` condition was present.
    pub(super) fn dispatch_fused_search(
        &self,
        cond: &crate::velesql::Condition,
        params: &std::collections::HashMap<String, serde_json::Value>,
        vector_search: Option<&[f32]>,
        similarity_conditions: &[(String, Vec<f32>, crate::velesql::CompareOp, f64)],
        limit: usize,
        order_by: Option<&[crate::velesql::SelectOrderBy]>,
    ) -> Result<Option<Vec<SearchResult>>> {
        let Some((vectors, fusion_strategy)) = self.extract_fused_vector_search(cond, params)?
        else {
            return Ok(None);
        };

        // Validate: NEAR_FUSED cannot be combined with NEAR or similarity()
        if vector_search.is_some() {
            return Err(crate::error::Error::Config(
                "Cannot combine NEAR and NEAR_FUSED in the same query. \
                 Use NEAR for single-vector search or NEAR_FUSED for multi-vector fusion."
                    .to_string(),
            ));
        }
        if !similarity_conditions.is_empty() {
            return Err(crate::error::Error::Config(
                "Cannot combine NEAR_FUSED with similarity() in the same query. \
                 NEAR_FUSED already performs multi-vector fusion with scoring."
                    .to_string(),
            ));
        }

        // Build metadata filter from remaining conditions
        let metadata_filter = Self::extract_metadata_filter(cond)
            .map(|c| crate::filter::Filter::new(crate::filter::Condition::from(c)));

        let vec_slices: Vec<&[f32]> = vectors.iter().map(Vec::as_slice).collect();

        let mut results = self.multi_query_search(
            &vec_slices,
            limit,
            fusion_strategy,
            metadata_filter.as_ref(),
        )?;

        if let Some(order_by) = order_by {
            self.apply_order_by(&mut results, order_by, params)?;
        }
        results.truncate(limit);
        Ok(Some(results))
    }

    /// Dispatch the main query based on extracted vector/similarity/filter conditions.
    ///
    /// This handles the core match statement for all combinations of:
    /// - `similarity()` only, NEAR + `similarity()`, NEAR + MATCH, NEAR + filter,
    ///   pure NEAR, MATCH + filter, metadata-only, and no-WHERE scans.
    #[allow(clippy::too_many_lines)]
    pub(super) fn dispatch_main_query(&self, ctx: &QueryContext<'_>) -> Result<Vec<SearchResult>> {
        match (
            &ctx.vector_search,
            &ctx.first_similarity,
            &ctx.filter_condition,
        ) {
            // similarity() function — cascade filtering
            (None, Some((field, vec, op, threshold)), filter_cond) => self
                .dispatch_similarity_query(
                    field,
                    vec,
                    *op,
                    *threshold,
                    ctx.all_similarity_conditions,
                    *filter_cond,
                    ctx.limit,
                    ctx.overfetch_base,
                ),
            // NEAR + similarity() — find candidates then filter
            (Some(vector), Some((field, sim_vec, op, threshold)), filter_cond) => self
                .dispatch_near_similarity_query(
                    vector,
                    field,
                    sim_vec,
                    *op,
                    *threshold,
                    ctx.all_similarity_conditions,
                    *filter_cond,
                    ctx.limit,
                    ctx.overfetch_base,
                ),
            // NEAR + optional MATCH/filter
            (Some(vector), None, Some(cond)) => {
                self.dispatch_near_with_condition(vector, cond, ctx.limit)
            }
            // Pure NEAR
            (Some(vector), _, None) => {
                if let Some(ef) = ctx.ef_search {
                    self.search_with_ef(vector, ctx.limit, ef)
                } else {
                    self.search(vector, ctx.limit)
                }
            }
            // No vector — MATCH or metadata filter or scan
            (None, None, Some(cond)) => Ok(self.dispatch_text_or_scan(cond, ctx.limit)),
            // No WHERE at all
            (None, None, None) => Ok(self.execute_scan_query(
                &crate::filter::Filter::new(crate::filter::Condition::And { conditions: vec![] }),
                ctx.limit,
            )),
        }
    }

    /// Dispatch similarity()-only queries with cascade filtering.
    #[allow(clippy::too_many_arguments)]
    fn dispatch_similarity_query(
        &self,
        field: &str,
        vec: &[f32],
        op: crate::velesql::CompareOp,
        threshold: f64,
        all_conditions: &[(String, Vec<f32>, crate::velesql::CompareOp, f64)],
        filter_cond: Option<&crate::velesql::Condition>,
        limit: usize,
        overfetch_base: usize,
    ) -> Result<Vec<SearchResult>> {
        if field != "vector" {
            return Err(crate::error::Error::Config(format!(
                "similarity() field '{field}' not found. Only 'vector' field is supported. \
                Multi-vector support is planned for a future release."
            )));
        }

        let overfetch_factor = overfetch_base * all_conditions.len().max(1);
        let candidates_k = limit.saturating_mul(overfetch_factor).min(MAX_LIMIT);
        let candidates = self.search(vec, candidates_k)?;

        let filter_k = limit.saturating_mul(2);
        let mut filtered =
            self.filter_by_similarity(candidates, field, vec, op, threshold, filter_k);

        // Apply remaining similarity conditions (cascade)
        for (sim_field, sim_vec, sim_op, sim_threshold) in all_conditions.iter().skip(1) {
            if sim_field != "vector" {
                return Err(crate::error::Error::Config(format!(
                    "similarity() field '{sim_field}' not found. Only 'vector' field is supported."
                )));
            }
            filtered = self.filter_by_similarity(
                filtered,
                sim_field,
                sim_vec,
                *sim_op,
                *sim_threshold,
                filter_k,
            );
        }

        // Apply additional metadata filters
        Ok(Self::apply_metadata_post_filter(
            filtered,
            filter_cond,
            limit,
        ))
    }

    /// Dispatch NEAR + `similarity()` queries.
    #[allow(clippy::too_many_arguments)]
    fn dispatch_near_similarity_query(
        &self,
        vector: &[f32],
        field: &str,
        sim_vec: &[f32],
        op: crate::velesql::CompareOp,
        threshold: f64,
        all_conditions: &[(String, Vec<f32>, crate::velesql::CompareOp, f64)],
        filter_cond: Option<&crate::velesql::Condition>,
        limit: usize,
        overfetch_base: usize,
    ) -> Result<Vec<SearchResult>> {
        if field != "vector" {
            return Err(crate::error::Error::Config(format!(
                "similarity() field '{field}' not found. Only 'vector' field is supported. \
                Multi-vector support is planned for a future release."
            )));
        }

        let overfetch_factor = overfetch_base * all_conditions.len().max(1);
        let candidates_k = limit.saturating_mul(overfetch_factor).min(MAX_LIMIT);
        let candidates = self.search(vector, candidates_k)?;

        let filter_k = limit.saturating_mul(2);
        let mut filtered =
            self.filter_by_similarity(candidates, field, sim_vec, op, threshold, filter_k);

        for (sim_field, sim_vec, sim_op, sim_threshold) in all_conditions.iter().skip(1) {
            if sim_field != "vector" {
                return Err(crate::error::Error::Config(format!(
                    "similarity() field '{sim_field}' not found. Only 'vector' field is supported."
                )));
            }
            filtered = self.filter_by_similarity(
                filtered,
                sim_field,
                sim_vec,
                *sim_op,
                *sim_threshold,
                filter_k,
            );
        }

        Ok(Self::apply_metadata_post_filter(
            filtered,
            filter_cond,
            limit,
        ))
    }

    /// VP-011: Dispatch NEAR + optional MATCH/filter combination.
    fn dispatch_near_with_condition(
        &self,
        vector: &[f32],
        cond: &crate::velesql::Condition,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        if let Some((text_query, remaining_filter)) = Self::extract_match_and_filter(cond) {
            if let Some(filter_cond) = remaining_filter {
                let filter =
                    crate::filter::Filter::new(crate::filter::Condition::from(filter_cond));
                self.hybrid_search_with_filter(vector, &text_query, limit, None, &filter)
            } else {
                self.hybrid_search(vector, &text_query, limit, None)
            }
        } else {
            let filter = crate::filter::Filter::new(crate::filter::Condition::from(cond.clone()));
            self.search_with_filter(vector, limit, &filter)
        }
    }

    /// VP-011: Dispatch text search or metadata scan.
    fn dispatch_text_or_scan(
        &self,
        cond: &crate::velesql::Condition,
        limit: usize,
    ) -> Vec<SearchResult> {
        if let Some((text_query, remaining_filter)) = Self::extract_match_and_filter(cond) {
            if let Some(filter_cond) = remaining_filter {
                let filter =
                    crate::filter::Filter::new(crate::filter::Condition::from(filter_cond));
                self.text_search_with_filter(&text_query, limit, &filter)
            } else {
                self.text_search(&text_query, limit)
            }
        } else {
            let filter = crate::filter::Filter::new(crate::filter::Condition::from(cond.clone()));
            self.execute_scan_query(&filter, limit)
        }
    }

    /// Helper: apply metadata post-filter to already-filtered results.
    fn apply_metadata_post_filter(
        filtered: Vec<SearchResult>,
        filter_cond: Option<&crate::velesql::Condition>,
        limit: usize,
    ) -> Vec<SearchResult> {
        if let Some(cond) = filter_cond {
            let metadata_filter = Self::extract_metadata_filter(cond);
            if let Some(filter_cond) = metadata_filter {
                let filter =
                    crate::filter::Filter::new(crate::filter::Condition::from(filter_cond));
                filtered
                    .into_iter()
                    .filter(|r| match r.point.payload.as_ref() {
                        Some(p) => filter.matches(p),
                        None => filter.matches(&serde_json::Value::Null),
                    })
                    .take(limit)
                    .collect()
            } else {
                filtered
            }
        } else {
            filtered
        }
    }
}
