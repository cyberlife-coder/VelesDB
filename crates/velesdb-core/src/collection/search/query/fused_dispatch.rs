//! `NEAR_FUSED` multi-vector fusion query dispatch (SQL surface).
//!
//! Routes a SELECT whose WHERE is a top-level `NEAR_FUSED` through the engine's
//! [`Collection::multi_query_search`](Collection::multi_query_search) — the same
//! fusion the multi-query engine API performs — then applies the standard
//! DISTINCT / ORDER BY / OFFSET / LIMIT finalize.

use super::{Collection, ExtractedComponents, Result, SearchResult};
use crate::velesql::FusionConfig;

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
        let strategy = Self::fused_config_to_strategy(config);
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

    /// Maps a `NEAR_FUSED` [`FusionConfig`] (string strategy + numeric params) to
    /// a [`FusionStrategy`](crate::fusion::FusionStrategy).
    ///
    /// `rrf` (default), `average`, and `maximum` map directly. `weighted` / `rsf`
    /// take per-branch dense/sparse weights that are ill-defined for N homogeneous
    /// query vectors, so they — and any unknown strategy — fall back to RRF
    /// (matching the parser's default strategy).
    fn fused_config_to_strategy(config: &FusionConfig) -> crate::fusion::FusionStrategy {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let k = config.params.get("k").map_or(60, |v| *v as u32);
        match config.strategy.to_lowercase().as_str() {
            "average" => crate::fusion::FusionStrategy::Average,
            "maximum" => crate::fusion::FusionStrategy::Maximum,
            _ => crate::fusion::FusionStrategy::RRF { k },
        }
    }
}
