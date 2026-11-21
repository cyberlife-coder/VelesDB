//! Query search options and extracted components for the query pipeline.
//!
//! Extracted from `query/mod.rs` to keep file NLOC under 500.

/// Maximum allowed LIMIT value to prevent overflow in over-fetch calculations.
pub(crate) const MAX_LIMIT: usize = 100_000;

// One ceiling: the engine's and the one the WASM executor caps a LIMIT to.
const _: () = assert!(MAX_LIMIT as u64 == crate::velesql::MAX_SELECT_LIMIT);

/// Query-time search options extracted from the WITH clause.
///
/// Consolidates `mode`, `ef_search`, `rerank`, and `fusion_clause` into a single
/// struct that flows through all dispatch paths. When no WITH clause is present,
/// all fields are `None` and the default behavior is preserved.
#[derive(Debug, Clone, Default)]
pub(crate) struct QuerySearchOptions {
    /// Score source guaranteed by a bounded, explicit f32 SELECT ordering.
    pub(in crate::collection::search) vector_score_source: super::super::vector::VectorScoreSource,
    /// Search quality profile parsed from `WITH (mode='...')`.
    pub quality: Option<crate::SearchQuality>,
    /// Explicit ef_search override from `WITH (ef_search=N)`.
    pub ef_search: Option<usize>,
    /// Force reranking on (`true`) or off (`false`) from `WITH (rerank=...)`.
    pub force_rerank: Option<bool>,
    /// Fusion clause from `USING FUSION (...)`.
    pub fusion_clause: Option<crate::velesql::FusionClause>,
    /// EXPLAIN ANALYZE out-channel: the executor records which filter
    /// strategy it actually ran into this cell (shared with the query's
    /// [`QueryContext`](crate::guardrails::QueryContext)). `None` outside
    /// the pipeline (raw search entry points) — recording is then a no-op.
    /// A shared atomic cell, not a thread-local, so the record does not
    /// depend on which thread the dispatch runs on.
    pub executed_strategy_probe: Option<std::sync::Arc<crate::guardrails::ExecutedStrategyCell>>,
}

impl QuerySearchOptions {
    /// Extracts search options from an optional WITH clause and fusion clause.
    ///
    /// Maps the `mode` option, or its alias `quality`, to a
    /// [`SearchQuality`](crate::SearchQuality) through
    /// [`WithClause::search_quality`](crate::velesql::WithClause::search_quality),
    /// and `ef_search` through
    /// [`WithClause::ef_search`](crate::velesql::WithClause::ef_search): a
    /// mode that is not a string or does not parse, or an `ef_search` outside
    /// the documented range, is a query error, not a silently ignored
    /// override (#2267, #2274). The query validator rejects both first on
    /// every query path; this keeps the conversion honest for a direct
    /// caller.
    pub(crate) fn from_with_clause(
        with: Option<&crate::velesql::WithClause>,
    ) -> crate::error::Result<Self> {
        let Some(with) = with else {
            return Ok(Self::default());
        };

        let quality = with.search_quality().map_err(crate::error::Error::Query)?;
        let ef_search = with.ef_search().map_err(crate::error::Error::Query)?;
        let force_rerank = with.get_rerank();

        Ok(Self {
            quality,
            ef_search,
            force_rerank,
            ..Self::default()
        })
    }

    /// Attaches the query context's executed-strategy slot, so the filtered
    /// vector-search dispatch can report which shape it ran to EXPLAIN
    /// ANALYZE.
    #[must_use]
    pub(crate) fn with_executed_strategy_probe(
        mut self,
        ctx: &crate::guardrails::QueryContext,
    ) -> Self {
        self.executed_strategy_probe = Some(ctx.executed_strategy_slot());
        self
    }

    /// Records the filter strategy the executor is about to run. No-op when
    /// no probe is attached (raw search entry points).
    pub(crate) fn record_executed_strategy(&self, strategy: crate::velesql::FilterStrategy) {
        if let Some(probe) = &self.executed_strategy_probe {
            probe.record(strategy);
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

    /// The quality a search with these options runs at: an explicit
    /// `ef_search`, which wins over `mode` as `docs/VELESQL_SPEC.md` documents
    /// and REST resolves (#2274), else `mode`, else `default` — the
    /// collection's runtime quality, so options that name no quality (e.g.
    /// only `rerank`) keep the configured `[search]` default (#2399). The one
    /// resolution the plain and the filtered vector path share.
    #[must_use]
    pub(crate) fn resolved_quality(&self, default: crate::SearchQuality) -> crate::SearchQuality {
        self.ef_search
            .map(crate::collection::search::vector_filter::ef_to_quality)
            .or(self.quality)
            .unwrap_or(default)
    }
}

/// Source of the ordered scan's native NEAR scores.
#[derive(Clone, Copy)]
pub(super) enum OrderedIndexScores {
    Index(crate::index::hnsw::index::IndexRowScores),
    /// Resolved after MATCH populates the candidate ids for the ordered scan.
    Anchors,
}

/// Extracted query components from the WHERE clause.
pub(in crate::collection::search::query) struct ExtractedComponents {
    /// Native primary score for an ordered NEAR whose index score can differ
    /// from a calculation on the hydrated row.
    pub(super) ordered_index_scores: Option<OrderedIndexScores>,
    /// Filter the window planner resolved to identify the native NEAR scorer.
    /// Reuse its bitmap in the search or in the ordered scan.
    pub(super) ordered_prefilter: Option<OrderedPrefilter>,
    pub(in crate::collection::search::query) vector_search: Option<Vec<f32>>,
    pub(in crate::collection::search::query) similarity_conditions:
        Vec<(String, Vec<f32>, crate::velesql::CompareOp, f64)>,
    pub(in crate::collection::search::query) filter_condition: Option<crate::velesql::Condition>,
    pub(in crate::collection::search::query) graph_match_predicates:
        Vec<crate::velesql::GraphMatchPredicate>,
    pub(in crate::collection::search::query) sparse_vector_search:
        Option<crate::velesql::SparseVectorSearch>,
    /// `NEAR_FUSED` multi-vector fusion: resolved query vectors + fusion config,
    /// routed to `multi_query_search`. `None` for non-fused queries.
    pub(in crate::collection::search::query) fused_search:
        Option<(Vec<Vec<f32>>, crate::velesql::FusionConfig)>,
    /// The WHERE needs the exact `similarity()` scan (`similarity_scan.rs`):
    /// see [`Collection::needs_similarity_scan`].
    ///
    /// [`Collection::needs_similarity_scan`]: crate::collection::Collection
    pub(in crate::collection::search::query) similarity_scan: bool,
}

impl ExtractedComponents {
    pub(super) fn is_unfiltered_near(&self) -> bool {
        use crate::velesql::Condition;
        let mut condition = self.filter_condition.as_ref();
        while let Some(Condition::Group(inner)) = condition {
            condition = Some(inner);
        }
        self.vector_search.is_some() && matches!(condition, Some(Condition::VectorSearch(_)))
    }

    /// A NEAR or an index-bounded threshold uses the configured vector
    /// search quality. Exact predicate scans have their own candidate ceiling.
    pub(super) fn requests_dense_search(&self) -> bool {
        self.vector_search.is_some()
            || (!self.similarity_scan && !self.similarity_conditions.is_empty())
    }
}

pub(super) struct OrderedPrefilter {
    pub(super) filter: crate::filter::Filter,
    pub(super) bitmap: Option<roaring::RoaringBitmap>,
    /// Payload indexes read literal field names; WHERE follows dotted paths.
    pub(super) covers_scan: bool,
}

/// Bundles the parameters for [`Collection::finalize_query_results`] to stay
/// within the 8-parameter limit.
///
/// [`Collection::finalize_query_results`]: crate::collection::Collection::finalize_query_results
pub(in crate::collection::search::query) struct QueryFinalizationContext<'a> {
    pub(in crate::collection::search::query) stmt: &'a crate::velesql::SelectStatement,
    pub(in crate::collection::search::query) params:
        &'a std::collections::HashMap<String, serde_json::Value>,
    pub(in crate::collection::search::query) limit: usize,
    pub(in crate::collection::search::query) extracted: &'a ExtractedComponents,
    pub(in crate::collection::search::query) ctx: &'a crate::guardrails::QueryContext,
    pub(in crate::collection::search::query) let_bindings: &'a [crate::velesql::LetBinding],
}
