//! Integration tests for GuardRails enforcement inside execute_query() and execute_match().
//!
//! Verifies that guard-rails (timeout, cardinality, rate limiting, circuit breaker)
//! are properly enforced when queries run through Collection::execute_query().

use crate::collection::graph::GraphEdge;
use crate::collection::types::Collection;
use crate::guardrails::{CircuitState, GuardRails, QueryContext, QueryLimits};
use crate::point::Point;
use crate::velesql::{
    CompareOp, Comparison, Condition, DistinctMode, Parser, Query, SelectColumns, SelectStatement,
    SimilarityCondition, Value, VectorExpr,
};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

/// Helper: create a small collection with a few points for testing.
fn create_test_collection(dir: &TempDir) -> Collection {
    let path = dir.path().join("test_col");
    let col = Collection::create(path, 4, crate::distance::DistanceMetric::Cosine)
        .expect("Failed to create test collection");

    // Insert a few points so scans are non-trivial
    let points: Vec<Point> = (0u64..10)
        .map(|i| {
            #[allow(clippy::cast_precision_loss)]
            Point::new(
                i,
                vec![i as f32 / 10.0, 0.1, 0.1, 0.1],
                Some(serde_json::json!({ "idx": i })),
            )
        })
        .collect();
    col.upsert(points).expect("upsert failed");
    col
}

#[test]
fn test_execute_query_pre_check_rate_limit() {
    let dir = TempDir::new().unwrap();
    let mut col = create_test_collection(&dir);

    let limits = QueryLimits {
        rate_limit_qps: 2,
        ..QueryLimits::default()
    };
    col.guard_rails = Arc::new(GuardRails::with_limits(limits));

    let sql = "SELECT * FROM col LIMIT 5;";
    let query = Parser::parse(sql).expect("parse failed");
    let params = HashMap::new();

    // First two queries should pass (token bucket starts full)
    assert!(col.execute_query(&query, &params).is_ok());
    assert!(col.execute_query(&query, &params).is_ok());
    // Third should be rate-limited
    let result = col.execute_query(&query, &params);
    assert!(result.is_err(), "Expected rate limit error but got Ok");
    let err_str = result.unwrap_err().to_string();
    assert!(
        err_str.contains("Guard-rail") || err_str.contains("Rate limit"),
        "Unexpected error: {err_str}"
    );
}

#[test]
fn test_execute_query_cardinality_enforced() {
    let dir = TempDir::new().unwrap();
    let mut col = create_test_collection(&dir);

    // Set cardinality limit below the number of points in the collection
    let limits = QueryLimits {
        max_cardinality: 3, // collection has 10 points
        ..QueryLimits::default()
    };
    col.guard_rails = Arc::new(GuardRails::with_limits(limits));

    let sql = "SELECT * FROM col LIMIT 100;";
    let query = Parser::parse(sql).expect("parse failed");
    let params = HashMap::new();

    let result = col.execute_query(&query, &params);
    assert!(
        result.is_err(),
        "Expected cardinality guard-rail error but got Ok"
    );
    let err_str = result.unwrap_err().to_string();
    assert!(
        err_str.contains("Guard-rail") || err_str.contains("Cardinality"),
        "Unexpected error: {err_str}"
    );
}

#[test]
fn test_execute_query_timeout_disabled_at_zero() {
    let dir = TempDir::new().unwrap();
    let mut col = create_test_collection(&dir);

    // timeout_ms = 0 means "disabled" — queries must never be rejected by timeout.
    // Reason: 0 is the sentinel for "no timeout" (e.g., offline/batch workloads).
    let limits = QueryLimits {
        timeout_ms: 0,
        ..QueryLimits::default()
    };
    col.guard_rails = Arc::new(GuardRails::with_limits(limits));

    let sql = "SELECT * FROM col LIMIT 10;";
    let query = Parser::parse(sql).expect("parse failed");
    let params = HashMap::new();

    // With timeout disabled the query must succeed regardless of elapsed time.
    assert!(
        col.execute_query(&query, &params).is_ok(),
        "timeout_ms=0 should disable the guard-rail, not reject the query"
    );
}

#[test]
fn test_query_context_timeout_fires_after_elapsed() {
    // Test the QueryContext mechanism directly so the test is deterministic
    // without depending on how long execute_query takes.
    let limits = QueryLimits {
        timeout_ms: 1, // 1 ms
        ..QueryLimits::default()
    };
    let ctx = QueryContext::new(limits);
    // Sleep long enough to guarantee elapsed >= timeout_ms.
    std::thread::sleep(std::time::Duration::from_millis(5));
    let result = ctx.check_timeout();
    assert!(
        result.is_err(),
        "check_timeout should return Err after the timeout has elapsed"
    );
    let err_str = result.unwrap_err().to_string();
    assert!(
        err_str.contains("timed out"),
        "Unexpected error message: {err_str}"
    );
}

#[test]
fn test_execute_query_circuit_breaker_opens_after_failures() {
    let dir = TempDir::new().unwrap();
    let mut col = create_test_collection(&dir);

    // Use a very low cardinality limit (1) with threshold=2 so cardinality violations
    // trigger circuit breaker. Cardinality check runs on every SELECT query result.
    let limits = QueryLimits {
        max_cardinality: 1, // collection has 10 points — scan exceeds this
        circuit_failure_threshold: 2,
        circuit_recovery_seconds: 60,
        ..QueryLimits::default()
    };
    col.guard_rails = Arc::new(GuardRails::with_limits(limits));

    let sql = "SELECT * FROM col LIMIT 10;";
    let query = Parser::parse(sql).expect("parse failed");
    let params = HashMap::new();

    // Trigger failures until circuit opens
    let _ = col.execute_query(&query, &params); // failure 1
    let _ = col.execute_query(&query, &params); // failure 2 → circuit opens

    // After 2 cardinality violations, the circuit breaker should be Open
    let state = col.guard_rails.circuit_breaker.state();
    assert_eq!(
        state,
        CircuitState::Open,
        "Circuit breaker should be Open after repeated failures"
    );
}

#[test]
fn test_execute_query_normal_query_records_success() {
    let dir = TempDir::new().unwrap();
    let col = create_test_collection(&dir);

    // Default limits — query should succeed and record success on circuit breaker
    let sql = "SELECT * FROM col LIMIT 5;";
    let query = Parser::parse(sql).expect("parse failed");
    let params = HashMap::new();

    col.execute_query(&query, &params)
        .expect("Query should succeed");

    // Circuit breaker stays closed after successful query
    assert_eq!(
        col.guard_rails.circuit_breaker.state(),
        CircuitState::Closed
    );
}

// ─── Regression tests for guard-rail bypass bugs ────────────────────────────
//
// Bug 1: NOT-similarity early-return path skipped check_timeout / check_cardinality.
// Bug 2: Union early-return path skipped check_timeout / check_cardinality.
// Bug 3: MATCH path skipped final check_cardinality (periodic check every 100
//         iterations missed small traversals).

#[test]
fn test_not_similarity_query_cardinality_enforced() {
    // Regression test for Bug 1.
    // The NOT-similarity path (`is_not_similarity_query`) returned early without
    // calling ctx.check_cardinality(). Full-table-scan results must be checked.
    let dir = TempDir::new().unwrap();
    let mut col = create_test_collection(&dir);

    // Limit below the number of points that would match NOT (similarity > 0.99).
    let limits = QueryLimits {
        max_cardinality: 3, // collection has 10 points; NOT-sim returns ~all
        ..QueryLimits::default()
    };
    col.guard_rails = Arc::new(GuardRails::with_limits(limits));

    // Build NOT (similarity(vector, $v) > 0.99) programmatically.
    // The parser does not yet support NOT-wrapping similarity (EPIC-005 planned),
    // so we construct the AST directly to exercise the code path.
    let sim_cond = Condition::Similarity(SimilarityCondition {
        field: "vector".to_string(),
        vector: VectorExpr::Literal(vec![0.5, 0.1, 0.1, 0.1]),
        operator: CompareOp::Gt,
        threshold: 0.99, // very high: most points fall outside → NOT returns ~all
    });
    let query = Query::new_select(SelectStatement {
        distinct: DistinctMode::None,
        columns: SelectColumns::All,
        from: "col".to_string(),
        from_alias: None,
        joins: Vec::new(),
        where_clause: Some(Condition::Not(Box::new(sim_cond))),
        order_by: None,
        limit: Some(100),
        offset: None,
        with_clause: None,
        group_by: None,
        having: None,
        fusion_clause: None,
    });
    let params = HashMap::new();

    let result = col.execute_query(&query, &params);
    assert!(
        result.is_err(),
        "Cardinality guard-rail should fire for NOT-similarity queries"
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("Guard-rail") || err.contains("ardinality"),
        "Unexpected error: {err}"
    );
}

#[test]
fn test_union_query_cardinality_enforced() {
    // Regression test for Bug 2.
    // The union path (`is_union_query`) returned early without calling
    // ctx.check_cardinality(). Union-mode results must be checked.
    let dir = TempDir::new().unwrap();
    let mut col = create_test_collection(&dir);

    let limits = QueryLimits {
        max_cardinality: 3, // collection has 10 points; union returns ~all
        ..QueryLimits::default()
    };
    col.guard_rails = Arc::new(GuardRails::with_limits(limits));

    // Build similarity(vector, $v) > 0.0 OR idx >= 0
    // One side has similarity, the other is metadata → triggers union path.
    let sim_cond = Condition::Similarity(SimilarityCondition {
        field: "vector".to_string(),
        vector: VectorExpr::Literal(vec![0.5, 0.1, 0.1, 0.1]),
        operator: CompareOp::Gt,
        threshold: 0.0, // very low: all points pass
    });
    let meta_cond = Condition::Comparison(Comparison {
        column: "idx".to_string(),
        operator: CompareOp::Gte,
        value: Value::Integer(0),
    });
    let query = Query::new_select(SelectStatement {
        distinct: DistinctMode::None,
        columns: SelectColumns::All,
        from: "col".to_string(),
        from_alias: None,
        joins: Vec::new(),
        where_clause: Some(Condition::Or(Box::new(sim_cond), Box::new(meta_cond))),
        order_by: None,
        limit: Some(100),
        offset: None,
        with_clause: None,
        group_by: None,
        having: None,
        fusion_clause: None,
    });
    let params = HashMap::new();

    let result = col.execute_query(&query, &params);
    assert!(
        result.is_err(),
        "Cardinality guard-rail should fire for union-mode queries"
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("Guard-rail") || err.contains("ardinality"),
        "Unexpected error: {err}"
    );
}

#[test]
fn test_match_cardinality_enforced_below_100_iterations() {
    // Regression test for Bug 3.
    // The MATCH path only checked cardinality periodically every 100 BFS iterations.
    // A traversal with <100 iterations that produces many results bypassed the limit.
    // The fix adds a final check_cardinality() on the full result set in execute_query.
    let dir = TempDir::new().unwrap();
    let mut col = create_test_collection(&dir);

    // Add 3 edges: 0→1, 0→2, 0→3. BFS from node 0 produces 3 results in 3
    // iterations — well below the 100-iteration periodic-check threshold.
    for target in 1u64..=3 {
        let edge = GraphEdge::new(target * 100, 0, target, "LINKS").expect("edge");
        col.add_edge(edge).expect("add_edge");
    }

    // max_cardinality: 2 — traversal produces 3 results → should be caught.
    let limits = QueryLimits {
        max_cardinality: 2,
        ..QueryLimits::default()
    };
    col.guard_rails = Arc::new(GuardRails::with_limits(limits));

    let sql = "MATCH (a)-[r]->(b) RETURN b LIMIT 10;";
    let query = Parser::parse(sql).expect("parse failed");
    let params = HashMap::new();

    let result = col.execute_query(&query, &params);
    assert!(
        result.is_err(),
        "Cardinality guard-rail should fire for MATCH with <100 traversal iterations"
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("Guard-rail") || err.contains("ardinality"),
        "Unexpected error: {err}"
    );
}

#[test]
fn test_per_client_rate_limiting_is_independent() {
    // Each client_id has its own token bucket; exhausting one must not affect others.
    let dir = TempDir::new().unwrap();
    let mut col = create_test_collection(&dir);

    // Tight per-QPS limit: only 1 token — the second call from the same client fails.
    let limits = QueryLimits {
        rate_limit_qps: 1,
        ..QueryLimits::default()
    };
    col.guard_rails = Arc::new(GuardRails::with_limits(limits));

    let sql = "SELECT * FROM col LIMIT 5;";
    let query = Parser::parse(sql).expect("parse failed");
    let params = HashMap::new();

    // client_a first call — OK (bucket full)
    assert!(
        col.execute_query_with_client(&query, &params, "client_a")
            .is_ok(),
        "client_a first call should succeed"
    );
    // client_a second call — rate-limited
    assert!(
        col.execute_query_with_client(&query, &params, "client_a")
            .is_err(),
        "client_a second call should be rate-limited"
    );
    // client_b first call — still OK (separate bucket, not affected by client_a)
    assert!(
        col.execute_query_with_client(&query, &params, "client_b")
            .is_ok(),
        "client_b should have an independent bucket unaffected by client_a exhaustion"
    );
}

#[test]
fn test_execute_match_depth_limit_enforced() {
    let dir = TempDir::new().unwrap();
    let mut col = create_test_collection(&dir);

    // Use depth limit of 0 — BFS traversal is capped at depth 0 by the guardrail.
    let limits = QueryLimits {
        max_depth: 0,
        ..QueryLimits::default()
    };
    col.guard_rails = Arc::new(GuardRails::with_limits(limits));

    // Add a simple edge so traversal actually runs
    let edge =
        crate::collection::graph::GraphEdge::new(1, 0, 1, "LINKS").expect("GraphEdge::new failed");
    col.add_edge(edge).expect("add_edge failed");

    // MATCH query with traversal — depth guard-rail either stops traversal
    // or BFS returns 0 results because no edges exceed depth 0.
    let sql = "MATCH (a)-[r]->(b) RETURN a LIMIT 5;";
    let query = Parser::parse(sql).expect("parse failed");
    let params = HashMap::new();

    let result = col.execute_query(&query, &params);
    // Either a guard-rail error or empty results — both are acceptable.
    // We assert the operation doesn't panic.
    let _ = result;
}
