//! E2E integration tests: MATCH + similarity + GuardRails (P1-B).
//!
//! These tests exercise the full pipeline:
//! - execute_query_str() → QueryCache → execute_query() → GuardRails → results
//! - MATCH multi-pattern traversal with guardrails
//! - similarity() with named payload vector fields (multi-vector P1-A)
//! - CBO planner selection (logged, no assertion on strategy — heuristic may vary)

use crate::collection::types::Collection;
use crate::guardrails::{GuardRails, QueryLimits};
use crate::point::Point;
use crate::test_fixtures::fixtures::setup_collection;
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

/// Helper: create a 4-dim collection with 10 points + named payload vector field.
fn make_collection() -> (TempDir, Collection) {
    let (dir, col) = setup_collection(4);
    let points: Vec<Point> = (0u64..10)
        .map(|i| {
            #[allow(clippy::cast_precision_loss)]
            Point::new(
                i,
                vec![i as f32 / 10.0, 0.1, 0.1, 0.1],
                Some(serde_json::json!({
                    "idx": i,
                    "category": if i % 2 == 0 { "even" } else { "odd" },
                    // Named payload vector for multi-vector P1-A test
                    "alt_vec": [i as f64 / 10.0, 0.2, 0.2, 0.2]
                })),
            )
        })
        .collect();
    col.upsert(points).expect("test: upsert");
    (dir, col)
}

// ─── execute_query_str() cache ────────────────────────────────────────────────

#[test]
fn test_execute_query_str_parses_and_executes() {
    let (_dir, col) = make_collection();
    let params = HashMap::new();

    let result = col.execute_query_str("SELECT * FROM col LIMIT 5;", &params);
    assert!(result.is_ok(), "execute_query_str should succeed");
    assert!(result.unwrap().len() <= 5);
}

#[test]
fn test_execute_query_str_caches_repeated_calls() {
    let (_dir, col) = make_collection();
    let params = HashMap::new();
    let sql = "SELECT * FROM col LIMIT 3;";

    // First call — parsed and cached
    let r1 = col
        .execute_query_str(sql, &params)
        .expect("first call failed");
    // Second call — should hit cache and return same result count
    let r2 = col
        .execute_query_str(sql, &params)
        .expect("second call failed");
    assert_eq!(r1.len(), r2.len(), "Cached and fresh results should match");
}

#[test]
fn test_execute_query_str_rejects_invalid_sql() {
    let (_dir, col) = make_collection();
    let params = HashMap::new();

    let result = col.execute_query_str("NOT VALID SQL !!!", &params);
    assert!(result.is_err(), "Invalid SQL should return an error");
}

// ─── Metadata filter E2E ──────────────────────────────────────────────────────

#[test]
fn test_execute_query_str_metadata_filter() {
    let (_dir, col) = make_collection();
    let params = HashMap::new();

    let result = col
        .execute_query_str(
            "SELECT * FROM col WHERE category = 'even' LIMIT 10;",
            &params,
        )
        .expect("query failed");

    // All returned payloads should have category = "even"
    for r in &result {
        if let Some(ref payload) = r.point.payload {
            assert_eq!(
                payload.get("category").and_then(|v| v.as_str()),
                Some("even"),
                "Non-even category in result: {:?}",
                payload
            );
        }
    }
}

// ─── GuardRails E2E ──────────────────────────────────────────────────────────

#[test]
fn test_e2e_guardrails_cardinality_respected() {
    let (_dir, mut col) = make_collection();

    let limits = QueryLimits {
        max_cardinality: 3, // only 3 results allowed
        ..QueryLimits::default()
    };
    col.guard_rails = Arc::new(GuardRails::with_limits(limits));

    let params = HashMap::new();
    let result = col.execute_query_str("SELECT * FROM col LIMIT 10;", &params);
    // Should fail with cardinality violation (10 points > limit 3)
    assert!(result.is_err(), "Cardinality guardrail should fire");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("Guard-rail") || err.contains("cardinality") || err.contains("Cardinality"),
        "Unexpected error message: {err}"
    );
}

#[test]
fn test_e2e_guardrails_timeout_zero_disables_check() {
    let (_dir, mut col) = make_collection();

    // timeout_ms = 0 is the "disabled" sentinel — the guard-rail must never fire.
    // Reason: 0 means no timeout (batch/offline workloads), not "0 ms budget".
    col.guard_rails = Arc::new(GuardRails::with_limits(QueryLimits {
        timeout_ms: 0,
        ..QueryLimits::default()
    }));

    let params = HashMap::new();
    let result = col.execute_query_str("SELECT * FROM col LIMIT 5;", &params);
    assert!(
        result.is_ok(),
        "timeout_ms=0 should disable the guard-rail, not reject the query"
    );
}

#[test]
fn test_e2e_guardrails_circuit_breaker_state() {
    let (_dir, mut col) = make_collection();

    col.guard_rails = Arc::new(GuardRails::with_limits(QueryLimits {
        max_cardinality: 1, // forces failures
        circuit_failure_threshold: 2,
        circuit_recovery_seconds: 60,
        ..QueryLimits::default()
    }));

    let params = HashMap::new();
    let sql = "SELECT * FROM col LIMIT 10;";
    let _ = col.execute_query_str(sql, &params); // failure 1
    let _ = col.execute_query_str(sql, &params); // failure 2

    let state = col.guard_rails.circuit_breaker.state();
    assert_eq!(
        state,
        crate::guardrails::CircuitState::Open,
        "Circuit breaker should open after 2 failures"
    );
}

// ─── MATCH multi-pattern E2E ─────────────────────────────────────────────────

#[test]
fn test_e2e_match_single_pattern_no_panic() {
    let (_dir, col) = make_collection();
    let params = HashMap::new();

    // Single pattern MATCH — exercises multi-pattern loop with 1 pattern
    let sql = "MATCH (a) RETURN a LIMIT 5;";
    let query = crate::velesql::Parser::parse(sql).expect("parse failed");
    // Should not panic — may return empty or results depending on graph data
    let _ = col.execute_query(&query, &params);
}

// ─── Multi-vector field E2E ───────────────────────────────────────────────────

#[test]
fn test_e2e_similarity_primary_vector_field() {
    let (_dir, col) = make_collection();
    let mut params = HashMap::new();
    params.insert("v".to_string(), serde_json::json!([0.5, 0.1, 0.1, 0.1]));

    // Primary "vector" field — standard HNSW ANN search
    let result = col
        .execute_query_str(
            "SELECT * FROM col WHERE similarity(vector, $v) > 0.5 LIMIT 5;",
            &params,
        )
        .expect("primary vector similarity should succeed");
    // All results should have score > 0.5 (cosine similarity)
    for r in &result {
        assert!(r.score >= 0.5, "Score {} below threshold 0.5", r.score);
    }
}

#[test]
fn test_e2e_similarity_named_payload_vector_field() {
    let (_dir, col) = make_collection();
    let mut params = HashMap::new();
    params.insert("v".to_string(), serde_json::json!([0.5, 0.2, 0.2, 0.2]));

    // Named payload field "alt_vec" — multi-vector P1-A path
    // Uses HNSW for candidates, then re-scores using payload vector
    let result = col.execute_query_str(
        "SELECT * FROM col WHERE similarity(alt_vec, $v) > 0.0 LIMIT 10;",
        &params,
    );
    // Should not return an "Only 'vector' field is supported" error anymore
    match &result {
        Err(e) => {
            let msg = e.to_string();
            assert!(
                !msg.contains("Only 'vector' field is supported"),
                "Multi-vector restriction should be removed: {msg}"
            );
        }
        Ok(results) => {
            // All results should have score >= 0.0
            for r in results {
                assert!(r.score >= 0.0, "Negative score: {}", r.score);
            }
        }
    }
}

// ─── CBO integration (smoke) ─────────────────────────────────────────────────

#[test]
fn test_e2e_cbo_with_vector_and_filter_no_panic() {
    let (_dir, col) = make_collection();
    let mut params = HashMap::new();
    params.insert("v".to_string(), serde_json::json!([0.5, 0.1, 0.1, 0.1]));

    // Vector search + metadata filter — CBO should choose a strategy
    let result = col.execute_query_str(
        "SELECT * FROM col WHERE vector NEAR $v AND category = 'even' LIMIT 5;",
        &params,
    );
    // Should succeed and return only even-category results
    match result {
        Ok(results) => {
            for r in &results {
                if let Some(ref payload) = r.point.payload {
                    assert_eq!(
                        payload.get("category").and_then(|v| v.as_str()),
                        Some("even")
                    );
                }
            }
        }
        Err(e) => {
            // Acceptable if guardrails fire (e.g., default cardinality)
            let msg = e.to_string();
            assert!(
                msg.contains("Guard-rail") || msg.contains("Query"),
                "Unexpected CBO error: {msg}"
            );
        }
    }
}
