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

#[cfg(test)]
mod tests {
    use super::Collection;
    use crate::fusion::FusionStrategy;
    use crate::velesql::FusionConfig;

    /// Builds a `FusionConfig` with the given strategy name and optional `k`.
    fn config(strategy: &str, k: Option<f64>) -> FusionConfig {
        let mut params = std::collections::HashMap::new();
        if let Some(k) = k {
            params.insert("k".to_string(), k);
        }
        FusionConfig {
            strategy: strategy.to_string(),
            params,
        }
    }

    #[test]
    fn maps_average_strategy() {
        let strat = Collection::fused_config_to_strategy(&config("average", None));
        assert_eq!(strat, FusionStrategy::Average);
    }

    #[test]
    fn maps_maximum_strategy() {
        let strat = Collection::fused_config_to_strategy(&config("maximum", None));
        assert_eq!(strat, FusionStrategy::Maximum);
    }

    #[test]
    fn maps_rrf_strategy_with_default_k() {
        // No `k` param => the documented default k=60.
        let strat = Collection::fused_config_to_strategy(&config("rrf", None));
        assert_eq!(strat, FusionStrategy::RRF { k: 60 });
    }

    #[test]
    fn rrf_honors_explicit_k() {
        let strat = Collection::fused_config_to_strategy(&config("rrf", Some(42.0)));
        assert_eq!(strat, FusionStrategy::RRF { k: 42 });
    }

    #[test]
    fn unknown_strategy_falls_back_to_rrf() {
        // weighted/rsf/garbage are ill-defined for N homogeneous query vectors
        // and must fall back to RRF (parser default), not error or silently drop.
        for name in ["weighted", "rsf", "nonsense"] {
            let strat = Collection::fused_config_to_strategy(&config(name, Some(7.0)));
            assert_eq!(
                strat,
                FusionStrategy::RRF { k: 7 },
                "strategy '{name}' must fall back to RRF (honoring k)"
            );
        }
    }

    #[test]
    fn strategy_name_is_case_insensitive() {
        assert_eq!(
            Collection::fused_config_to_strategy(&config("AVERAGE", None)),
            FusionStrategy::Average
        );
        assert_eq!(
            Collection::fused_config_to_strategy(&config("Maximum", None)),
            FusionStrategy::Maximum
        );
    }
}
