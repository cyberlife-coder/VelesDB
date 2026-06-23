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
mod tests {
    use super::*;
    use crate::velesql_value::parse_params;
    use velesdb_core::velesql::Parser;

    fn parse_where(sql: &str) -> Option<Condition> {
        Parser::parse(sql).expect("test: parse").select.where_clause
    }

    fn seed(db: &mut DatabaseInner) {
        db.create_collection("vecs", 4, "cosine")
            .expect("test: create");
        let store = db.get_shared_store("vecs").expect("test: store");
        for (id, v, cat) in [
            (10u64, vec![1.0, 0.0, 0.0, 0.0], "a"),
            (11, vec![0.9, 0.1, 0.0, 0.0], "a"),
            (12, vec![0.0, 1.0, 0.0, 0.0], "b"),
            (13, vec![0.0, 0.0, 1.0, 0.0], "b"),
        ] {
            crate::store_insert::insert_with_payload(
                &mut store.borrow_mut(),
                id,
                &v,
                Some(serde_json::json!({ "cat": cat })),
            );
        }
    }

    fn run(db: &DatabaseInner, sql: &str, params_json: &str) -> Vec<OwnedScanRow> {
        let q = Parser::parse(sql).expect("test: parse");
        let params = parse_params(Some(params_json)).expect("test: params");
        let fused = find_fused_search(q.select.where_clause.as_ref()).expect("test: fused");
        execute_fused_search(db, &q.select, fused, &params).expect("test: fused exec")
    }

    #[test]
    fn test_fused_returns_fused_ranking() {
        let mut db = DatabaseInner::new();
        seed(&mut db);
        // Both query vectors point at id 12 ([0,1,0,0]): $a is exactly it and
        // $b is dominated by the y-axis. So id 12 is the unambiguous top of the
        // fused ranking even though it is stored THIRD — a no-op that returned
        // storage order (10,11,12,13), the bug class this path exists to kill,
        // would put 10 first and fail the assertion below.
        let rows = run(
            &db,
            "SELECT * FROM vecs WHERE vector NEAR_FUSED [$a, $b]",
            r#"{"a": [0.0, 1.0, 0.0, 0.0], "b": [0.1, 0.9, 0.0, 0.0]}"#,
        );
        let ids: Vec<u64> = rows.iter().map(|(id, _, _)| *id).collect();
        assert_eq!(ids.len(), 4, "all four ids present in the fused ranking");
        assert_eq!(
            ids[0], 12,
            "id 12 (favored by both query vectors) must be the top fused result, not storage-order id 10"
        );
        // Fused output is ordered by descending fused score (a real ranking).
        let scores: Vec<f32> = rows.iter().map(|(_, s, _)| *s).collect();
        assert!(
            scores.windows(2).all(|w| w[0] >= w[1]),
            "rows must be sorted by descending fused score, got {scores:?}"
        );
    }

    #[test]
    fn test_fused_with_metadata_filter() {
        let mut db = DatabaseInner::new();
        seed(&mut db);
        let rows = run(
            &db,
            "SELECT * FROM vecs WHERE vector NEAR_FUSED [$a, $b] AND cat = 'a'",
            r#"{"a": [1.0, 0.0, 0.0, 0.0], "b": [0.0, 1.0, 0.0, 0.0]}"#,
        );
        let ids: Vec<u64> = rows.iter().map(|(id, _, _)| *id).collect();
        assert_eq!(
            ids.len(),
            2,
            "only cat='a' rows survive the pre-fusion filter"
        );
        assert!(ids.contains(&10) && ids.contains(&11));
    }

    #[test]
    fn test_fused_rejects_under_or() {
        let mut db = DatabaseInner::new();
        seed(&mut db);
        let q = Parser::parse("SELECT * FROM vecs WHERE vector NEAR_FUSED [$a, $b] OR cat = 'b'")
            .expect("test: parse");
        let params = parse_params(Some(
            r#"{"a": [1.0, 0.0, 0.0, 0.0], "b": [0.0, 1.0, 0.0, 0.0]}"#,
        ))
        .expect("test: params");
        let fused = find_fused_search(q.select.where_clause.as_ref()).expect("test: fused");
        let err = execute_fused_search(&db, &q.select, fused, &params);
        assert!(err.is_err());
        assert!(err.expect_err("test: err").contains("NEAR_FUSED"));
    }

    #[test]
    fn test_fused_rejects_mixed_with_near() {
        let cond =
            parse_where("SELECT * FROM vecs WHERE vector NEAR_FUSED [$a, $b] AND vector NEAR $c");
        let err = validate_fused_structure(cond.as_ref());
        assert!(err.is_err());
    }

    #[test]
    fn test_config_to_strategy_maps_like_core() {
        let mk = |s: &str| FusionConfig {
            strategy: s.to_string(),
            params: std::collections::HashMap::new(),
        };
        assert!(matches!(
            config_to_strategy(&mk("average")),
            FusionStrategy::Average
        ));
        assert!(matches!(
            config_to_strategy(&mk("maximum")),
            FusionStrategy::Maximum
        ));
        assert!(matches!(
            config_to_strategy(&mk("weighted")),
            FusionStrategy::RRF { .. }
        ));
        assert!(matches!(
            config_to_strategy(&mk("rrf")),
            FusionStrategy::RRF { .. }
        ));
    }
}
