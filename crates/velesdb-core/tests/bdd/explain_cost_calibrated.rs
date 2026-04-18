//! BDD tests for calibrated EXPLAIN cost estimation (#471).
//!
//! Verifies that once ANALYZE has produced `CollectionStats`, `EXPLAIN`:
//! - Scales `estimated_cost_ms` with collection size and ef_search
//! - Switches `filter_strategy` based on selectivity vs. a recall guardrail
//! - Falls back bit-for-bit to the heuristic when stats are absent
//! - Does not panic on corrupt or empty stats
//!
//! These are nominal + edge + negative tests (§bdd-testing.md coverage rule).

use serde_json::json;
use velesdb_core::velesql::{FilterStrategy, Parser};
use velesdb_core::{Database, Point};

use super::helpers::{create_test_db, execute_sql, vector_param};

// =========================================================================
// Helpers
// =========================================================================

/// Creates a collection with `n` points, each carrying:
/// - `cat`  : string tag (`"rare"` for ids < n/100, `"common"` otherwise)
/// - `price`: integer uniformly in [0, 1000) so histograms have dispersion
///
/// The numeric `price` field drives the selectivity-switching test because
/// histograms on strings fall back to cardinality-only estimates, whereas
/// numeric histograms give true range selectivity.
fn seed_collection(db: &Database, name: &str, n: u64) {
    execute_sql(
        db,
        &format!("CREATE COLLECTION {name} (dimension = 4, metric = 'cosine')"),
    )
    .expect("test: create collection");

    let vc = db
        .get_vector_collection(name)
        .expect("test: get collection");

    let rare_limit = (n / 100).max(1);
    let batch: Vec<Point> = (0..n)
        .map(|i| {
            let cat = if i < rare_limit { "rare" } else { "common" };
            // Reason: i is a small cardinality (≤ 10k in tests), fits in u32.
            #[allow(clippy::cast_precision_loss)]
            let fi = i as f32;
            // `price` spans [0, 1000); most values are in a broad range so
            // `price < 10` picks ~1 %, `price > 100` picks ~90 %.
            let price = i % 1_000;
            Point::new(
                i + 1,
                vec![fi.sin(), fi.cos(), (fi * 0.5).sin(), (fi * 0.3).cos()],
                Some(json!({ "cat": cat, "price": price })),
            )
        })
        .collect();

    vc.upsert(batch).expect("test: upsert seed");
}

/// Parses `sql` and returns the EXPLAIN plan.
fn explain(db: &Database, sql: &str) -> velesdb_core::Result<velesdb_core::velesql::QueryPlan> {
    let query = Parser::parse(sql).map_err(|e| velesdb_core::Error::Query(e.to_string()))?;
    db.explain_query(&query)
}

// =========================================================================
// Nominal BDD — cost scales with collection size
// =========================================================================

/// GIVEN two collections with 100 and 10_000 points, both analyzed
/// WHEN EXPLAIN is called on an identical NEAR query
/// THEN the larger collection's estimated_cost_ms is strictly greater.
#[test]
fn test_explain_cost_scales_with_collection_size() {
    let (_dir, db) = create_test_db();

    seed_collection(&db, "small", 100);
    seed_collection(&db, "large", 10_000);

    execute_sql(&db, "ANALYZE small").expect("test: ANALYZE small");
    execute_sql(&db, "ANALYZE large").expect("test: ANALYZE large");

    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let q_small = Parser::parse("SELECT * FROM small WHERE vector NEAR $v LIMIT 10")
        .expect("test: parse small");
    let q_large = Parser::parse("SELECT * FROM large WHERE vector NEAR $v LIMIT 10")
        .expect("test: parse large");

    // Use explain_analyze_query so stats are resolved via the normal path.
    let plan_small = db
        .explain_analyze_query(&q_small, &params)
        .expect("test: explain small")
        .plan;
    let plan_large = db
        .explain_analyze_query(&q_large, &params)
        .expect("test: explain large")
        .plan;

    assert!(
        plan_large.estimated_cost_ms > plan_small.estimated_cost_ms,
        "large cost {} must exceed small cost {}",
        plan_large.estimated_cost_ms,
        plan_small.estimated_cost_ms
    );
}

// =========================================================================
// Nominal BDD — filter strategy switches based on calibrated selectivity
// =========================================================================

/// GIVEN a collection analyzed (histograms built) on a numeric `price` column
/// WHEN EXPLAIN runs NEAR + WHERE price < 10 (~1% selectivity)
/// THEN filter_strategy is PreFilter (small candidate set feeds HNSW fast)
/// AND  WHEN EXPLAIN runs NEAR + WHERE price > 100 (~90%, >= 0.5 guardrail)
/// THEN filter_strategy is PostFilter (recall guardrail trips)
#[test]
fn test_filter_strategy_switches_on_selectivity() {
    let (_dir, db) = create_test_db();
    seed_collection(&db, "docs", 1_000);
    execute_sql(&db, "ANALYZE docs").expect("test: ANALYZE");

    let rare_plan = explain(
        &db,
        "SELECT * FROM docs WHERE price < 10 AND vector NEAR $v LIMIT 10",
    )
    .expect("test: explain selective");
    let loose_plan = explain(
        &db,
        "SELECT * FROM docs WHERE price > 100 AND vector NEAR $v LIMIT 10",
    )
    .expect("test: explain loose");

    // A narrow range (~1% of rows) should NOT trip the recall guardrail;
    // the cost-based comparison is free to pick PreFilter.
    assert_eq!(
        rare_plan.filter_strategy,
        FilterStrategy::PreFilter,
        "narrow range (~1%) must use PreFilter (cost-based)"
    );

    // Loose range (~90%) must trip the recall guardrail → PostFilter.
    assert_eq!(
        loose_plan.filter_strategy,
        FilterStrategy::PostFilter,
        "loose range (~90%) should PostFilter via recall guardrail"
    );
}

// =========================================================================
// Edge BDD — fallback without stats is bit-for-bit identical
// =========================================================================

/// GIVEN a collection with data but NEVER analyzed (no stats cached)
/// WHEN EXPLAIN runs
/// THEN estimated_cost_ms equals the heuristic (from_select without stats).
///
/// This guards backward compatibility with the ~50 existing EXPLAIN tests.
#[test]
fn test_explain_cost_fallback_without_stats() {
    let (_dir, db) = create_test_db();
    seed_collection(&db, "docs", 50);
    // NOTE: no ANALYZE — stats are absent.

    let sql = "SELECT * FROM docs WHERE vector NEAR $v LIMIT 5";
    let query = Parser::parse(sql).expect("test: parse");
    let db_plan = db.explain_query(&query).expect("test: explain");

    // Direct heuristic path for comparison.
    let heuristic_plan = velesdb_core::velesql::QueryPlan::from_query(&query);

    assert!(
        (db_plan.estimated_cost_ms - heuristic_plan.estimated_cost_ms).abs() < f64::EPSILON,
        "without stats, db cost {} must equal heuristic cost {}",
        db_plan.estimated_cost_ms,
        heuristic_plan.estimated_cost_ms
    );
    assert_eq!(
        db_plan.filter_strategy, heuristic_plan.filter_strategy,
        "filter_strategy must be identical on fallback path"
    );
}

// =========================================================================
// Edge BDD — unanalyzed collections return equal costs
// =========================================================================

/// GIVEN two freshly-created collections with 100 and 10_000 points
/// AND   neither has been analyzed
/// WHEN EXPLAIN is called
/// THEN both plans use the heuristic (identical cost — size-independent
/// baseline), so they are equal within floating-point epsilon.
///
/// This is the companion to the "scales with size" test and guards against
/// silent regressions where stats would leak through unexpectedly.
#[test]
fn test_explain_cost_without_stats_is_size_independent() {
    let (_dir, db) = create_test_db();
    seed_collection(&db, "tiny", 100);
    seed_collection(&db, "huge", 10_000);
    // deliberately skip ANALYZE on both

    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let q_tiny = Parser::parse("SELECT * FROM tiny WHERE vector NEAR $v LIMIT 10")
        .expect("test: parse tiny");
    let q_huge = Parser::parse("SELECT * FROM huge WHERE vector NEAR $v LIMIT 10")
        .expect("test: parse huge");

    let p_tiny = db
        .explain_analyze_query(&q_tiny, &params)
        .expect("test: explain tiny")
        .plan;
    let p_huge = db
        .explain_analyze_query(&q_huge, &params)
        .expect("test: explain huge")
        .plan;

    assert!(
        (p_tiny.estimated_cost_ms - p_huge.estimated_cost_ms).abs() < f64::EPSILON,
        "without stats, cost must be size-independent"
    );
}

// =========================================================================
// Negative BDD — pathological stats do not panic
// =========================================================================

/// GIVEN a collection that has been created AND analyzed (so stats exist)
/// AND   the query references a column that has no histogram (zero info)
/// WHEN EXPLAIN runs
/// THEN the call succeeds, estimated_cost_ms is finite and positive,
///      and filter_strategy is assigned (never panics).
#[test]
fn test_explain_cost_on_unknown_column_does_not_panic() {
    let (_dir, db) = create_test_db();
    seed_collection(&db, "docs", 200);
    execute_sql(&db, "ANALYZE docs").expect("test: ANALYZE");

    // Filter on a column that was never populated (no histogram).
    let plan = explain(
        &db,
        "SELECT * FROM docs WHERE nonexistent = 42 AND vector NEAR $v LIMIT 5",
    )
    .expect("test: explain on missing column");

    assert!(
        plan.estimated_cost_ms.is_finite() && plan.estimated_cost_ms > 0.0,
        "cost must be finite and positive even without histogram: {}",
        plan.estimated_cost_ms
    );
}
