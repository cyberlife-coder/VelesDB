//! Query search options and extracted components for the query pipeline.
//!
//! Extracted from `query/mod.rs` to keep file NLOC under 500.

/// Maximum allowed LIMIT value to prevent overflow in over-fetch calculations.
pub(in crate::collection::search::query) const MAX_LIMIT: usize = 100_000;

/// Query-time search options extracted from the WITH clause.
///
/// Consolidates `mode`, `ef_search`, `rerank`, and `fusion_clause` into a single
/// struct that flows through all dispatch paths. When no WITH clause is present,
/// all fields are `None` and the default behavior is preserved.
#[derive(Debug, Clone, Default)]
pub(crate) struct QuerySearchOptions {
    /// Search quality profile parsed from `WITH (mode='...')`.
    pub quality: Option<crate::SearchQuality>,
    /// Explicit ef_search override from `WITH (ef_search=N)`.
    pub ef_search: Option<usize>,
    /// Force reranking on (`true`) or off (`false`) from `WITH (rerank=...)`.
    pub force_rerank: Option<bool>,
    /// Fusion clause from `USING FUSION (...)`.
    pub fusion_clause: Option<crate::velesql::FusionClause>,
}

impl QuerySearchOptions {
    /// Extracts search options from an optional WITH clause and fusion clause.
    ///
    /// Maps `mode` string to [`SearchQuality`](crate::SearchQuality) using the
    /// same parsing logic as `mode_to_search_quality()`. Invalid mode strings
    /// are silently ignored (quality remains `None`).
    #[must_use]
    pub(crate) fn from_with_clause(with: Option<&crate::velesql::WithClause>) -> Self {
        let Some(with) = with else {
            return Self::default();
        };

        let quality = with.get_mode().and_then(parse_mode_to_quality);

        let ef_search = with.get_ef_search();
        let force_rerank = with.get_rerank();

        Self {
            quality,
            ef_search,
            force_rerank,
            fusion_clause: None,
        }
    }

    /// Creates options with a fusion clause attached.
    #[must_use]
    pub(crate) fn with_fusion(mut self, fusion: Option<crate::velesql::FusionClause>) -> Self {
        self.fusion_clause = fusion;
        self
    }

    /// Returns `true` when any quality-related override is set.
    #[must_use]
    pub(crate) fn has_quality_overrides(&self) -> bool {
        self.quality.is_some() || self.ef_search.is_some() || self.force_rerank.is_some()
    }
}

/// Maps a mode string from `WITH (mode='...')` to a [`SearchQuality`](crate::SearchQuality).
///
/// Delegates to [`crate::api_types::mode_to_search_quality`] which also handles
/// advanced modes (`custom:<ef>`, `adaptive:<min>:<max>`).
#[cfg(feature = "persistence")]
fn parse_mode_to_quality(mode: &str) -> Option<crate::SearchQuality> {
    crate::api_types::mode_to_search_quality(mode)
}

/// Extracted query components from the WHERE clause.
pub(in crate::collection::search::query) struct ExtractedComponents {
    pub(in crate::collection::search::query) vector_search: Option<Vec<f32>>,
    pub(in crate::collection::search::query) similarity_conditions:
        Vec<(String, Vec<f32>, crate::velesql::CompareOp, f64)>,
    pub(in crate::collection::search::query) filter_condition: Option<crate::velesql::Condition>,
    pub(in crate::collection::search::query) graph_match_predicates:
        Vec<crate::velesql::GraphMatchPredicate>,
    pub(in crate::collection::search::query) sparse_vector_search:
        Option<crate::velesql::SparseVectorSearch>,
    pub(in crate::collection::search::query) is_union_query: bool,
    pub(in crate::collection::search::query) is_not_similarity_query: bool,
}

/// Bundles the parameters for [`Collection::finalize_query_results`] to stay
/// within the 8-parameter limit.
pub(in crate::collection::search::query) struct QueryFinalizationContext<'a> {
    pub(in crate::collection::search::query) stmt: &'a crate::velesql::SelectStatement,
    pub(in crate::collection::search::query) params:
        &'a std::collections::HashMap<String, serde_json::Value>,
    pub(in crate::collection::search::query) limit: usize,
    pub(in crate::collection::search::query) extracted: &'a ExtractedComponents,
    pub(in crate::collection::search::query) ctx: &'a crate::guardrails::QueryContext,
    pub(in crate::collection::search::query) let_bindings: &'a [crate::velesql::LetBinding],
}
