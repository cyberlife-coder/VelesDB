//! Integration tests for hybrid dense+sparse search execution.

use crate::collection::types::Collection;
use crate::index::sparse::SparseVector;
use crate::point::Point;
use std::collections::{BTreeMap, HashMap};
use tempfile::TempDir;

/// Helper: create a collection and insert points with both dense and sparse vectors.
fn setup_hybrid_collection() -> (TempDir, Collection) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("hybrid_col");
    let col = Collection::create(path, 4, crate::distance::DistanceMetric::Cosine)
        .expect("Failed to create collection");

    // Insert 12 points with varying dense + sparse profiles.
    // Points 0-5: have both dense and sparse vectors (term 1, 2).
    // Points 6-9: dense only.
    // Points 10-11: have sparse only (dense vector still required by collection schema).
    let mut points = Vec::new();
    for i in 0u64..12 {
        #[allow(clippy::cast_precision_loss)]
        let fi = i as f32;
        let dense = vec![fi / 12.0, 0.5, 0.3, 0.1];
        let sparse = if (6..10).contains(&i) {
            None
        } else {
            let mut map = BTreeMap::new();
            // Different weights per point so sparse search produces ranking.
            #[allow(clippy::cast_precision_loss)]
            let w = 1.0 + i as f32;
            map.insert(
                String::new(), // default sparse index name
                SparseVector::new(vec![(1, w), (2, 0.5)]),
            );
            Some(map)
        };
        points.push(Point {
            id: i,
            vector: dense,
            payload: Some(serde_json::json!({ "idx": i })),
            sparse_vectors: sparse,
        });
    }
    col.upsert(points).expect("upsert failed");
    (dir, col)
}

// -----------------------------------------------------------------------
// Sparse-only tests
// -----------------------------------------------------------------------

#[test]
fn test_sparse_only_search() {
    let (_dir, col) = setup_hybrid_collection();

    let sparse_query = SparseVector::new(vec![(1, 1.0), (2, 1.0)]);
    let svs = crate::velesql::SparseVectorSearch {
        vector: crate::velesql::SparseVectorExpr::Literal(sparse_query),
        index_name: None,
    };

    let results = col
        .execute_sparse_search(&svs, &HashMap::new(), None, 5)
        .expect("sparse search failed");

    assert!(!results.is_empty(), "Should find sparse results");
    assert!(results.len() <= 5, "Should respect limit");

    // Results should be ordered by score descending.
    for i in 1..results.len() {
        assert!(
            results[i - 1].score >= results[i].score,
            "Results must be sorted by score descending: {} < {}",
            results[i - 1].score,
            results[i].score
        );
    }
}

// -----------------------------------------------------------------------
// Hybrid dense+sparse with RRF
// -----------------------------------------------------------------------

#[test]
fn test_hybrid_dense_sparse_rrf() {
    let (_dir, col) = setup_hybrid_collection();

    // Dense query: close to point 11 (0.917, 0.5, 0.3, 0.1)
    let dense_query = vec![0.9, 0.5, 0.3, 0.1];

    // Sparse query: strong match for term 1 (points with high term-1 weight win)
    let sparse_query = SparseVector::new(vec![(1, 1.0), (2, 1.0)]);
    let svs = crate::velesql::SparseVectorSearch {
        vector: crate::velesql::SparseVectorExpr::Literal(sparse_query),
        index_name: None,
    };

    let results = col
        .execute_hybrid_search(&dense_query, &svs, &HashMap::new(), None, 10)
        .expect("hybrid search failed");

    assert!(!results.is_empty(), "Hybrid search should return results");

    // Verify fused results contain docs from both dense and sparse hits.
    let result_ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    // Point 11 should be present (has both dense proximity and sparse vector).
    assert!(
        result_ids.contains(&11),
        "Point 11 should appear in hybrid results (dense-close + sparse-hit)"
    );
}

// -----------------------------------------------------------------------
// Hybrid with RSF strategy
// -----------------------------------------------------------------------

#[test]
fn test_hybrid_dense_sparse_rsf() {
    let (_dir, col) = setup_hybrid_collection();

    let dense_query = vec![0.9, 0.5, 0.3, 0.1];
    let sparse_query = SparseVector::new(vec![(1, 1.0), (2, 1.0)]);
    let svs = crate::velesql::SparseVectorSearch {
        vector: crate::velesql::SparseVectorExpr::Literal(sparse_query),
        index_name: None,
    };

    let rsf_strategy = crate::fusion::FusionStrategy::relative_score(0.6, 0.4).unwrap();

    let results = col
        .execute_hybrid_search_with_strategy(
            &dense_query,
            &svs,
            &HashMap::new(),
            None,
            10,
            &rsf_strategy,
        )
        .expect("hybrid RSF search failed");

    assert!(
        !results.is_empty(),
        "RSF hybrid search should return results"
    );

    // Results should be scored and ordered
    for i in 1..results.len() {
        assert!(
            results[i - 1].score >= results[i].score,
            "RSF results should be sorted descending"
        );
    }
}

// -----------------------------------------------------------------------
// Graceful degradation: one branch empty
// -----------------------------------------------------------------------

#[test]
fn test_hybrid_empty_sparse_branch() {
    let (_dir, col) = setup_hybrid_collection();

    let dense_query = vec![0.9, 0.5, 0.3, 0.1];
    // Query on a term that no document has -> empty sparse branch
    let sparse_query = SparseVector::new(vec![(99999, 1.0)]);
    let svs = crate::velesql::SparseVectorSearch {
        vector: crate::velesql::SparseVectorExpr::Literal(sparse_query),
        index_name: None,
    };

    let results = col
        .execute_hybrid_search(&dense_query, &svs, &HashMap::new(), None, 5)
        .expect("hybrid search with empty sparse should succeed");

    // Should gracefully fall back to dense-only results.
    assert!(
        !results.is_empty(),
        "Should return dense results when sparse is empty"
    );
    let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    assert!(
        ids.iter().any(|&id| (6..10).contains(&id)),
        "Dense-fallback must surface dense-only points (ids 6-9), got {ids:?}"
    );
}

// -----------------------------------------------------------------------
// ORDER BY sparse_score on a SPARSE_NEAR query (backlog #20)
// -----------------------------------------------------------------------

/// On a plain `SPARSE_NEAR` query, ranking by the built-in `sparse_score`
/// variable requires the **arithmetic** form (`sparse_score * 1.0`), which
/// resolves the component variable. The setup gives each sparse-bearing point a
/// term-1 weight of `1.0 + id`, so higher ids score strictly higher and the
/// query must come back in descending-score order (id 11 first). This locks the
/// SPEC's documented arithmetic-form behavior.
#[test]
fn test_sparse_near_order_by_sparse_score_arithmetic_desc() {
    let (_dir, col) = setup_hybrid_collection();

    let results = col
        .execute_query_str(
            "SELECT * FROM docs WHERE vector SPARSE_NEAR {1: 1.0} ORDER BY sparse_score * 1.0 DESC LIMIT 6",
            &HashMap::new(),
        )
        .expect("SPARSE_NEAR ORDER BY sparse_score * 1.0 DESC must execute");

    assert!(
        !results.is_empty(),
        "SPARSE_NEAR ORDER BY sparse_score must return sparse-bearing rows"
    );
    // sparse_score DESC => non-increasing scores.
    for w in results.windows(2) {
        assert!(
            w[0].score >= w[1].score - 1e-6,
            "ORDER BY sparse_score * 1.0 DESC must yield non-increasing scores: {} then {}",
            w[0].score,
            w[1].score
        );
    }
    // Highest term-1 weight (id 11, weight 12.0) must rank first.
    assert_eq!(
        results[0].point.id, 11,
        "highest term-1 weight (id 11) must be first under sparse_score * 1.0 DESC"
    );
}

/// A **bare** `ORDER BY sparse_score DESC` ranks by the sparse component score,
/// resolved from the component breakdown exactly like the arithmetic form — it is
/// no longer a silent payload-field no-op.
#[test]
fn test_sparse_near_order_by_bare_sparse_score_sorts_desc() {
    let (_dir, col) = setup_hybrid_collection();

    let results = col
        .execute_query_str(
            "SELECT * FROM docs WHERE vector SPARSE_NEAR {1: 1.0} ORDER BY sparse_score DESC LIMIT 6",
            &HashMap::new(),
        )
        .expect("bare SPARSE_NEAR ORDER BY sparse_score must execute");

    // Sparse-bearing points are ids 0-5 and 10-11; only ids >= 2 carry a term-1
    // hit in the candidate window. The bare ordering now resolves the built-in
    // sparse_score, so rows come back in DESCENDING sparse-score order — identical
    // to the arithmetic form `ORDER BY sparse_score * 1.0 DESC`.
    let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    assert_eq!(
        ids,
        vec![11, 10, 5, 4, 3, 2],
        "bare ORDER BY sparse_score DESC ranks by the sparse component score"
    );
}

/// A LET clause on the same SPARSE_NEAR query is rejected with an explicit
/// error (not silently dropped) — the counterpart to the positive test above.
#[test]
fn test_sparse_near_let_binding_returns_error() {
    let (_dir, col) = setup_hybrid_collection();

    let result = col.execute_query_str(
        "LET s = sparse_score SELECT * FROM docs WHERE vector SPARSE_NEAR {1: 1.0} ORDER BY s DESC LIMIT 5",
        &HashMap::new(),
    );

    assert!(result.is_err(), "LET + SPARSE_NEAR must return an error");
    let msg = result.expect_err("checked is_err").to_string();
    assert!(
        msg.contains("SPARSE_NEAR"),
        "expected SPARSE_NEAR rejection, got: {msg}"
    );
}

// -----------------------------------------------------------------------
// Sparse vector parameter resolution
// -----------------------------------------------------------------------

#[test]
fn test_resolve_sparse_vector_structured() {
    let mut params = HashMap::new();
    params.insert(
        "sv".to_string(),
        serde_json::json!({ "indices": [1, 2, 3], "values": [0.5, 0.3, 0.1] }),
    );

    let expr = crate::velesql::SparseVectorExpr::Parameter("sv".to_string());
    let sv = Collection::resolve_sparse_vector(&expr, &params).expect("resolve failed");
    assert_eq!(sv.indices, vec![1, 2, 3]);
    assert_eq!(sv.values, vec![0.5, 0.3, 0.1]);
}

#[test]
fn test_resolve_sparse_vector_shorthand() {
    let mut params = HashMap::new();
    params.insert(
        "sv".to_string(),
        serde_json::json!({ "10": 0.8, "20": 0.3 }),
    );

    let expr = crate::velesql::SparseVectorExpr::Parameter("sv".to_string());
    let sv = Collection::resolve_sparse_vector(&expr, &params).expect("resolve failed");
    assert_eq!(sv.nnz(), 2);
    // Values should be present (order from BTreeMap is sorted by string key)
    assert!(sv.indices.contains(&10));
    assert!(sv.indices.contains(&20));
    assert!(
        sv.values.iter().any(|&v| (v - 0.8_f32).abs() < 1e-5),
        "weight 0.8 for index 10 must survive shorthand parse"
    );
    assert!(
        sv.values.iter().any(|&v| (v - 0.3_f32).abs() < 1e-5),
        "weight 0.3 for index 20 must survive shorthand parse"
    );
}

#[test]
fn test_resolve_sparse_vector_missing_param() {
    let params = HashMap::new();
    let expr = crate::velesql::SparseVectorExpr::Parameter("missing".to_string());
    let result = Collection::resolve_sparse_vector(&expr, &params);
    assert!(result.is_err());
}
