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
use velesdb_core::velesql::{FilterStrategy, IndexType, Parser, PlanNode};
use velesdb_core::{Database, Point};

use super::helpers::{create_test_db, execute_sql, execute_sql_with_params, vector_param};

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
// Nominal BDD — filter strategy respects ef_search (Devin finding 4, #471)
// =========================================================================

/// GIVEN a collection with a price histogram tuned to ~2 % selectivity
///   (`price < 20` over the [0, 1000) domain)
/// AND   default `ef_search = 100` is used
/// WHEN EXPLAIN is called
/// THEN the cost-based comparison is free to pick PreFilter.
/// AND  WHEN the same query is run with `WITH (ef_search = 2000)` — a value
///      large enough that the `hnsw_cost * selectivity` term dominates the
///      filter scan cost
/// THEN the strategy stays valid (one of PreFilter / PostFilter) and the
///      cost reported in estimated_cost_ms scales up with ef_search, proving
///      resolve_filter_strategy no longer hard-codes k = 10.
#[test]
fn test_filter_strategy_respects_ef_search() {
    let (_dir, db) = create_test_db();
    seed_collection(&db, "docs", 1_000);
    execute_sql(&db, "ANALYZE docs").expect("test: ANALYZE");

    let default_ef = explain(
        &db,
        "SELECT * FROM docs WHERE price < 20 AND vector NEAR $v LIMIT 10",
    )
    .expect("test: explain default ef_search");
    let high_ef = explain(
        &db,
        "SELECT * FROM docs WHERE price < 20 AND vector NEAR $v LIMIT 10 \
         WITH (ef_search = 2000)",
    )
    .expect("test: explain high ef_search");

    // Both plans must exist with a sensible strategy (non-None).
    assert_ne!(
        default_ef.filter_strategy,
        FilterStrategy::None,
        "default ef_search plan must select a strategy"
    );
    assert_ne!(
        high_ef.filter_strategy,
        FilterStrategy::None,
        "ef_search=2000 plan must select a strategy"
    );

    // Critical: the HNSW cost reflected in estimated_cost_ms must scale with
    // ef_search. If resolve_filter_strategy still hard-coded k=10, this
    // invariant would fail because the cost model would ignore ef_search.
    assert!(
        high_ef.estimated_cost_ms > default_ef.estimated_cost_ms,
        "ef_search=2000 must yield higher cost than default ef_search=100: \
         default={} high={}",
        default_ef.estimated_cost_ms,
        high_ef.estimated_cost_ms
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

// =========================================================================
// Nominal BDD — pre-filter scans FULL table (Devin finding A, #606)
// =========================================================================

/// GIVEN a large analyzed collection (10 000 points)
/// AND   a filter with ~1 % selectivity (`price < 10` on a uniform [0,1000))
/// AND   a query with NEAR + LIMIT 10 (default ef_search=100)
/// WHEN EXPLAIN is called
/// THEN filter_strategy must be PostFilter, because evaluating the predicate
///      on ALL 10 000 rows (pre-filter scan = full table) costs more than
///      running HNSW first and filtering the top-k afterwards.
///
/// This mirrors the concrete example in the Devin-finding-A report on
/// #606: with `total=10_000`, `sel=0.01`, `ef=100`, `k=10` and
/// `hnsw_cost≈2000`, the correct (post-fix) cost model yields
/// `pre_filter ≈ 10_000 + 2000*0.01 ≈ 10_020` vs
/// `post_filter ≈ 2000 + 10_000*0.01*0.01 ≈ 2001` — PostFilter wins by ~5×.
///
/// Before the fix, `resolve_filter_strategy` priced the pre-filter scan as
/// `total * sel` (matching rows only), under-estimating the true cost by
/// a factor of `1 / selectivity`. That shortcut made PreFilter look
/// artificially cheap and the CBO would (incorrectly) pick PreFilter
/// here (`pre_filter_bug ≈ 100 + 20 = 120`).
#[test]
fn test_prefilter_accounts_for_full_table_scan() {
    let (_dir, db) = create_test_db();
    seed_collection(&db, "docs", 10_000);
    execute_sql(&db, "ANALYZE docs").expect("test: ANALYZE");

    let plan = explain(
        &db,
        "SELECT * FROM docs WHERE price < 10 AND vector NEAR $v LIMIT 10",
    )
    .expect("test: explain tight filter on large collection");

    assert_eq!(
        plan.filter_strategy,
        FilterStrategy::PostFilter,
        "tight filter (~1 %) on 10K-row collection: scanning the full \
         table for the predicate must cost more than HNSW + top-k filter, \
         so the CBO should pick PostFilter. If this asserts PreFilter, \
         resolve_filter_strategy is under-pricing the pre-filter scan \
         (Devin finding A regression)."
    );
}

// =========================================================================
// BDD — issue #609: post-filter cost modelled as k * cpu_tuple_cost
// =========================================================================

/// GIVEN a large analyzed collection (10K rows) and a query whose filter
///       selectivity sits just below the `PREFILTER_RECALL_GUARD = 0.5`
///       threshold,
/// WHEN  EXPLAIN runs a hybrid NEAR + predicate query with the new post-filter
///       cost model (`k * cpu_tuple_cost * cpu_ratio`),
/// THEN  the reported `filter_strategy` is `PostFilter` — because the HNSW
///       term plus the true `k`-scaled post-filter cost beats a full scan
///       + HNSW-on-reduced-set path. Under the old
///       `POSTFILTER_TOPK_COST_FRACTION = 0.01` formula the post-filter cost
///       was inflated by up to ~5× for this regime, which used to flip the
///       strategy to `PreFilter` incorrectly for selectivities close to but
///       below the guardrail.
#[test]
fn test_postfilter_preferred_on_large_collection_near_guardrail() {
    let (_dir, db) = create_test_db();
    seed_collection(&db, "docs", 10_000);
    execute_sql(&db, "ANALYZE docs").expect("test: ANALYZE docs");

    // Selectivity ≈ 0.4 via `price < 400` on values uniformly in [0, 1000).
    // Below the 0.5 recall guardrail so both branches of resolve_filter_strategy
    // engage the cost comparison — this is exactly where the #609 fix changes
    // the decision for large collections.
    let plan = explain(
        &db,
        "SELECT * FROM docs WHERE vector NEAR $v AND price < 400 LIMIT 10",
    )
    .expect("test: EXPLAIN hybrid near-guardrail");

    assert_eq!(
        plan.filter_strategy,
        FilterStrategy::PostFilter,
        "large collection + sel ≈ 0.4 must choose PostFilter with the \
         k*cpu_tuple_cost model (issue #609). Old model inflated post-filter \
         cost by ~5× and would flip to PreFilter here."
    );
}

// =========================================================================
// BDD — issue #607: IndexLookup plan nodes for indexed columns
// =========================================================================

/// Walks the plan tree and returns true when any node is an `IndexLookup`.
/// Recurses into `Sequence` children because the plan root for
/// `SELECT ... WHERE col = 'x' LIMIT N` is a `Sequence([IndexLookup, Filter,
/// Limit])` shape, not a bare `IndexLookup` leaf.
fn plan_contains_index_lookup(node: &PlanNode) -> bool {
    match node {
        PlanNode::IndexLookup(_) => true,
        PlanNode::Sequence(children) => children.iter().any(plan_contains_index_lookup),
        _ => false,
    }
}

/// GIVEN a collection with a registered secondary index on a metadata column,
/// WHEN  EXPLAIN runs a pure-WHERE equality query targeting that column,
/// THEN  the plan tree contains an `IndexLookup` node AND `index_used` is
///       `Some(IndexType::Property)` — proving that `build_plan_with_stats`
///       now threads the real indexed-field set through
///       `from_query_with_stats` (issue #607 closure).
#[test]
fn test_explain_generates_index_lookup_for_indexed_field() {
    let (_dir, db) = create_test_db();
    seed_collection(&db, "docs", 500);

    execute_sql(&db, "CREATE INDEX ON docs (cat)").expect("test: CREATE INDEX on cat");
    execute_sql(&db, "ANALYZE docs").expect("test: ANALYZE so stats path runs");

    let plan = explain(&db, "SELECT * FROM docs WHERE cat = 'rare' LIMIT 10")
        .expect("test: EXPLAIN indexed equality");

    assert_eq!(
        plan.index_used,
        Some(IndexType::Property),
        "indexed equality query must report Property index use, got {:?}",
        plan.index_used
    );
    assert!(
        plan_contains_index_lookup(&plan.root),
        "plan tree must contain an IndexLookup node; tree: {:?}",
        plan.root
    );
}

/// Negative counterpart: when the column is NOT indexed, EXPLAIN must still
/// fall back to a `TableScan` tree without claiming an `IndexLookup`.
#[test]
fn test_explain_falls_back_to_table_scan_when_column_not_indexed() {
    let (_dir, db) = create_test_db();
    seed_collection(&db, "docs", 500);
    // No CREATE INDEX on `cat` this time.
    execute_sql(&db, "ANALYZE docs").expect("test: ANALYZE");

    let plan = explain(&db, "SELECT * FROM docs WHERE cat = 'rare' LIMIT 10")
        .expect("test: EXPLAIN non-indexed equality");

    assert!(
        !plan_contains_index_lookup(&plan.root),
        "no CREATE INDEX should mean no IndexLookup in the plan tree; tree: {:?}",
        plan.root
    );
    assert_ne!(
        plan.index_used,
        Some(IndexType::Property),
        "index_used must not be Property when no secondary index exists"
    );
}

// =========================================================================
// BDD — issue #608: plan cache invalidation on ANALYZE
// =========================================================================

/// GIVEN a collection with data but no ANALYZE yet, and a cached EXPLAIN
///       plan built from the pre-ANALYZE heuristic cost estimates,
/// WHEN  ANALYZE is run on the collection (no intervening write),
/// THEN  a subsequent EXPLAIN on the identical query returns a plan whose
///       `estimated_cost_ms` differs from the first one, proving the cache
///       was invalidated by the analyze_generation bump (issue #608).
///
/// The acceptance criterion from the issue is "no intermediate write is
/// required for c2 to appear" — this test deliberately avoids any upsert
/// or delete between the two EXPLAIN calls so a pass means the cache key
/// flip came exclusively from `analyze_generation`, not from an incidental
/// `write_generation` bump.
#[test]
fn test_analyze_invalidates_plan_cache_without_write() {
    let (_dir, db) = create_test_db();
    seed_collection(&db, "docs", 1_000);

    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);

    // Force the plan cache to populate with the pre-ANALYZE heuristic plan.
    // `execute_query` is the only entry that writes to the plan cache, and
    // `explain_query` by design does NOT populate it — so a bare SELECT
    // with bound parameters must run first.
    execute_sql_with_params(
        &db,
        "SELECT * FROM docs WHERE vector NEAR $v LIMIT 10",
        &params,
    )
    .expect("test: prime cache with pre-ANALYZE plan");

    let query = Parser::parse("SELECT * FROM docs WHERE vector NEAR $v LIMIT 10")
        .expect("test: parse pre-analyze");

    let plan_before = db
        .explain_analyze_query(&query, &params)
        .expect("test: explain pre-analyze")
        .plan;
    let cost_before = plan_before.estimated_cost_ms;

    // ANALYZE is the ONLY mutation — no upsert/delete in between.
    execute_sql(&db, "ANALYZE docs").expect("test: ANALYZE docs");

    let plan_after = db
        .explain_analyze_query(&query, &params)
        .expect("test: explain post-analyze")
        .plan;
    let cost_after = plan_after.estimated_cost_ms;

    assert!(
        (cost_before - cost_after).abs() > f64::EPSILON,
        "ANALYZE must flip the plan cache key (issue #608): cost_before={cost_before} cost_after={cost_after} — staleness means analyze_generation is not threaded into the cache key"
    );
}

/// GIVEN a collection seeded and analyzed once (c1 recorded),
/// WHEN  a second ANALYZE is run (no intervening write),
/// THEN  the analyze_generation bumps again and the plan cache invalidates,
///       so a third EXPLAIN returns a plan distinct from the c1 cache entry.
///
/// This is the "rolling ANALYZE" case — even after stats exist, subsequent
/// runs must continue to invalidate.
#[test]
fn test_repeated_analyze_keeps_invalidating_plan_cache() {
    let (_dir, db) = create_test_db();
    seed_collection(&db, "docs", 1_000);
    execute_sql(&db, "ANALYZE docs").expect("test: first ANALYZE");

    // Prime the cache with the post-first-analyze plan.
    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    execute_sql_with_params(
        &db,
        "SELECT * FROM docs WHERE vector NEAR $v LIMIT 10",
        &params,
    )
    .expect("test: prime cache post-ANALYZE-1");

    // Capture the analyze_generation so we can assert it bumped.
    let gen_before = db
        .collection_analyze_generation("docs")
        .expect("test: collection exists");

    execute_sql(&db, "ANALYZE docs").expect("test: second ANALYZE");

    let gen_after = db
        .collection_analyze_generation("docs")
        .expect("test: collection exists");

    assert!(
        gen_after > gen_before,
        "second ANALYZE must bump analyze_generation: {gen_before} -> {gen_after}"
    );
}
