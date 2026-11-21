//! `NEAR_FUSED` multi-vector fusion for the WASM VelesQL executor.
//!
//! Core's `Collection::multi_query_search` contract, over WASM's brute-force
//! scoring: at most `velesdb_core::fusion::MAX_FUSED_VECTORS` query vectors;
//! one branch per vector holding the best passing points up to
//! `branch_depth(fetch)`, where [`CandidateWindow`] sizes the candidate set
//! for the statement; the branches fused, in the
//! metric's score direction, by the strategy core maps from the fusion
//! config ([`velesdb_core::velesql::FusionConfig::near_fused_strategy`]).
//!
//! Isolation contract (mirrors core's `validate_similarity_query_structure`):
//! a `NEAR_FUSED` leaf must be the only vector predicate and cannot appear under
//! `OR` / `NOT`. More than one fused, a fused mixed with `NEAR` / `similarity()`
//! / `SPARSE_NEAR`, or a fused under `OR`/`NOT` is rejected so the fused vectors
//! are never silently dropped to a non-fused scan.

use velesdb_core::velesql::{Condition, SelectStatement, VectorFusedSearch};

use crate::database::DatabaseInner;
use crate::exec_error::ExecError;
use crate::velesql_scan::OwnedScanRow;
use crate::velesql_value::Params;
use crate::velesql_where;

/// Finds a top-level `NEAR_FUSED` leaf in the WHERE clause, recursing through
/// the boolean combinators (the isolation contract is enforced before
/// dispatch by `velesdb_core::velesql::validate_vector_predicates`).
pub(crate) fn find_fused_search(cond: Option<&Condition>) -> Option<&VectorFusedSearch> {
    let cond = cond?;
    match cond {
        Condition::VectorFusedSearch(vfs) => Some(vfs),
        Condition::And(l, r) | Condition::Or(l, r) => {
            find_fused_search(Some(l)).or_else(|| find_fused_search(Some(r)))
        }
        Condition::Not(inner) | Condition::Group(inner) => find_fused_search(Some(inner)),
        _ => None,
    }
}

/// Executes a `NEAR_FUSED` SELECT and returns its raw (id, score, payload) rows.
///
/// `finalize` (DISTINCT / ORDER BY / LIMIT / OFFSET) is applied by the caller,
/// exactly as for the plain and single-vector paths.
pub(crate) fn execute_fused_search(
    db: &DatabaseInner,
    stmt: &SelectStatement,
    fused: &VectorFusedSearch,
    params: &Params,
) -> Result<Vec<OwnedScanRow>, ExecError> {
    let store = db.get_shared_store(&stmt.from)?;
    let borrowed = store.borrow();
    let vectors = resolve_fused_vectors(fused, params)?;
    let residual = residual_metadata_filter(stmt.where_clause.as_ref());
    let window = CandidateWindow::for_search(stmt, false);
    let fetch = window.count();
    let depth = velesdb_core::fusion::branch_depth(fetch);
    let branches = score_branches(&vectors, &borrowed, residual.as_ref(), params, depth)?;
    let directions = vec![borrowed.metric.score_direction(); branches.len()];
    let mut fused_scores = fused
        .fusion
        .near_fused_strategy()
        .fuse_with_directions(branches, &directions)
        .map_err(velesdb_core::Error::from)?;
    fused_scores.truncate(fetch);
    window.warn_if_full(&stmt.from, fused_scores.len());

    // Hydrate rows via the shared collector (same id->idx map + drop-unknown
    // semantics as the single-vector fusion path).
    crate::velesql_select::collect_vector_rows_unfiltered(&fused_scores, &borrowed)
}

/// Binds each fused query vector and checks their count as core's
/// `multi_query_search` does: one to `MAX_FUSED_VECTORS` vectors
/// ([`velesdb_core::fusion::validate_fused_vector_count`]). Each vector's
/// dimension was checked with the statement's other query vectors before
/// the scan (`velesql_select::check_query_vectors`).
fn resolve_fused_vectors(
    fused: &VectorFusedSearch,
    params: &Params,
) -> Result<Vec<Vec<f32>>, ExecError> {
    velesdb_core::fusion::validate_fused_vector_count(fused.vectors.len())?;
    let mut out = Vec::with_capacity(fused.vectors.len());
    for expr in &fused.vectors {
        out.push(velesdb_core::velesql::bind_vector(expr, params)?);
    }
    Ok(out)
}

/// One ranked branch per query vector: the `depth` best points that pass the
/// residual metadata filter, best first (core's `multi_query_search` takes
/// them from `search_with_filter`).
fn score_branches(
    vectors: &[Vec<f32>],
    store: &crate::store_state::StoreState,
    residual: Option<&Condition>,
    params: &Params,
    depth: usize,
) -> Result<Vec<Vec<(u64, f32)>>, ExecError> {
    let keep = passing_ids(store, residual, params)?;
    let mut branches = Vec::with_capacity(vectors.len());
    for query in vectors {
        let mut scored = crate::velesql_select::score_all(query, store);
        if let Some(allowed) = keep.as_ref() {
            scored.retain(|(id, _)| allowed.contains(id));
        }
        scored.truncate(depth);
        branches.push(scored);
    }
    Ok(branches)
}

/// The candidates needed before the sparse or fused ranking's output window.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CandidateWindow {
    /// The search can return the requested rows, including all rows for a scalar aggregate.
    Requested(usize),
    /// The SELECT ceiling can exclude rows needed by the final order or partition.
    Capped,
}

impl CandidateWindow {
    pub(crate) fn for_search(stmt: &SelectStatement, ranks_by_descending_similarity: bool) -> Self {
        use velesdb_core::velesql::{DEFAULT_SELECT_LIMIT, MAX_SELECT_LIMIT};
        if stmt.is_aggregation_query() {
            return Self::Requested(usize::MAX);
        }
        let requested = stmt
            .limit
            .unwrap_or(DEFAULT_SELECT_LIMIT)
            .saturating_add(stmt.offset.unwrap_or(0));
        if stmt.reshapes_rows_before_window()
            || !search_preserves_order(stmt, ranks_by_descending_similarity)
            || requested > MAX_SELECT_LIMIT
        {
            Self::Capped
        } else {
            Self::Requested(usize::try_from(requested).unwrap_or(usize::MAX))
        }
    }

    pub(crate) fn count(self) -> usize {
        match self {
            Self::Requested(count) => count,
            Self::Capped => {
                usize::try_from(velesdb_core::velesql::MAX_SELECT_LIMIT).unwrap_or(usize::MAX)
            }
        }
    }

    pub(crate) fn warn_if_full(self, collection: &str, rows: usize) {
        if self != Self::Capped || rows < self.count() {
            return;
        }
        let warning = format!(
            "SELECT on {collection} reached its {rows}-row candidate ceiling before its final window. \
             Narrow the WHERE to include all candidates in the ranking."
        );
        #[cfg(target_arch = "wasm32")]
        web_sys::console::warn_1(&warning.into());
        #[cfg(not(target_arch = "wasm32"))]
        eprintln!("{warning}");
    }
}

/// A ranked prefix preserves an explicit order only when both use the same single key.
fn search_preserves_order(stmt: &SelectStatement, ranks_by_descending_similarity: bool) -> bool {
    match stmt.order_by.as_deref() {
        None => true,
        Some([key]) => {
            ranks_by_descending_similarity
                && key.descending
                && matches!(key.expr, velesdb_core::velesql::OrderByExpr::SimilarityBare)
        }
        Some(_) => false,
    }
}

/// Returns the set of ids passing the residual metadata predicate, or `None`
/// when there is no residual (no filtering required).
pub(crate) fn passing_ids(
    store: &crate::store_state::StoreState,
    residual: Option<&Condition>,
    params: &Params,
) -> Result<Option<std::collections::HashSet<u64>>, ExecError> {
    let Some(cond) = residual else {
        return Ok(None);
    };
    let filter = velesql_where::WhereFilter::compile(cond, params)?;
    let mut keep = std::collections::HashSet::new();
    for (idx, &id) in store.ids.iter().enumerate() {
        let payload = store.payloads.get(idx).and_then(|p| p.as_ref());
        if filter.matches(id, payload) {
            keep.insert(id);
        }
    }
    Ok(Some(keep))
}

/// Strips every vector / fused / sparse leaf from the WHERE clause, leaving the
/// residual metadata predicate (mirrors core's `extract_metadata_filter`).
///
/// `push_not_inward` runs first so `NOT` is De-Morgan-distributed before the
/// strip — symmetric with the single-vector path in `velesql_select`.
pub(crate) fn residual_metadata_filter(cond: Option<&Condition>) -> Option<Condition> {
    let normalized = cond.cloned().map(crate::velesql_logic::push_not_inward);
    crate::velesql_similarity::strip_condition_if(normalized.as_ref(), &|c| {
        matches!(
            c,
            Condition::VectorSearch(_)
                | Condition::VectorFusedSearch(_)
                | Condition::SparseVectorSearch(_)
        )
    })
}

#[cfg(test)]
#[path = "velesql_fused_tests.rs"]
mod tests;
