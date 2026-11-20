//! `SPARSE_NEAR` for the WASM VelesQL executor: a sparse-only search, or the
//! hybrid dense + sparse search core runs when a `NEAR` sits beside it.
//!
//! Core's contract (`Collection::dispatch_sparse_query`), over WASM's
//! brute-force dense scoring and the store's sparse index:
//!
//! - the operand resolved by core's own
//!   [`SparseVectorExpr::resolve`](velesdb_core::velesql::SparseVectorExpr::resolve)
//!   (literal or `$param`, finite weights only);
//! - the store's one sparse index: it is the default index, and a store
//!   without sparse vectors, or a named index, is refused as core refuses a
//!   missing one;
//! - both branches bounded by the rest of the WHERE clause before they rank,
//!   as core pushes its metadata filter into each branch; a `similarity()`
//!   threshold there is evaluated as the dense SELECT path evaluates it
//!   ([`eligible_ids`](crate::velesql_select::eligible_ids));
//! - sparse only: the `LIMIT + OFFSET` best sparse hits, scored by the sparse
//!   kernel;
//! - hybrid: [`dense_sparse_candidate_k`] points from each branch, fused by
//!   the strategy core maps from `USING FUSION`
//!   ([`FusionClause::dense_sparse_strategy`], RRF without one), the dense
//!   branch read in the metric's direction and the sparse one as
//!   higher-is-better (`fuse_with_directions`);
//! - an aggregation reads every point its metadata filter keeps, as core's
//!   `execute_aggregate` does, where a vector leaf filters no point.

use std::collections::HashSet;

use velesdb_core::fusion::{dense_sparse_candidate_k, FusionStrategy, ScoreDirection};
use velesdb_core::sparse_index::{SparseVector, DEFAULT_SPARSE_INDEX_NAME};
use velesdb_core::velesql::{
    Condition, FusionClause, SelectStatement, SparseVectorSearch, VectorSearch,
};

use crate::database::DatabaseInner;
use crate::exec_error::ExecError;
use crate::sparse::SparseIndex;
use crate::store_state::StoreState;
use crate::velesql_scan::OwnedScanRow;
use crate::velesql_value::Params;

/// Finds the `SPARSE_NEAR` leaf of a WHERE clause, recursing through the
/// boolean combinators. There is at most one: core's
/// `validate_vector_predicates` refuses a second before dispatch.
pub(crate) fn find_sparse_search(cond: Option<&Condition>) -> Option<&SparseVectorSearch> {
    match cond? {
        Condition::SparseVectorSearch(svs) => Some(svs),
        Condition::And(l, r) | Condition::Or(l, r) => {
            find_sparse_search(Some(l)).or_else(|| find_sparse_search(Some(r)))
        }
        Condition::Not(inner) | Condition::Group(inner) => find_sparse_search(Some(inner)),
        _ => None,
    }
}

/// Executes a SELECT whose WHERE holds `svs`, beside a `NEAR` or alone, and
/// returns its ranked `(id, score, payload)` rows; the caller windows them.
pub(crate) fn execute_sparse_search(
    db: &DatabaseInner,
    stmt: &SelectStatement,
    svs: &SparseVectorSearch,
    params: &Params,
) -> Result<Vec<OwnedScanRow>, ExecError> {
    let store = db.get_shared_store(&stmt.from)?;
    let store = store.borrow();
    let keep = crate::velesql_select::eligible_ids(db, stmt, &store, params)?;
    if stmt.is_aggregation_query() {
        return Ok(every_kept_point(&store, keep.as_ref()));
    }
    let search = SparseQuery {
        index: sparse_index(&store, svs)?,
        vector: svs.vector.resolve(params)?,
        keep: keep.as_ref(),
    };
    let fetch = crate::velesql_fused::fetch_limit(stmt);
    let scored = match crate::velesql_select::find_vector_search(stmt.where_clause.as_ref()) {
        None => search.ranked(fetch),
        Some(dense) => fuse_hybrid(stmt, dense, &search, &store, params, fetch)?,
    };
    crate::velesql_select::collect_vector_rows_unfiltered(&scored, &store)
}

/// The sparse side of a search: the index, the query, and the ids the
/// rest of the WHERE clause keeps (`None` keeps every id).
struct SparseQuery<'a> {
    index: &'a SparseIndex,
    vector: SparseVector,
    keep: Option<&'a HashSet<u64>>,
}

impl SparseQuery<'_> {
    /// The `k` best kept documents, best first.
    fn ranked(&self, k: usize) -> Vec<(u64, f32)> {
        self.index.search_filtered(&self.vector, k, self.keep)
    }
}

/// The store's sparse index, the one a `SPARSE_NEAR` names: WASM keeps a
/// single, default, index per store. A missing one is refused with core's
/// refusal ([`velesdb_core::Error::sparse_index_not_found`], VELES-009).
fn sparse_index<'a>(
    store: &'a StoreState,
    svs: &SparseVectorSearch,
) -> Result<&'a SparseIndex, ExecError> {
    let name = svs
        .index_name
        .as_deref()
        .unwrap_or(DEFAULT_SPARSE_INDEX_NAME);
    store
        .sparse_index
        .as_ref()
        .filter(|_| name == DEFAULT_SPARSE_INDEX_NAME)
        .ok_or_else(|| velesdb_core::Error::sparse_index_not_found(name).into())
}

/// Core's hybrid search: each branch's `dense_sparse_candidate_k(fetch)` best
/// kept points, fused in each branch's score direction, the `fetch` best kept.
fn fuse_hybrid(
    stmt: &SelectStatement,
    dense: &VectorSearch,
    sparse: &SparseQuery<'_>,
    store: &StoreState,
    params: &Params,
    fetch: usize,
) -> Result<Vec<(u64, f32)>, ExecError> {
    let query = crate::velesql_select::resolve_query_vector(dense, params)?;
    let depth = dense_sparse_candidate_k(fetch);
    let mut dense_branch = crate::velesql_select::score_all(&query, store);
    if let Some(keep) = sparse.keep {
        dense_branch.retain(|(id, _)| keep.contains(id));
    }
    dense_branch.truncate(depth);
    let sparse_branch = sparse.ranked(depth);
    if dense_branch.is_empty() && sparse_branch.is_empty() {
        return Ok(Vec::new());
    }
    let strategy = stmt.fusion_clause.as_ref().map_or_else(
        FusionStrategy::rrf_default,
        FusionClause::dense_sparse_strategy,
    );
    let directions = [
        store.metric.score_direction(),
        ScoreDirection::HigherIsBetter,
    ];
    let mut fused = strategy
        .fuse_with_directions(vec![dense_branch, sparse_branch], &directions)
        .map_err(velesdb_core::Error::from)?;
    fused.truncate(fetch);
    Ok(fused)
}

/// Every point the metadata filter keeps, with a metadata match's score.
fn every_kept_point(store: &StoreState, keep: Option<&HashSet<u64>>) -> Vec<OwnedScanRow> {
    store
        .ids
        .iter()
        .enumerate()
        .filter(|(_, id)| keep.is_none_or(|k| k.contains(id)))
        .map(|(idx, &id)| {
            let payload = store.payloads.get(idx).and_then(Clone::clone);
            OwnedScanRow::new(id, velesdb_core::velesql::METADATA_MATCH_SCORE, payload)
        })
        .collect()
}

#[cfg(test)]
#[path = "velesql_sparse_tests.rs"]
mod tests;

// Host-only: compares against a `velesdb-core` database, which needs core's
// `persistence` feature (see `Cargo.toml`).
#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "velesql_sparse_parity_tests.rs"]
mod parity_tests;
