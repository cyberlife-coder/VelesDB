//! Index-backed `ORDER BY <field> LIMIT k` fast path (EPIC-081 phase 2).
//!
//! Routes a plain scalar `ORDER BY <indexed_field> LIMIT k` to the field's
//! ordered secondary B-tree index (`O(log n + k)`) instead of the B001
//! exhaustive `MAX_LIMIT` fetch + in-memory sort (`O(n log n)`). The route is
//! gated hard and falls through to the exact pre-existing behaviour for every
//! shape it does not cover — see [`ordered_index_scan_applies`].

use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::SearchResult;

use super::ExtractedComponents;

impl Collection {
    /// Attempts the ordered-index fast path. Returns `Ok(Some(results))` when it
    /// fires (the page is already OFFSET/LIMIT-applied and ordered), `Ok(None)`
    /// to fall through to today's exact behaviour.
    ///
    /// Fires only when ALL hold (see [`ordered_index_scan_applies`]): no LET
    /// bindings; the ORDER BY is a single plain `Field`; that field has a
    /// fully-covering secondary index; and the query carries no WHERE / JOIN /
    /// graph / vector / sparse / DISTINCT / GROUP BY / aggregate. Because the
    /// route is restricted to fully-covered fields, the result id-sequence is
    /// identical to the exhaustive sort truncated to k — the full-scan sort's
    /// ascending-id tie-break (added in `ordering.rs`) mirrors the index walk.
    ///
    /// The ordered IDs are snapshotted under the secondary-index read lock,
    /// which is released before `get` re-reads payloads/vectors — so the
    /// hydration tolerates concurrent writes and respects the lock order.
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
        let Some(field) = ordered_index_scan_applies(stmt, extracted) else {
            return Ok(None);
        };
        let Some(page) = self.ordered_index_page(stmt, field) else {
            // Eligible shape but no covering index → record an advisor
            // observation (EPIC-081 phase 3a), then fall through unchanged. The
            // index lock taken by `ordered_index_page` is already released here,
            // so the brief advisor write lock holds nothing else.
            self.order_by_advisor.write().observe(field);
            return Ok(None);
        };

        // Hydrate the snapshot; `get` drops deleted/TTL-expired ids (returned as
        // `None`), matching the read contract. Score is 1.0 to match the
        // exhaustive metadata-scan path (scalar `ORDER BY` carries no similarity
        // score), so results are identical with or without the index.
        let results: Vec<SearchResult> = self
            .get(&page)
            .into_iter()
            .flatten()
            .map(|point| SearchResult::new(point, 1.0))
            .collect();

        self.check_guardrails_and_record(ctx, results.len())?;
        self.guard_rails.circuit_breaker.record_success();
        Ok(Some(results))
    }

    /// Resolves the page of point IDs for the fast path: the top `offset+limit`
    /// ordered IDs from the covered index, with OFFSET then LIMIT applied.
    /// Returns `None` when the index is missing or not fully covered (caller
    /// then falls back to the exhaustive path, which places field-missing rows
    /// first for ASC / last for DESC).
    fn ordered_index_page(
        &self,
        stmt: &crate::velesql::SelectStatement,
        field: &str,
    ) -> Option<Vec<u64>> {
        let (limit, fetch_limit) = Self::compute_fetch_limit(stmt);
        let descending = stmt
            .order_by
            .as_ref()
            .and_then(|ob| ob.first())
            .is_some_and(|first| first.descending);
        let ids = self.ordered_ids_if_covered(field, descending, fetch_limit)?;
        // SQL-standard: OFFSET applied after ORDER BY, before LIMIT.
        let offset = stmt
            .offset
            .map_or(0, |o| usize::try_from(o).unwrap_or(usize::MAX));
        Some(ids.into_iter().skip(offset).take(limit).collect())
    }
}

/// Returns `Some(field)` when the query is eligible for the index-backed
/// `ORDER BY <field> LIMIT k` fast path, `None` otherwise.
///
/// Eligible shape (all required): the ORDER BY is exactly **one** plain
/// `Field` key (not Aggregate / Arithmetic / similarity); no WHERE / filter,
/// no JOIN, no graph MATCH, no vector / similarity / sparse search, no
/// DISTINCT, no GROUP BY / HAVING, and a plain (non-computed) projection — no
/// aggregate, window function, `similarity()` score, or qualified wildcard.
/// Coverage and index existence are verified later via `ordered_ids_if_covered`.
fn ordered_index_scan_applies<'a>(
    stmt: &'a crate::velesql::SelectStatement,
    extracted: &ExtractedComponents,
) -> Option<&'a str> {
    let single_field = match stmt.order_by.as_deref() {
        Some([only]) => match &only.expr {
            crate::velesql::OrderByExpr::Field(name) => name.as_str(),
            _ => return None,
        },
        _ => return None,
    };
    (plain_query_shape(stmt) && plain_fetch_shape(extracted)).then_some(single_field)
}

/// Whether the statement carries no clause that changes the result shape
/// (WHERE / JOIN / DISTINCT / GROUP BY / HAVING / computed projection).
fn plain_query_shape(stmt: &crate::velesql::SelectStatement) -> bool {
    stmt.where_clause.is_none()
        && stmt.joins.is_empty()
        && stmt.distinct == crate::velesql::DistinctMode::None
        && stmt.group_by.is_none()
        && stmt.having.is_none()
        && projection_is_plain(&stmt.columns)
}

/// Whether the extracted components carry no ranked / graph / set-op fetch
/// (vector / similarity / sparse / graph MATCH / residual filter / union /
/// NOT-similarity), all of which need the regular dispatch.
fn plain_fetch_shape(extracted: &ExtractedComponents) -> bool {
    extracted.vector_search.is_none()
        && extracted.similarity_conditions.is_empty()
        && extracted.graph_match_predicates.is_empty()
        && extracted.sparse_vector_search.is_none()
        && extracted.filter_condition.is_none()
        && !extracted.is_union_query
        && !extracted.is_not_similarity_query
}

/// Returns `true` when the projection is "plain" — `SELECT *` or a bare column
/// list — so the fast path reproduces it exactly. The fast path returns its
/// page directly (`mod.rs`), bypassing the post-processing stage that runs
/// DISTINCT, **window functions** (`select_dispatch::evaluate`) and similarity
/// scoring; any *computed* projection (aggregate, window function, `similarity()`
/// score, or qualified wildcard) needs that bypassed stage and therefore
/// disqualifies the route. The `Mixed` arm names every field (no `..`) so a
/// future computed field is a compile error here, not a silently-dropped
/// projection.
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
