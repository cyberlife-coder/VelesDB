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

use super::{ExtractedComponents, MAX_LIMIT};
use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::SearchResult;
use crate::velesql::text_match::TextRanking;
use crate::velesql::{OrderByExpr, SelectStatement};

/// How a SELECT reads the rows its window runs over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FetchWindow {
    /// The fetch stops at these `LIMIT + OFFSET` rows, past any tie at the
    /// cut: they are the window's.
    Bounded(usize),
    /// The ordered scan reads every admitted row and keeps the window.
    OrderedScan,
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
}

impl Collection {
    /// How the SELECT `stmt` reads its rows (see the module docs):
    /// `fetch_limit` is its `LIMIT` plus `OFFSET`.
    pub(super) fn select_fetch_window(
        &self,
        stmt: &SelectStatement,
        extracted: &ExtractedComponents,
        params: &std::collections::HashMap<String, serde_json::Value>,
        fetch_limit: usize,
    ) -> FetchWindow {
        if stmt.reshapes_rows_before_window() {
            return FetchWindow::Exhaustive;
        }
        let Some(order_by) = stmt.order_by.as_deref() else {
            return FetchWindow::Bounded(fetch_limit);
        };
        let ranking = FetchRanking::of(stmt, extracted);
        if self.ranks_as_ordered(&ranking, order_by, params) {
            FetchWindow::Bounded(fetch_limit)
        } else if ranking.scannable() {
            FetchWindow::OrderedScan
        } else {
            FetchWindow::Exhaustive
        }
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

/// The rows `fetch(n)` reads, best first, for a window of `leading` rows
/// that an `ORDER BY` of `stmt` ranks: read on past the window while the
/// last row read ties the window's last one, so the tie rule — ascending id
/// — decides which tied rows take a place, not the order the fetch met them
/// in. Without an `ORDER BY` the fetch order is the result order, read once.
///
/// # Errors
///
/// What `fetch` returns.
pub(super) fn fetch_past_ties(
    stmt: &SelectStatement,
    leading: usize,
    mut fetch: impl FnMut(usize) -> Result<Vec<SearchResult>>,
) -> Result<Vec<SearchResult>> {
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
                    crate::distance::rank_scores(last_kept.score, last_read.score, true).is_eq()
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
