//! Index-backed `ORDER BY <field> [WHERE <metadata>] LIMIT k` fast path
//! (EPIC-081 phases 2 & 3b).
//!
//! Routes a scalar `ORDER BY <indexed_field> LIMIT k` — optionally carrying a
//! pure-metadata `WHERE` — to the field's ordered secondary B-tree index instead
//! of the B001 exhaustive `MAX_LIMIT` fetch + in-memory sort. Without a filter
//! the walk is `O(log n + k)`. With one it walks the fully-covered ordered index
//! applying the **same** metadata predicate the exhaustive path applies, stopping
//! once the page is filled — equivalent to the exhaustive `filter → sort → limit`
//! because the index yields rows in the identical total order (ties broken by
//! ascending id) and a filter only removes rows. The route is gated hard and
//! falls through to the exact pre-existing behaviour for every shape it does not
//! cover — see [`ordered_index_scan_applies`].

use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::SearchResult;
use crate::velesql::Condition;

use super::options::MAX_LIMIT;
use super::ExtractedComponents;

/// Rows hydrated per batch while walking the ordered index under a WHERE filter.
/// Bounds peak memory; a broadly-matching filter fills its page within the first
/// batch or two and stops.
const FILTER_HYDRATION_CHUNK: usize = 1024;

/// Minimum estimated WHERE selectivity for the filtered route. Below this the
/// filter is selective enough that the exhaustive path's bitmap prefilter (which
/// hydrates only matches) is likely cheaper than walking the ordered index and
/// hydrating non-matches, so the route declines. Conservative; tunable. Note the
/// route is never *incorrect* below it — it just falls back to the exhaustive
/// path, which is also correct.
const MIN_FILTERED_ROUTE_SELECTIVITY: f64 = 0.1;

impl Collection {
    /// Attempts the ordered-index fast path. Returns `Ok(Some(results))` when it
    /// fires (the page is already OFFSET/LIMIT-applied and ordered), `Ok(None)`
    /// to fall through to today's exact behaviour.
    ///
    /// Fires only when ALL hold (see [`ordered_index_scan_applies`]): no LET
    /// bindings; the ORDER BY is a single plain `Field`; that field has a
    /// fully-covering secondary index; no JOIN / graph / vector / sparse /
    /// DISTINCT / GROUP BY / aggregate / window; and the only WHERE permitted is
    /// a pure-metadata filter. The result is identical to the exhaustive path
    /// because the route restricts itself to fully-covered fields and applies the
    /// exhaustive metadata predicate verbatim.
    ///
    /// The ordered IDs are snapshotted under the secondary-index read lock, which
    /// is released before `get` re-reads payloads — so hydration tolerates
    /// concurrent writes and respects the lock order.
    pub(super) fn try_ordered_index_scan(
        &self,
        query: &crate::velesql::Query,
        stmt: &crate::velesql::SelectStatement,
        extracted: &ExtractedComponents,
        ctx: &crate::guardrails::QueryContext,
    ) -> Result<Option<Vec<SearchResult>>> {
        // LET bindings need the post-processing pipeline, so they keep the
        // exhaustive path.
        if !query.let_bindings.is_empty() {
            return Ok(None);
        }
        let Some((field, filter)) = ordered_index_scan_applies(stmt, extracted) else {
            return Ok(None);
        };
        let results = match filter {
            None => self.ordered_index_plain_results(stmt, field),
            Some(cond) => self.ordered_index_filtered_results(stmt, field, &cond),
        };
        let Some(results) = results else {
            return Ok(None);
        };
        self.check_guardrails_and_record(ctx, results.len())?;
        self.guard_rails.circuit_breaker.record_success();
        Ok(Some(results))
    }

    /// Plain (no-WHERE) route: hydrate the top `offset+limit` ordered IDs. Score
    /// is 1.0 to match the exhaustive metadata-scan path. Records an advisor
    /// observation and returns `None` when the index is missing or not fully
    /// covering (EPIC-081 phase 3a).
    fn ordered_index_plain_results(
        &self,
        stmt: &crate::velesql::SelectStatement,
        field: &str,
    ) -> Option<Vec<SearchResult>> {
        let Some(page) = self.ordered_index_page(stmt, field) else {
            self.order_by_advisor.write().observe(field);
            return None;
        };
        Some(
            self.get(&page)
                .into_iter()
                .flatten()
                .map(|point| SearchResult::new(point, 1.0))
                .collect(),
        )
    }

    /// WHERE-filtered route (EPIC-081 phase 3b): walk the fully-covered ordered
    /// index applying the same metadata predicate the exhaustive path applies,
    /// stopping once the OFFSET+LIMIT page is filled.
    ///
    /// Declines (returns `None`) **without** observing the advisor when the
    /// collection exceeds `MAX_LIMIT` (matching the capped exhaustive baseline)
    /// or the filter is too selective for the walk to beat the exhaustive bitmap
    /// prefilter — in those cases a covering index would not enable the route, so
    /// it is not an advisable gap. It *does* observe when the only thing missing
    /// is a covering index.
    fn ordered_index_filtered_results(
        &self,
        stmt: &crate::velesql::SelectStatement,
        field: &str,
        metadata_cond: &Condition,
    ) -> Option<Vec<SearchResult>> {
        let point_count = self.len();
        if point_count > MAX_LIMIT {
            return None;
        }
        let selectivity = crate::velesql::CostEstimator::new(&self.get_stats())
            .estimate_condition_selectivity(metadata_cond)
            .clamp(0.001, 1.0);
        if selectivity < MIN_FILTERED_ROUTE_SELECTIVITY {
            return None;
        }
        // Snapshot every covered ordered id under one lock + coverage check, then
        // release before hydrating.
        let Some(ids) = self.ordered_ids_if_covered(field, order_descending(stmt), point_count)
        else {
            self.order_by_advisor.write().observe(field);
            return None;
        };
        let predicate =
            crate::filter::Filter::new(crate::filter::Condition::from(metadata_cond.clone()));
        let (limit, _) = Self::compute_fetch_limit(stmt);
        Some(self.collect_filtered_page(&ids, &predicate, order_offset(stmt), limit))
    }

    /// Hydrates `ids` (already in ORDER BY order) in batches, keeps rows passing
    /// `predicate`, skips the first `offset` matches, and collects up to `limit`
    /// — stopping as soon as the page is full so a broad filter does not hydrate
    /// the whole collection.
    fn collect_filtered_page(
        &self,
        ids: &[u64],
        predicate: &crate::filter::Filter,
        offset: usize,
        limit: usize,
    ) -> Vec<SearchResult> {
        let mut out: Vec<SearchResult> = Vec::new();
        if limit == 0 {
            return out;
        }
        let null = serde_json::Value::Null;
        let mut skipped = 0usize;
        for chunk in ids.chunks(FILTER_HYDRATION_CHUNK) {
            for point in self.get(chunk).into_iter().flatten() {
                if !predicate.matches(point.payload.as_ref().unwrap_or(&null)) {
                    continue;
                }
                if skipped < offset {
                    skipped += 1;
                    continue;
                }
                out.push(SearchResult::new(point, 1.0));
                if out.len() >= limit {
                    return out;
                }
            }
        }
        out
    }

    /// Resolves the page of point IDs for the plain route: the top `offset+limit`
    /// ordered IDs from the covered index, OFFSET then LIMIT applied. `None` when
    /// the index is missing or not fully covered (caller then falls back to the
    /// exhaustive path, which places field-missing rows first for ASC / last for
    /// DESC).
    fn ordered_index_page(
        &self,
        stmt: &crate::velesql::SelectStatement,
        field: &str,
    ) -> Option<Vec<u64>> {
        let (limit, fetch_limit) = Self::compute_fetch_limit(stmt);
        let ids = self.ordered_ids_if_covered(field, order_descending(stmt), fetch_limit)?;
        Some(
            ids.into_iter()
                .skip(order_offset(stmt))
                .take(limit)
                .collect(),
        )
    }
}

/// Whether the leading ORDER BY key is descending.
fn order_descending(stmt: &crate::velesql::SelectStatement) -> bool {
    stmt.order_by
        .as_ref()
        .and_then(|ob| ob.first())
        .is_some_and(|first| first.descending)
}

/// OFFSET as a `usize` (SQL-standard: applied after ORDER BY, before LIMIT).
fn order_offset(stmt: &crate::velesql::SelectStatement) -> usize {
    stmt.offset
        .map_or(0, |o| usize::try_from(o).unwrap_or(usize::MAX))
}

/// Returns `Some((field, filter))` when the query is eligible for the
/// index-backed `ORDER BY <field> LIMIT k` fast path. `filter` is `Some(cond)`
/// for the WHERE-filtered route (the pure-metadata predicate, identical to the
/// one the exhaustive path applies) or `None` for the plain route.
///
/// Eligible shape (all required): the ORDER BY is exactly **one** plain `Field`
/// key (not Aggregate / Arithmetic / similarity); no JOIN, no DISTINCT, no
/// GROUP BY / HAVING, and a plain (non-computed) projection — no aggregate,
/// window function, `similarity()` score, or qualified wildcard. The only WHERE
/// permitted is a pure-metadata filter; any vector / similarity / sparse / graph
/// MATCH / union / NOT-similarity fetch declines. Coverage and index existence
/// are verified later via `ordered_ids_if_covered`.
fn ordered_index_scan_applies<'a>(
    stmt: &'a crate::velesql::SelectStatement,
    extracted: &ExtractedComponents,
) -> Option<(&'a str, Option<Condition>)> {
    let single_field = match stmt.order_by.as_deref() {
        Some([only]) => match &only.expr {
            crate::velesql::OrderByExpr::Field(name) => name.as_str(),
            _ => return None,
        },
        _ => return None,
    };
    if !plain_query_shape(stmt) {
        return None;
    }
    let filter = match route_metadata_filter(stmt, extracted) {
        MetadataRoute::Decline => return None,
        MetadataRoute::Plain => None,
        MetadataRoute::Filtered(cond) => Some(cond),
    };
    Some((single_field, filter))
}

/// Outcome of classifying a SELECT's WHERE for the ordered-index route.
enum MetadataRoute {
    /// A non-metadata fetch (vector / similarity / sparse / graph / union) or a
    /// WHERE that yields no usable metadata filter → decline the route.
    Decline,
    /// No WHERE → the plain (no-filter) route.
    Plain,
    /// A pure-metadata WHERE → the filtered route with this predicate (the same
    /// one the exhaustive path applies).
    Filtered(Condition),
}

/// Whether the statement carries no clause — other than a metadata `WHERE`,
/// handled separately — that changes the result shape (JOIN / DISTINCT /
/// GROUP BY / HAVING / computed projection).
fn plain_query_shape(stmt: &crate::velesql::SelectStatement) -> bool {
    stmt.joins.is_empty()
        && stmt.distinct == crate::velesql::DistinctMode::None
        && stmt.group_by.is_none()
        && stmt.having.is_none()
        && projection_is_plain(&stmt.columns)
}

/// Classifies the WHERE clause for the route. The filtered case carries the
/// **same** metadata predicate the exhaustive path applies
/// (`extract_metadata_filter`). Any ranked / graph / set-op fetch form — or a
/// WHERE that yields no usable metadata filter — declines, so the route never
/// silently drops a non-metadata predicate.
fn route_metadata_filter(
    stmt: &crate::velesql::SelectStatement,
    extracted: &ExtractedComponents,
) -> MetadataRoute {
    if has_non_metadata_fetch(extracted) {
        return MetadataRoute::Decline;
    }
    match &extracted.filter_condition {
        Some(cond) => match Collection::extract_metadata_filter(cond) {
            Some(metadata) => MetadataRoute::Filtered(metadata),
            None => MetadataRoute::Decline,
        },
        None if stmt.where_clause.is_none() => MetadataRoute::Plain,
        None => MetadataRoute::Decline,
    }
}

/// Whether the extracted components carry a ranked / graph / set-op fetch
/// (vector / similarity / sparse / graph MATCH / union / NOT-similarity), all of
/// which need the regular dispatch and disqualify the ordered-index route.
fn has_non_metadata_fetch(extracted: &ExtractedComponents) -> bool {
    extracted.vector_search.is_some()
        || !extracted.similarity_conditions.is_empty()
        || !extracted.graph_match_predicates.is_empty()
        || extracted.sparse_vector_search.is_some()
        || extracted.is_union_query
        || extracted.is_not_similarity_query
}

/// Returns `true` when the projection is "plain" — `SELECT *` or a bare column
/// list — so the fast path reproduces it exactly. The fast path returns its page
/// directly (`mod.rs`), bypassing the post-processing stage that runs DISTINCT,
/// **window functions** (`select_dispatch::evaluate`) and similarity scoring; any
/// *computed* projection (aggregate, window function, `similarity()` score, or
/// qualified wildcard) needs that bypassed stage and therefore disqualifies the
/// route. The `Mixed` arm names every field (no `..`) so a future computed field
/// is a compile error here, not a silently-dropped projection.
fn projection_is_plain(columns: &crate::velesql::SelectColumns) -> bool {
    use crate::velesql::SelectColumns;
    match columns {
        SelectColumns::All | SelectColumns::Columns(_) => true,
        SelectColumns::Mixed {
            columns: _,
            aggregations,
            similarity_scores,
            qualified_wildcards,
            window_functions,
        } => {
            aggregations.is_empty()
                && similarity_scores.is_empty()
                && qualified_wildcards.is_empty()
                && window_functions.is_empty()
        }
        SelectColumns::Aggregations(_)
        | SelectColumns::SimilarityScore(_)
        | SelectColumns::QualifiedWildcard(_) => false,
    }
}
