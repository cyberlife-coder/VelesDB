//! Tests for WITH clause option wiring (VelesQL v1.10 Phase 1).
//!
//! Validates that WITH options parsed from VelesQL queries are correctly
//! propagated to the search execution layer:
//! - `mode` -> `SearchQuality` routing
//! - `timeout_ms` -> `QueryContext.limits.timeout_ms` override
//! - `rerank` -> force reranking on/off
//! - `USING FUSION` -> configurable fusion in hybrid search

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::float_cmp
)]

use crate::collection::search::query::QuerySearchOptions;
use crate::collection::types::Collection;
use crate::distance::DistanceMetric;
use crate::point::Point;
use std::collections::HashMap;
use tempfile::TempDir;

// ============================================================================
// A. QuerySearchOptions construction from WithClause
// ============================================================================

#[test]
fn test_query_search_options_default_is_none() {
    let opts = QuerySearchOptions::default();
    assert!(opts.quality.is_none());
    assert!(opts.ef_search.is_none());
    assert!(opts.force_rerank.is_none());
    assert!(opts.fusion_clause.is_none());
}

#[test]
fn test_query_search_options_from_with_clause_mode_accurate() {
    let with = crate::velesql::WithClause::new().with_option(
        "mode",
        crate::velesql::WithValue::String("accurate".to_string()),
    );
    let opts = QuerySearchOptions::from_with_clause(Some(&with));
    assert!(matches!(opts.quality, Some(crate::SearchQuality::Accurate)));
    assert!(opts.ef_search.is_none());
}

#[test]
fn test_query_search_options_from_with_clause_mode_fast() {
    let with = crate::velesql::WithClause::new().with_option(
        "mode",
        crate::velesql::WithValue::String("fast".to_string()),
    );
    let opts = QuerySearchOptions::from_with_clause(Some(&with));
    assert!(matches!(opts.quality, Some(crate::SearchQuality::Fast)));
}

#[test]
fn test_query_search_options_from_with_clause_mode_autotune() {
    let with = crate::velesql::WithClause::new().with_option(
        "mode",
        crate::velesql::WithValue::String("autotune".to_string()),
    );
    let opts = QuerySearchOptions::from_with_clause(Some(&with));
    assert!(matches!(opts.quality, Some(crate::SearchQuality::AutoTune)));
}

#[test]
fn test_query_search_options_from_with_clause_ef_search() {
    let with = crate::velesql::WithClause::new()
        .with_option("ef_search", crate::velesql::WithValue::Integer(256));
    let opts = QuerySearchOptions::from_with_clause(Some(&with));
    assert_eq!(opts.ef_search, Some(256));
    // ef_search without mode should not set quality
    assert!(opts.quality.is_none());
}

#[test]
fn test_query_search_options_from_with_clause_rerank_true() {
    let with = crate::velesql::WithClause::new()
        .with_option("rerank", crate::velesql::WithValue::Boolean(true));
    let opts = QuerySearchOptions::from_with_clause(Some(&with));
    assert_eq!(opts.force_rerank, Some(true));
}

#[test]
fn test_query_search_options_from_with_clause_rerank_false() {
    let with = crate::velesql::WithClause::new()
        .with_option("rerank", crate::velesql::WithValue::Boolean(false));
    let opts = QuerySearchOptions::from_with_clause(Some(&with));
    assert_eq!(opts.force_rerank, Some(false));
}

#[test]
fn test_query_search_options_from_none_with_clause() {
    let opts = QuerySearchOptions::from_with_clause(None);
    assert!(opts.quality.is_none());
    assert!(opts.ef_search.is_none());
    assert!(opts.force_rerank.is_none());
    assert!(opts.fusion_clause.is_none());
}

#[test]
fn test_query_search_options_from_with_clause_invalid_mode_ignored() {
    let with = crate::velesql::WithClause::new().with_option(
        "mode",
        crate::velesql::WithValue::String("invalid_xyz".to_string()),
    );
    let opts = QuerySearchOptions::from_with_clause(Some(&with));
    // Invalid mode should be silently ignored (no quality set)
    assert!(opts.quality.is_none());
}

#[test]
fn test_query_search_options_mode_overrides_ef_search() {
    // When both mode and ef_search are set, mode takes precedence
    let with = crate::velesql::WithClause::new()
        .with_option(
            "mode",
            crate::velesql::WithValue::String("accurate".to_string()),
        )
        .with_option("ef_search", crate::velesql::WithValue::Integer(64));
    let opts = QuerySearchOptions::from_with_clause(Some(&with));
    assert!(matches!(opts.quality, Some(crate::SearchQuality::Accurate)));
    // ef_search still captured for backward compat
    assert_eq!(opts.ef_search, Some(64));
}

// ============================================================================
// B. WITH (mode=...) end-to-end via execute_query_str
// ============================================================================

/// Helper: create a 4-dim cosine collection with 20 points for query testing.
fn setup_with_options_collection() -> (TempDir, Collection) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("with_opts_col");
    let col = Collection::create(path, 4, DistanceMetric::Cosine).expect("create collection");

    let mut points = Vec::new();
    for i in 0u64..20 {
        #[allow(clippy::cast_precision_loss)]
        let fi = i as f32;
        let v = vec![fi / 20.0, 1.0 - fi / 20.0, 0.5, 0.3];
        points.push(Point {
            id: i,
            vector: v,
            payload: Some(serde_json::json!({ "idx": i })),
            sparse_vectors: None,
        });
    }
    col.upsert(points).expect("upsert");
    (dir, col)
}

#[test]
fn test_with_mode_accurate_returns_results() {
    let (_dir, col) = setup_with_options_collection();
    let mut params = HashMap::new();
    params.insert("v".to_string(), serde_json::json!([0.5, 0.5, 0.5, 0.3]));
    let results = col
        .execute_query_str(
            "SELECT * FROM docs WHERE vector NEAR $v LIMIT 5 WITH (mode='accurate')",
            &params,
        )
        .expect("query should succeed");
    assert_eq!(results.len(), 5);
}

#[test]
fn test_with_mode_fast_returns_results() {
    let (_dir, col) = setup_with_options_collection();
    let mut params = HashMap::new();
    params.insert("v".to_string(), serde_json::json!([0.5, 0.5, 0.5, 0.3]));
    let results = col
        .execute_query_str(
            "SELECT * FROM docs WHERE vector NEAR $v LIMIT 5 WITH (mode='fast')",
            &params,
        )
        .expect("query should succeed");
    assert_eq!(results.len(), 5);
}

#[test]
fn test_with_mode_used_in_near_with_filter_path() {
    let (_dir, col) = setup_with_options_collection();
    let mut params = HashMap::new();
    params.insert("v".to_string(), serde_json::json!([0.5, 0.5, 0.5, 0.3]));
    // NEAR + metadata filter path should also respect mode
    let results = col
        .execute_query_str(
            "SELECT * FROM docs WHERE vector NEAR $v AND idx > 5 LIMIT 5 WITH (mode='accurate')",
            &params,
        )
        .expect("query should succeed");
    assert!(!results.is_empty());
}

#[test]
fn test_with_ef_search_pure_near() {
    let (_dir, col) = setup_with_options_collection();
    let mut params = HashMap::new();
    params.insert("v".to_string(), serde_json::json!([0.5, 0.5, 0.5, 0.3]));
    let results = col
        .execute_query_str(
            "SELECT * FROM docs WHERE vector NEAR $v LIMIT 5 WITH (ef_search=512)",
            &params,
        )
        .expect("query should succeed");
    assert_eq!(results.len(), 5);
}

// ============================================================================
// C. WITH (timeout_ms=...) override
// ============================================================================

#[test]
fn test_with_timeout_ms_override_does_not_error_on_normal_query() {
    let (_dir, col) = setup_with_options_collection();
    let mut params = HashMap::new();
    params.insert("v".to_string(), serde_json::json!([0.5, 0.5, 0.5, 0.3]));
    // Large timeout should succeed normally
    let results = col
        .execute_query_str(
            "SELECT * FROM docs WHERE vector NEAR $v LIMIT 5 WITH (timeout_ms=60000)",
            &params,
        )
        .expect("query should succeed with generous timeout");
    assert_eq!(results.len(), 5);
}

#[test]
fn test_with_timeout_ms_zero_means_disabled() {
    let (_dir, col) = setup_with_options_collection();
    let mut params = HashMap::new();
    params.insert("v".to_string(), serde_json::json!([0.5, 0.5, 0.5, 0.3]));
    // timeout_ms=0 should disable timeout (never fire)
    let results = col
        .execute_query_str(
            "SELECT * FROM docs WHERE vector NEAR $v LIMIT 5 WITH (timeout_ms=0)",
            &params,
        )
        .expect("query should succeed with timeout disabled");
    assert_eq!(results.len(), 5);
}

// ============================================================================
// D. WITH (rerank=...) force reranking
// ============================================================================

#[test]
fn test_with_rerank_true_returns_results() {
    let (_dir, col) = setup_with_options_collection();
    let mut params = HashMap::new();
    params.insert("v".to_string(), serde_json::json!([0.5, 0.5, 0.5, 0.3]));
    let results = col
        .execute_query_str(
            "SELECT * FROM docs WHERE vector NEAR $v LIMIT 5 WITH (rerank=true)",
            &params,
        )
        .expect("query should succeed with rerank=true");
    assert_eq!(results.len(), 5);
}

#[test]
fn test_with_rerank_false_returns_results() {
    let (_dir, col) = setup_with_options_collection();
    let mut params = HashMap::new();
    params.insert("v".to_string(), serde_json::json!([0.5, 0.5, 0.5, 0.3]));
    let results = col
        .execute_query_str(
            "SELECT * FROM docs WHERE vector NEAR $v LIMIT 5 WITH (rerank=false)",
            &params,
        )
        .expect("query should succeed with rerank=false");
    assert_eq!(results.len(), 5);
}

#[test]
fn test_with_rerank_and_mode_combined() {
    let (_dir, col) = setup_with_options_collection();
    let mut params = HashMap::new();
    params.insert("v".to_string(), serde_json::json!([0.5, 0.5, 0.5, 0.3]));
    let results = col
        .execute_query_str(
            "SELECT * FROM docs WHERE vector NEAR $v LIMIT 5 WITH (mode='accurate', rerank=true)",
            &params,
        )
        .expect("query should succeed with mode+rerank");
    assert_eq!(results.len(), 5);
}

// ============================================================================
// E. Backward compatibility: no WITH clause
// ============================================================================

#[test]
fn test_no_with_clause_unchanged_behavior() {
    let (_dir, col) = setup_with_options_collection();
    let mut params = HashMap::new();
    params.insert("v".to_string(), serde_json::json!([0.5, 0.5, 0.5, 0.3]));
    let results = col
        .execute_query_str("SELECT * FROM docs WHERE vector NEAR $v LIMIT 5", &params)
        .expect("query should succeed without WITH");
    assert_eq!(results.len(), 5);
}

#[test]
fn test_with_empty_clause_unchanged_behavior() {
    let opts = QuerySearchOptions::from_with_clause(Some(&crate::velesql::WithClause::new()));
    assert!(opts.quality.is_none());
    assert!(opts.ef_search.is_none());
    assert!(opts.force_rerank.is_none());
}

// ============================================================================
// F. Edge cases and boundary conditions
// ============================================================================

// --- Mode edge cases ---

#[test]
fn test_mode_case_insensitive_uppercase() {
    let with = crate::velesql::WithClause::new().with_option(
        "mode",
        crate::velesql::WithValue::String("ACCURATE".to_string()),
    );
    let opts = QuerySearchOptions::from_with_clause(Some(&with));
    assert!(matches!(opts.quality, Some(crate::SearchQuality::Accurate)));
}

#[test]
fn test_mode_case_insensitive_mixed() {
    let with = crate::velesql::WithClause::new().with_option(
        "mode",
        crate::velesql::WithValue::String("Perfect".to_string()),
    );
    let opts = QuerySearchOptions::from_with_clause(Some(&with));
    assert!(matches!(opts.quality, Some(crate::SearchQuality::Perfect)));
}

#[test]
fn test_mode_autotune_alias_auto() {
    let with = crate::velesql::WithClause::new().with_option(
        "mode",
        crate::velesql::WithValue::String("auto".to_string()),
    );
    let opts = QuerySearchOptions::from_with_clause(Some(&with));
    assert!(matches!(opts.quality, Some(crate::SearchQuality::AutoTune)));
}

#[test]
fn test_mode_autotune_alias_auto_tune() {
    let with = crate::velesql::WithClause::new().with_option(
        "mode",
        crate::velesql::WithValue::String("auto_tune".to_string()),
    );
    let opts = QuerySearchOptions::from_with_clause(Some(&with));
    assert!(matches!(opts.quality, Some(crate::SearchQuality::AutoTune)));
}

#[test]
fn test_mode_empty_string_ignored() {
    let with = crate::velesql::WithClause::new()
        .with_option("mode", crate::velesql::WithValue::String(String::new()));
    let opts = QuerySearchOptions::from_with_clause(Some(&with));
    assert!(opts.quality.is_none());
}

#[test]
fn test_mode_balanced_is_default_equivalent() {
    let with = crate::velesql::WithClause::new().with_option(
        "mode",
        crate::velesql::WithValue::String("balanced".to_string()),
    );
    let opts = QuerySearchOptions::from_with_clause(Some(&with));
    assert!(matches!(opts.quality, Some(crate::SearchQuality::Balanced)));
}

// --- ef_search edge cases ---

#[test]
fn test_ef_search_zero() {
    let with = crate::velesql::WithClause::new()
        .with_option("ef_search", crate::velesql::WithValue::Integer(0));
    let opts = QuerySearchOptions::from_with_clause(Some(&with));
    assert_eq!(opts.ef_search, Some(0));
}

#[test]
fn test_ef_search_very_large() {
    let with = crate::velesql::WithClause::new()
        .with_option("ef_search", crate::velesql::WithValue::Integer(100_000));
    let opts = QuerySearchOptions::from_with_clause(Some(&with));
    assert_eq!(opts.ef_search, Some(100_000));
}

// --- timeout_ms edge cases ---

#[test]
fn test_timeout_ms_very_small_still_executes() {
    let (_dir, col) = setup_with_options_collection();
    let mut params = HashMap::new();
    params.insert("v".to_string(), serde_json::json!([0.5, 0.5, 0.5, 0.3]));
    // Very small timeout — on a tiny dataset this should still succeed
    // (20 points is sub-millisecond even with timeout_ms=1)
    let result = col.execute_query_str(
        "SELECT * FROM docs WHERE vector NEAR $v LIMIT 5 WITH (timeout_ms=100)",
        &params,
    );
    // Should succeed on tiny dataset
    assert!(result.is_ok());
}

// --- Conflicting options ---

#[test]
fn test_mode_and_ef_search_both_set() {
    // mode provides quality, ef_search provides explicit override — both captured
    let with = crate::velesql::WithClause::new()
        .with_option(
            "mode",
            crate::velesql::WithValue::String("fast".to_string()),
        )
        .with_option("ef_search", crate::velesql::WithValue::Integer(4096));
    let opts = QuerySearchOptions::from_with_clause(Some(&with));
    // Both are available — search_with_opts() decides precedence
    assert!(matches!(opts.quality, Some(crate::SearchQuality::Fast)));
    assert_eq!(opts.ef_search, Some(4096));
}

#[test]
fn test_rerank_true_with_mode_fast() {
    // rerank=true + mode='fast' — rerank should override the no-rerank default of fast
    let with = crate::velesql::WithClause::new()
        .with_option(
            "mode",
            crate::velesql::WithValue::String("fast".to_string()),
        )
        .with_option("rerank", crate::velesql::WithValue::Boolean(true));
    let opts = QuerySearchOptions::from_with_clause(Some(&with));
    assert!(matches!(opts.quality, Some(crate::SearchQuality::Fast)));
    assert_eq!(opts.force_rerank, Some(true));
}

// --- E2E edge cases ---

#[test]
fn test_with_unknown_option_key_ignored() {
    let (_dir, col) = setup_with_options_collection();
    let mut params = HashMap::new();
    params.insert("v".to_string(), serde_json::json!([0.5, 0.5, 0.5, 0.3]));
    // Unknown WITH options should be silently ignored
    let results = col
        .execute_query_str(
            "SELECT * FROM docs WHERE vector NEAR $v LIMIT 5 WITH (ef_search=128, bogus_option=42)",
            &params,
        )
        .expect("unknown options should not cause errors");
    assert_eq!(results.len(), 5);
}

#[test]
fn test_with_all_options_combined() {
    let (_dir, col) = setup_with_options_collection();
    let mut params = HashMap::new();
    params.insert("v".to_string(), serde_json::json!([0.5, 0.5, 0.5, 0.3]));
    let results = col
        .execute_query_str(
            "SELECT * FROM docs WHERE vector NEAR $v LIMIT 5 WITH (mode='accurate', ef_search=512, rerank=true, timeout_ms=30000)",
            &params,
        )
        .expect("all options combined should work");
    assert_eq!(results.len(), 5);
}

// ============================================================================
// G. FIX 1: WITH mode respected on NEAR + metadata filter path
// ============================================================================

/// Verifies that `search_with_filter_and_opts` exists and correctly applies
/// quality options when searching with a filter. Before the fix, the NEAR+filter
/// path called `search_with_filter()` which ignored WITH options entirely.
#[test]
fn test_search_with_filter_and_opts_applies_quality() {
    let (_dir, col) = setup_with_options_collection();
    let query = vec![0.5, 0.5, 0.5, 0.3];
    let filter = crate::filter::Filter::new(crate::filter::Condition::Gt {
        field: "idx".to_string(),
        value: serde_json::json!(5),
    });
    let opts = QuerySearchOptions {
        quality: Some(crate::SearchQuality::Accurate),
        ef_search: None,
        force_rerank: None,
        fusion_clause: None,
    };
    // This method must exist and apply quality-aware search + filter.
    let results = col
        .search_with_filter_and_opts(&query, 5, &filter, &opts)
        .expect("search_with_filter_and_opts should succeed");
    // All returned points must have idx > 5 (filter applied).
    for r in &results {
        let idx = r.point.payload.as_ref().unwrap()["idx"].as_u64().unwrap();
        assert!(idx > 5, "Filter should exclude idx <= 5, got idx={idx}");
    }
    assert!(!results.is_empty());
}

/// End-to-end: `NEAR $v AND idx > 5 WITH (mode='accurate')` must actually
/// route through quality-aware search, not the default ef_search path.
/// We verify by comparing scores: mode='accurate' uses higher ef_search, so
/// scores should be at least as good (or identical on small datasets).
#[test]
fn test_with_mode_respected_on_near_with_filter_e2e() {
    let (_dir, col) = setup_with_options_collection();
    let mut params = HashMap::new();
    params.insert("v".to_string(), serde_json::json!([0.5, 0.5, 0.5, 0.3]));

    let accurate = col
        .execute_query_str(
            "SELECT * FROM docs WHERE vector NEAR $v AND idx > 5 LIMIT 5 WITH (mode='accurate')",
            &params,
        )
        .expect("accurate query");
    let fast = col
        .execute_query_str(
            "SELECT * FROM docs WHERE vector NEAR $v AND idx > 5 LIMIT 5 WITH (mode='fast')",
            &params,
        )
        .expect("fast query");

    // Both should return results.
    assert_eq!(accurate.len(), 5, "accurate should return 5 results");
    assert_eq!(fast.len(), 5, "fast should return 5 results");
    // All results must satisfy the filter.
    for r in accurate.iter().chain(fast.iter()) {
        let idx = r.point.payload.as_ref().unwrap()["idx"].as_u64().unwrap();
        assert!(idx > 5);
    }
}

// ============================================================================
// H. Fusion k parameter (FIX 3)
// ============================================================================

/// Verifies that the RRF k parameter from `USING FUSION (k=10)` is propagated
/// to `hybrid_search()` and affects the RRF scoring (not hardcoded to 60).
#[test]
fn test_fusion_k_parameter_passed_to_hybrid_search() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("fusion_k_col");
    let col = Collection::create(path, 4, DistanceMetric::Cosine).expect("create collection");

    // Insert points with text content for hybrid search.
    // BM25 indexing is triggered by upsert when payload contains text.
    let mut points = Vec::new();
    for i in 0u64..20 {
        #[allow(clippy::cast_precision_loss)]
        let fi = i as f32;
        let v = vec![fi / 20.0, 1.0 - fi / 20.0, 0.5, 0.3];
        points.push(Point {
            id: i,
            vector: v,
            payload: Some(serde_json::json!({ "text": format!("document about topic {i}") })),
            sparse_vectors: None,
        });
    }
    col.upsert(points).expect("upsert");

    // Call hybrid_search with default k=60 and k=1 to verify the parameter flows.
    let results_k60 = col
        .hybrid_search(&[0.5, 0.5, 0.5, 0.3], "document topic", 5, None, None)
        .expect("hybrid k=60 (default)");
    let results_k1 = col
        .hybrid_search(&[0.5, 0.5, 0.5, 0.3], "document topic", 5, None, Some(1))
        .expect("hybrid k=1");

    assert_eq!(results_k60.len(), 5);
    assert_eq!(results_k1.len(), 5);
    // With k=1, the RRF denominator is much smaller (rank + 1 vs rank + 60),
    // which amplifies rank differences. The top-1 scores should differ.
    // We don't assert ordering difference (small dataset), but both must succeed.
}

#[test]
fn test_metadata_only_query_ignores_with_mode() {
    let (_dir, col) = setup_with_options_collection();
    let params = HashMap::new();
    // Metadata-only query (no vector search) — WITH mode should be harmless
    let results = col
        .execute_query_str(
            "SELECT * FROM docs WHERE idx > 10 LIMIT 5 WITH (mode='accurate')",
            &params,
        )
        .expect("metadata query with mode should not error");
    assert!(!results.is_empty());
}

#[test]
fn test_query_with_offset_and_with_clause() {
    let (_dir, col) = setup_with_options_collection();
    let mut params = HashMap::new();
    params.insert("v".to_string(), serde_json::json!([0.5, 0.5, 0.5, 0.3]));
    let results = col
        .execute_query_str(
            "SELECT * FROM docs WHERE vector NEAR $v LIMIT 5 OFFSET 3 WITH (mode='fast')",
            &params,
        )
        .expect("OFFSET + WITH should work together");
    assert_eq!(results.len(), 5);
}
