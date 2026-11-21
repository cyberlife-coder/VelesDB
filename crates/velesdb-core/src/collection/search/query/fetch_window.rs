//! How a SELECT reads the rows its `ORDER BY`, `OFFSET` and `LIMIT` run
//! over.
//!
//! After excluding shapes that need the whole partition, a fetch may stop
//! at `LIMIT + OFFSET` when no `ORDER BY` reorders its rows, or the
//! `ORDER BY` is a single `similarity()` key read most similar first that the
//! fetch already ranks by — a `NEAR` or an index-bounded threshold on that
//! vector, or a sparse search's own score under a metric where higher is more
//! similar ([`FetchWindow::Bounded`]). A vector fetch reads past a tie at
//! the cut ([`fetch_past_ties`]). The sparse ranking already breaks ties
//! by ascending id.
//!
//! Any other `ORDER BY` ranks every row the statement admits. When the rows
//! carry no score only their search can give, the ordered scan reads every
//! one of them and keeps only the leading `OFFSET + LIMIT` hydrated rows
//! ([`FetchWindow::OrderedScan`], `ordered_scan.rs`), whatever their number.
//! A fusion (its scores depend on how deep it reads), a BM25 or sparse score
//! read in another order, and the shapes that reshape the rows before the
//! window (DISTINCT, a vector GROUP BY, a window function) read up to
//! [`MAX_LIMIT`] rows instead ([`FetchWindow::Exhaustive`]), and a read that
//! fills that ceiling is reported, never silent
//! (`Collection::warn_if_window_capped`).

use super::{ExtractedComponents, QuerySearchOptions, MAX_LIMIT};
use crate::collection::types::Collection;
use crate::error::Result;
use crate::velesql::text_match::TextRanking;
use crate::velesql::{OrderByExpr, SelectOrderBy, SelectStatement};

pub(super) fn filtered_search_depth(limit: usize, over_fetch: usize) -> usize {
    limit.saturating_mul(over_fetch).min(MAX_LIMIT)
}

/// The native threshold cascade grows the same candidate window for row
/// and score-only reads. Its caller decides whether the ranked boundary
/// already proves that no further candidate can pass.
pub(super) fn fill_similarity_window<T>(
    total: usize,
    limit: usize,
    leaves: usize,
    mut fetch: impl FnMut(usize) -> Result<(Vec<T>, bool)>,
) -> Result<Vec<T>> {
    let mut k = limit
        .saturating_mul(10 * leaves.max(1))
        .clamp(1, total.max(1));
    loop {
        let (rows, exhausted) = fetch(k)?;
        if rows.len() >= limit || exhausted || k >= total {
            return Ok(rows);
        }
        k = k.saturating_mul(2).min(total);
    }
}

/// How a SELECT reads the rows its window runs over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FetchWindow {
    /// The fetch stops at these `LIMIT + OFFSET` rows, past any tie at the
    /// cut: they are the window's.
    Bounded(usize),
    /// The ordered scan reads every admitted row and keeps the window.
    /// A default PQ NEAR contributes its native primary scores.
    OrderedScan { pq_scores: bool },
    /// The fetch reads up to [`MAX_LIMIT`] rows, and the window is selected
    /// from those.
    Exhaustive,
}

/// What a SELECT's fetch ranks its rows by, best first.
enum FetchRanking<'a> {
    /// No search ranks the rows — a metadata scan, a graph's node scan — or
    /// the exact `similarity()` scan, which puts the rows it cannot score
    /// after the others, where a `DESC` puts a null key first
    /// ([`SelectOrderBy::directed_score`]).
    ///
    /// [`SelectOrderBy::directed_score`]: crate::velesql::SelectOrderBy::directed_score
    Unranked,
    /// A vector search in the collection's metric — a `NEAR`, or a
    /// `similarity()` threshold an index bounds — on `field`, to `vector`:
    /// the rows come back most similar first, each scored by it.
    Vector { field: &'a str, vector: &'a [f32] },
    /// A score of the search's own, higher first, whose rows a scan cannot
    /// score: BM25 alone, a sparse search alone.
    OwnScore,
    /// A fusion of two rankings, each read to a depth the window sets: its
    /// first rows at one depth are not the first rows at another.
    Fused,
}

/// Where the dense dispatch gets its primary score.
#[derive(Copy, Clone, PartialEq, Eq)]
enum NearPath {
    Rows,
    Search,
    PostFilter,
    Bitmap,
}

impl FetchRanking<'_> {
    /// The ranking of the fetch `extracted` describes, over `stmt`'s bound
    /// WHERE (a text `MATCH` left in it ranks; every other one was bound to
    /// its hits, `velesql::text_match::TextRanking`).
    fn of<'a>(stmt: &SelectStatement, extracted: &'a ExtractedComponents) -> FetchRanking<'a> {
        let text = stmt
            .where_clause
            .as_ref()
            .map_or(TextRanking::None, TextRanking::of);
        Self::by_score(&text, extracted)
            .or_else(|| {
                (!extracted.similarity_scan)
                    .then(|| Self::by_vector(extracted))
                    .flatten()
            })
            .unwrap_or(FetchRanking::Unranked)
    }

    /// The ranking of a fetch by a score of its own or a fusion: `None`
    /// when no text `MATCH`, `SPARSE_NEAR` or `NEAR_FUSED` ranks it.
    fn by_score(
        text: &TextRanking<'_>,
        extracted: &ExtractedComponents,
    ) -> Option<FetchRanking<'static>> {
        let sparse = extracted.sparse_vector_search.is_some();
        if matches!(text, TextRanking::Hybrid(_))
            || extracted.fused_search.is_some()
            || (sparse && extracted.vector_search.is_some())
        {
            return Some(FetchRanking::Fused);
        }
        (matches!(text, TextRanking::Text(_)) || sparse).then_some(FetchRanking::OwnScore)
    }

    /// The ranking of a fetch by a vector search: its `NEAR`, else its
    /// first `similarity()` threshold.
    fn by_vector(extracted: &ExtractedComponents) -> Option<FetchRanking<'_>> {
        if let Some(vector) = extracted.vector_search.as_deref() {
            return Some(FetchRanking::Vector {
                field: crate::velesql::PRIMARY_VECTOR_FIELD,
                vector,
            });
        }
        extracted
            .similarity_conditions
            .first()
            .map(|(field, vector, _, _)| FetchRanking::Vector { field, vector })
    }

    /// Whether `expr`, read most similar first (`DESC`, whatever the
    /// metric), orders the rows as this ranking does: the search score
    /// itself (`similarity()`) — an own score only under a metric where
    /// higher is more similar, which `DESC` then reads higher first — or
    /// `similarity(field, $v)` on the field and vector the search ranked by.
    fn orders_like(
        &self,
        expr: &OrderByExpr,
        params: &std::collections::HashMap<String, serde_json::Value>,
        higher_is_better: bool,
    ) -> bool {
        match (self, expr) {
            (Self::Vector { .. }, OrderByExpr::SimilarityBare) => true,
            (Self::OwnScore, OrderByExpr::SimilarityBare) => higher_is_better,
            (Self::Vector { field, vector }, OrderByExpr::Similarity(sim)) => {
                sim.field == *field
                    && crate::velesql::bind_vector(&sim.vector, params)
                        .is_ok_and(|bound| bound == *vector)
            }
            _ => false,
        }
    }

    /// Whether the ordered scan can read the rows this ranking reads: every
    /// score it gives is a vector's similarity the scan computes too.
    const fn scannable(&self) -> bool {
        matches!(self, Self::Unranked | Self::Vector { .. })
    }

    fn window(&self, matching: bool, fetch_limit: usize) -> FetchWindow {
        if matching {
            FetchWindow::Bounded(fetch_limit)
        } else if self.scannable() {
            FetchWindow::OrderedScan { pq_scores: false }
        } else {
            FetchWindow::Exhaustive
        }
    }
}

impl Collection {
    /// How the SELECT `stmt` reads its rows (see the module docs):
    /// `fetch_limit` is its `LIMIT` plus `OFFSET`.
    pub(super) fn select_fetch_window(
        &self,
        stmt: &SelectStatement,
        extracted: &mut ExtractedComponents,
        params: &std::collections::HashMap<String, serde_json::Value>,
        fetch_limit: usize,
        search_opts: &mut QuerySearchOptions,
    ) -> FetchWindow {
        if stmt.reshapes_rows_before_window() {
            return FetchWindow::Exhaustive;
        }
        let Some(order_by) = stmt.order_by.as_deref() else {
            return FetchWindow::Bounded(fetch_limit);
        };
        let ranking = FetchRanking::of(stmt, extracted);
        let window = ranking.window(
            self.ranks_as_ordered(&ranking, order_by, params),
            fetch_limit,
        );
        let FetchRanking::Vector { .. } = ranking else {
            return window;
        };
        // A NEAR already ranks by its primary score, including an ADC score.
        if extracted.vector_search.is_some()
            && bare_score_first(order_by)
            && (order_by.len() == 1
                || self.complete_native_prefix(extracted, search_opts, fetch_limit))
        {
            return FetchWindow::Bounded(fetch_limit);
        }
        if let Some(pq_scores) =
            self.ordered_native_scores(stmt, extracted, search_opts, fetch_limit)
        {
            return FetchWindow::OrderedScan { pq_scores };
        }
        if matches!(window, FetchWindow::Bounded(_)) {
            search_opts.vector_score_source = super::super::vector::VectorScoreSource::Original;
        }
        window
    }

    /// A search over rounded index scores or PQ codes cannot cut an order
    /// on original stored vectors. A threshold alone owns its original score;
    /// under a NEAR the candidates keep the native primary score as well.
    fn ordered_native_scores(
        &self,
        stmt: &SelectStatement,
        extracted: &mut ExtractedComponents,
        search_opts: &QuerySearchOptions,
        limit: usize,
    ) -> Option<bool> {
        // Required anchors choose between original-vector and bitmap scores
        // only after MATCH. The scan resolves them once with its WHERE cache.
        if Self::anchored_near(extracted) {
            if !self.anchored_scores_can_differ() {
                return None;
            }
            extracted.ordered_index_scores = Some(super::options::OrderedIndexScores::Anchors);
            return Some(false);
        }
        self.ordered_search_scores(stmt, extracted, search_opts, limit)
    }

    fn ordered_search_scores(
        &self,
        stmt: &SelectStatement,
        extracted: &mut ExtractedComponents,
        search_opts: &QuerySearchOptions,
        limit: usize,
    ) -> Option<bool> {
        let (rounded, pq) = self.distinct_index_scores(search_opts)?;
        let path = self.near_score_path(stmt, extracted, limit);
        if path == NearPath::Rows {
            return None;
        }
        let pq = pq && path != NearPath::Bitmap;
        if !rounded && !pq {
            return None;
        }
        if extracted.vector_search.is_none() {
            return Some(false);
        }
        if rounded {
            extracted.ordered_index_scores = Some(super::options::OrderedIndexScores::Index(
                self.near_index_scores(path, extracted, search_opts),
            ));
        }
        Some(pq)
    }

    fn anchored_near(extracted: &ExtractedComponents) -> bool {
        extracted.vector_search.is_some()
            && extracted.similarity_conditions.is_empty()
            && !extracted.graph_match_predicates.is_empty()
            && extracted.filter_condition.as_ref().is_some_and(|cond| {
                !super::graph_prefilter::collect_required_graph_predicates(cond).is_empty()
            })
    }

    fn anchored_scores_can_differ(&self) -> bool {
        let rounded = matches!(
            self.storage.config.read().metric,
            crate::DistanceMetric::Cosine | crate::DistanceMetric::Jaccard
        );
        #[cfg(feature = "gpu")]
        let rounded = rounded || self.storage.index.graph_scores_may_use_device();
        rounded
    }

    fn distinct_index_scores(&self, opts: &QuerySearchOptions) -> Option<(bool, bool)> {
        let config = self.storage.config.read();
        // The cosine arena can be normalized on insert. Graph traversal
        // round-trips Jaccard through 1 - similarity and the inverse.
        let rounded = matches!(
            config.metric,
            crate::DistanceMetric::Cosine | crate::DistanceMetric::Jaccard
        );
        let can_use_pq = config.pq_search_oversampling() != 0 && !opts.has_quality_overrides();
        drop(config);
        #[cfg(feature = "gpu")]
        let rounded = rounded
            || self.storage.index.device_scoring_eligible(
                opts.resolved_quality(self.runtime_search_quality()),
                opts.force_rerank,
            );
        let pq = can_use_pq
            && !self.storage.pq_cache.read().is_empty()
            && self.storage.pq_quantizer.read().is_some();
        (rounded || pq).then_some((rounded, pq))
    }

    fn near_index_scores(
        &self,
        path: NearPath,
        extracted: &ExtractedComponents,
        opts: &QuerySearchOptions,
    ) -> crate::index::hnsw::index::IndexRowScores {
        let quality = opts.resolved_quality(self.runtime_search_quality());
        if let Some(bitmap) = extracted
            .ordered_prefilter
            .as_ref()
            .and_then(|p| p.bitmap.as_ref())
        {
            return self.filtered_row_scores(bitmap, opts, quality);
        }
        let rerank = (path == NearPath::Search)
            .then_some(opts.force_rerank)
            .flatten();
        self.storage.index.full_depth_row_scores(quality, rerank)
    }

    fn near_score_path(
        &self,
        stmt: &SelectStatement,
        extracted: &mut ExtractedComponents,
        limit: usize,
    ) -> NearPath {
        // The similarity cascade searches first, then checks all predicates.
        if !extracted.similarity_conditions.is_empty() {
            return NearPath::Search;
        }
        let Some(cond) = extracted.filter_condition.as_ref() else {
            return NearPath::Search;
        };
        if !extracted.graph_match_predicates.is_empty() && Self::condition_contains_or(cond) {
            return NearPath::Search;
        }
        let Some(metadata) = Self::extract_metadata_filter(cond) else {
            return NearPath::Search;
        };
        let (strategy, _) = self.compute_cbo_strategy(stmt, Some(cond), limit);
        if strategy == crate::velesql::ExecutionStrategy::GraphFirst {
            return NearPath::Rows;
        }
        let covers_scan = !Self::index_reads_dotted_field(&metadata);
        let filter = crate::filter::Filter::new(metadata.into());
        let bitmap = self.build_prefilter_bitmap(&filter);
        let path = if bitmap.is_some() {
            NearPath::Bitmap
        } else {
            NearPath::PostFilter
        };
        extracted.ordered_prefilter = Some(super::options::OrderedPrefilter {
            filter,
            bitmap,
            covers_scan,
        });
        path
    }

    /// Whether `ranking` returns rows in the order of `order_by`: one key,
    /// read `DESC`, that it orders like ([`FetchRanking::orders_like`]).
    fn ranks_as_ordered(
        &self,
        ranking: &FetchRanking<'_>,
        order_by: &[crate::velesql::SelectOrderBy],
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> bool {
        let [only] = order_by else {
            return false;
        };
        let higher_is_better = self.storage.config.read().metric.higher_is_better();
        only.descending && ranking.orders_like(&only.expr, params, higher_is_better)
    }

    /// A secondary key can use the native prefix only when the retrieval is
    /// exhaustive and neither its page nor its boundary tie can hit the cap.
    fn complete_native_prefix(
        &self,
        extracted: &ExtractedComponents,
        opts: &QuerySearchOptions,
        limit: usize,
    ) -> bool {
        if !extracted.is_unfiltered_near() || limit >= MAX_LIMIT || self.vector_count() >= MAX_LIMIT
        {
            return false;
        }
        let pq = !opts.has_quality_overrides()
            && self.storage.config.read().pq_search_oversampling() != 0;
        !pq && self.storage.index.searches_arena_exhaustively(
            opts.resolved_quality(self.runtime_search_quality()),
            opts.force_rerank,
        )
    }

    /// Reports a fetch that read `fetched` rows — the [`MAX_LIMIT`] ceiling
    /// — for a window that ranks or reshapes them afterwards: rows past the
    /// ceiling may have been left out of that window. The ordered scan never
    /// stops there and does not report.
    pub(super) fn warn_if_window_capped(&self, stmt: &SelectStatement, fetched: usize) {
        self.warn_if_window_capped_at(stmt, fetched, MAX_LIMIT);
    }

    /// Reports the raw candidate count against the ceiling used for this fetch.
    pub(super) fn warn_if_window_capped_at(
        &self,
        stmt: &SelectStatement,
        fetched: usize,
        ceiling: usize,
    ) {
        let reorders = stmt.order_by.is_some() || stmt.reshapes_rows_before_window();
        if reorders && fetched >= ceiling {
            tracing::warn!(
                collection = %self.storage.config.read().name,
                ceiling,
                "SELECT fetch stopped at its row ceiling before its ORDER BY, DISTINCT, \
                 vector GROUP BY or window function ran: rows past the ceiling may be \
                 missing from the result. Narrow the WHERE so it admits fewer rows."
            );
        }
    }
}

pub(super) fn bare_score_first(order: &[SelectOrderBy]) -> bool {
    matches!(
        order.first(),
        Some(SelectOrderBy {
            expr: OrderByExpr::SimilarityBare,
            descending: true
        })
    )
}

/// The rows `fetch(n)` reads, best first, for a window of `leading` rows
/// that an `ORDER BY` of `stmt` ranks: read on past the window while the
/// last row read ties the window's last one, so the tie rule — ascending id
/// — decides which tied rows take a place, not the order the fetch met them
/// in. Without an `ORDER BY` the fetch order is the result order, read once.
///
/// # Errors
///
/// What `fetch` returns.
pub(super) fn fetch_past_ties<T: super::super::live::LiveHit>(
    stmt: &SelectStatement,
    leading: usize,
    mut fetch: impl FnMut(usize) -> Result<Vec<T>>,
) -> Result<Vec<T>> {
    if stmt.order_by.is_none() || leading == 0 || leading >= MAX_LIMIT {
        return fetch(leading);
    }
    // One row past the window shows whether the cut splits a tie.
    let mut want = leading + 1;
    loop {
        let rows = fetch(want)?;
        let splits_a_tie = rows.len() >= want
            && rows
                .get(leading - 1)
                .zip(rows.last())
                .is_some_and(|(last_kept, last_read)| {
                    crate::distance::rank_scores(last_kept.score(), last_read.score(), true).is_eq()
                });
        if !splits_a_tie || want >= MAX_LIMIT {
            return Ok(rows);
        }
        want = want.saturating_mul(2).min(MAX_LIMIT);
    }
}

#[cfg(test)]
#[path = "fetch_window_tests.rs"]
mod tests;
