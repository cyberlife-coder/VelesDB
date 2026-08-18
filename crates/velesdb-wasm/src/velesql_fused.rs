//! `NEAR_FUSED` multi-vector fusion for the WASM VelesQL executor.
//!
//! Mirrors core's
//! [`dispatch_fused_query`](velesdb_core::collection::Collection) semantics with
//! WASM's brute-force primitives: resolve the N query vectors from the
//! `VectorFusedSearch`, run a per-vector brute-force similarity scan, fuse the N
//! ranked lists via the [`FusionStrategy`] mapped from the fusion config
//! (`rrf(k)` / `average` / `maximum`; any other strategy falls back to RRF —
//! matching core's `fused_config_to_strategy`), then apply the residual metadata
//! AND-filter as a pre-fusion filter.
//!
//! Isolation contract (mirrors core's `validate_similarity_query_structure`):
//! a `NEAR_FUSED` leaf must be the only vector predicate and cannot appear under
//! `OR` / `NOT`. More than one fused, a fused mixed with `NEAR` / `similarity()`
//! / `SPARSE_NEAR`, or a fused under `OR`/`NOT` is rejected so the fused vectors
//! are never silently dropped to a non-fused scan.

use velesdb_core::fusion::FusionStrategy;
use velesdb_core::velesql::{Condition, FusionConfig, SelectStatement, VectorFusedSearch};

use crate::database::DatabaseInner;
use crate::velesql_scan::OwnedScanRow;
use crate::velesql_value::{resolve_vector, Params};
use crate::velesql_where;

/// Finds a top-level `NEAR_FUSED` leaf in the WHERE clause, recursing through
/// the boolean combinators (the isolation contract is enforced separately by
/// [`validate_fused_structure`]).
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
    validate_fused_structure(stmt.where_clause.as_ref())?;

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
    let branches = score_branches(&vectors, &borrowed, residual.as_ref(), params)?;
    let strategy = config_to_strategy(&fused.fusion);
    let fused_scores = strategy.fuse(branches).unwrap_or_default();

    // Hydrate rows via the shared collector (same id->idx map + drop-unknown
    // semantics as the single-vector fusion path).
    crate::velesql_select::collect_vector_rows_unfiltered(&fused_scores, &borrowed)
}

/// Resolves and dimension-checks each fused query vector. Rejects an empty
/// fused list (mirrors core's `multi_query_search` "at least one vector").
fn resolve_fused_vectors(
    fused: &VectorFusedSearch,
    store: &crate::vector_store::VectorStore,
    params: &Params,
) -> Result<Vec<Vec<f32>>, String> {
    if fused.vectors.is_empty() {
        return Err("NEAR_FUSED requires at least one query vector".to_string());
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

/// Builds one ranked branch per query vector via brute-force scoring, applying
/// the residual metadata filter pre-fusion (matching core's
/// `apply_pre_fusion_filter`).
fn score_branches(
    vectors: &[Vec<f32>],
    store: &crate::vector_store::VectorStore,
    residual: Option<&Condition>,
    params: &Params,
) -> Result<Vec<Vec<(u64, f32)>>, String> {
    let keep = passing_ids(store, residual, params)?;
    let mut branches = Vec::with_capacity(vectors.len());
    for query in vectors {
        let mut scored = crate::velesql_select::score_all(query, store);
        if let Some(allowed) = keep.as_ref() {
            scored.retain(|(id, _)| allowed.contains(id));
        }
        branches.push(scored);
    }
    Ok(branches)
}

/// Returns the set of ids passing the residual metadata predicate, or `None`
/// when there is no residual (no filtering required).
fn passing_ids(
    store: &crate::vector_store::VectorStore,
    residual: Option<&Condition>,
    params: &Params,
) -> Result<Option<std::collections::HashSet<u64>>, String> {
    let Some(cond) = residual else {
        return Ok(None);
    };
    let mut keep = std::collections::HashSet::new();
    for (idx, &id) in store.ids.iter().enumerate() {
        let payload = store.payloads.get(idx).and_then(|p| p.as_ref());
        if velesql_where::matches(cond, id, payload, params)? {
            keep.insert(id);
        }
    }
    Ok(Some(keep))
}

/// Maps a `NEAR_FUSED` [`FusionConfig`] to a [`FusionStrategy`], mirroring
/// core's `fused_config_to_strategy`: `average` / `maximum` map directly,
/// everything else (incl. `weighted` / `rsf` / unknown) falls back to `rrf`.
fn config_to_strategy(config: &FusionConfig) -> FusionStrategy {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let k = config.params.get("k").map_or(60, |v| *v as u32);
    match config.strategy.to_lowercase().as_str() {
        "average" => FusionStrategy::Average,
        "maximum" => FusionStrategy::Maximum,
        _ => FusionStrategy::RRF { k },
    }
}

/// Strips every vector / fused / sparse leaf from the WHERE clause, leaving the
/// residual metadata predicate (mirrors core's `extract_metadata_filter`).
///
/// `push_not_inward` runs first so `NOT` is De-Morgan-distributed before the
/// strip — symmetric with the single-vector path in `velesql_select`.
fn residual_metadata_filter(cond: Option<&Condition>) -> Option<Condition> {
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

// --- Isolation contract --------------------------------------------------

/// Rejects fused shapes the executor cannot honor, mirroring core's
/// `validate_similarity_query_structure`: a `NEAR_FUSED` must be the only vector
/// predicate and cannot appear under `OR` / `NOT`.
fn validate_fused_structure(cond: Option<&Condition>) -> Result<(), String> {
    let Some(cond) = cond else {
        return Ok(());
    };
    let fused_count = count_leaves(cond, &|c| matches!(c, Condition::VectorFusedSearch(_)));
    if fused_count == 0 {
        return Ok(());
    }
    let other_vector = count_leaves(cond, &|c| {
        matches!(
            c,
            Condition::VectorSearch(_)
                | Condition::SparseVectorSearch(_)
                | Condition::Similarity(_)
        )
    });
    if fused_count > 1 || other_vector > 0 || fused_under_or_not(cond) {
        return Err(
            "NEAR_FUSED must be the only vector predicate and cannot appear under \
                    OR/NOT; combine it only with AND <metadata filter>."
                .to_string(),
        );
    }
    Ok(())
}

/// Counts leaves matching `pred` across the condition tree.
fn count_leaves(cond: &Condition, pred: &dyn Fn(&Condition) -> bool) -> usize {
    match cond {
        Condition::And(l, r) | Condition::Or(l, r) => count_leaves(l, pred) + count_leaves(r, pred),
        Condition::Not(inner) | Condition::Group(inner) => count_leaves(inner, pred),
        c => usize::from(pred(c)),
    }
}

/// True if any `NEAR_FUSED` leaf sits under an `OR` or `NOT`.
fn fused_under_or_not(cond: &Condition) -> bool {
    let has_fused =
        |c: &Condition| count_leaves(c, &|x| matches!(x, Condition::VectorFusedSearch(_))) > 0;
    match cond {
        Condition::Or(l, r) => {
            has_fused(l) || has_fused(r) || fused_under_or_not(l) || fused_under_or_not(r)
        }
        Condition::Not(inner) => has_fused(inner) || fused_under_or_not(inner),
        Condition::And(l, r) => fused_under_or_not(l) || fused_under_or_not(r),
        Condition::Group(inner) => fused_under_or_not(inner),
        _ => false,
    }
}

#[cfg(test)]
#[path = "velesql_fused_tests.rs"]
mod tests;
