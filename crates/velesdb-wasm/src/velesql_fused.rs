//! `NEAR_FUSED` multi-vector fusion for the WASM VelesQL executor.
//!
//! Core's `Collection::multi_query_search` contract, over WASM's brute-force
//! scoring: at most `velesdb_core::fusion::MAX_FUSED_VECTORS` query vectors;
//! one branch per vector holding the `branch_depth(LIMIT + OFFSET)` best points
//! that pass the residual metadata filter (every passing point for an
//! aggregation, see [`branch_depth`]); the branches fused, in the
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
use crate::velesql_scan::OwnedScanRow;
use crate::velesql_value::{resolve_vector, Params};
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
) -> Result<Vec<OwnedScanRow>, String> {
    let store = db.get_shared_store(&stmt.from)?;
    let borrowed = store.borrow();
    if borrowed.dimension == 0 {
        return Err(format!(
            "Collection '{}' is metadata-only; NEAR_FUSED queries require a vector collection",
            stmt.from
        ));
    }

    let vectors = resolve_fused_vectors(fused, &borrowed, params)?;
    let residual = residual_metadata_filter(stmt.where_clause.as_ref());
    let depth = branch_depth(stmt);
    let branches = score_branches(&vectors, &borrowed, residual.as_ref(), params, depth)?;
    let directions = vec![borrowed.metric.score_direction(); branches.len()];
    let fused_scores = fused
        .fusion
        .near_fused_strategy()
        .fuse_with_directions(branches, &directions)
        .map_err(|e| format!("Fusion error: {e}"))?;

    // Hydrate rows via the shared collector (same id->idx map + drop-unknown
    // semantics as the single-vector fusion path).
    crate::velesql_select::collect_vector_rows_unfiltered(&fused_scores, &borrowed)
}

/// Resolves and dimension-checks each fused query vector. Rejects an empty
/// fused list (mirrors core's `multi_query_search` "at least one vector").
fn resolve_fused_vectors(
    fused: &VectorFusedSearch,
    store: &crate::store_state::StoreState,
    params: &Params,
) -> Result<Vec<Vec<f32>>, String> {
    if fused.vectors.is_empty() {
        return Err("NEAR_FUSED requires at least one query vector".to_string());
    }
    let max = velesdb_core::fusion::MAX_FUSED_VECTORS;
    if fused.vectors.len() > max {
        return Err(format!(
            "multi_query_search supports at most {max} vectors, got {}",
            fused.vectors.len()
        ));
    }
    let mut out = Vec::with_capacity(fused.vectors.len());
    for expr in &fused.vectors {
        let v = resolve_vector(expr, params)?;
        if v.len() != store.dimension {
            return Err(format!(
                "NEAR_FUSED query dimension mismatch: expected {}, got {}",
                store.dimension,
                v.len()
            ));
        }
        out.push(v);
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
) -> Result<Vec<Vec<(u64, f32)>>, String> {
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

/// How many points each branch holds.
///
/// A search (rows, or a vector GROUP BY) follows core's `multi_query_search`:
/// the `branch_depth(LIMIT + OFFSET)` best points passing the filter. An
/// aggregation reads the whole collection, as core's `execute_aggregate`
/// does, where a vector leaf filters no point: every branch then holds every
/// passing point, so neither LIMIT nor the default window bounds what an
/// aggregate counts.
fn branch_depth(stmt: &SelectStatement) -> usize {
    if stmt.is_aggregation_query() {
        usize::MAX
    } else {
        velesdb_core::fusion::branch_depth(fetch_limit(stmt))
    }
}

/// The rows a SELECT window needs: `LIMIT` (core's default when absent) plus
/// `OFFSET` (core's `compute_fetch_limit`).
pub(crate) fn fetch_limit(stmt: &SelectStatement) -> usize {
    let limit = stmt
        .limit
        .unwrap_or(velesdb_core::velesql::DEFAULT_SELECT_LIMIT);
    let offset = stmt.offset.unwrap_or(0);
    usize::try_from(limit.saturating_add(offset)).unwrap_or(usize::MAX)
}

/// Returns the set of ids passing the residual metadata predicate, or `None`
/// when there is no residual (no filtering required).
pub(crate) fn passing_ids(
    store: &crate::store_state::StoreState,
    residual: Option<&Condition>,
    params: &Params,
) -> Result<Option<std::collections::HashSet<u64>>, String> {
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
