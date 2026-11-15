//! `NEAR_FUSED` multi-vector fusion query dispatch (SQL surface).
//!
//! Routes a SELECT whose WHERE is a top-level `NEAR_FUSED` through the engine's
//! [`Collection::multi_query_search`](Collection::multi_query_search) — the same
//! fusion the multi-query engine API performs — then applies the standard
//! DISTINCT / ORDER BY / OFFSET / LIMIT finalize.

use super::{Collection, ExtractedComponents, Result, SearchResult};

impl Collection {
    /// Dispatches a `NEAR_FUSED` query through real multi-vector fusion.
    ///
    /// Reuses `multi_query_search` verbatim (per-vector search + `fusion.fuse`),
    /// threading the residual metadata predicate as a pre-fusion filter.
    ///
    /// # Errors
    ///
    /// Returns an error if `multi_query_search` rejects the inputs (empty,
    /// dimension mismatch, or more than the supported vector count) or a
    /// guard-rail fires.
    pub(super) fn dispatch_fused_query(
        &self,
        stmt: &crate::velesql::SelectStatement,
        params: &std::collections::HashMap<String, serde_json::Value>,
        extracted: &ExtractedComponents,
        limit: usize,
        ctx: &crate::guardrails::QueryContext,
    ) -> Result<Vec<SearchResult>> {
        // The caller routes here only when `fused_search` is Some.
        let Some((vectors, config)) = extracted.fused_search.as_ref() else {
            return Ok(Vec::new());
        };
        let slices: Vec<&[f32]> = vectors.iter().map(Vec::as_slice).collect();
        let strategy = config.near_fused_strategy();
        // Residual metadata predicate (the fused leaf is dropped by
        // extract_metadata_filter) becomes the pre-fusion filter.
        let filter = extracted
            .filter_condition
            .as_ref()
            .and_then(Self::extract_metadata_filter)
            .map(|c| crate::filter::Filter::new(crate::filter::Condition::from(c)));
        let results = self.multi_query_search(&slices, limit, strategy, filter.as_ref())?;
        self.check_guardrails_and_record(ctx, results.len())?;
        self.finalize_sparse_results(stmt, params, results)
    }
}

#[cfg(test)]
#[path = "fused_dispatch_tests.rs"]
mod tests;
