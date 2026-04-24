//! BDD-style end-to-end tests for `VelesQL` window functions
//! (`ROW_NUMBER`, `RANK`, `DENSE_RANK`) and their interactions with
//! DISTINCT + qualified wildcards.
//!
//! These tests exercise the **full pipeline** from the user's perspective:
//! SQL string -> `Parser::parse()` -> `Database::execute_query()` -> verify
//! the injected window values appear in the result payloads and the
//! DISTINCT-then-window ordering produces a contiguous `1..N` numbering.

use velesdb_core::{Database, Point};

use super::helpers::{create_test_db, execute_sql};

// =========================================================================
// Shared fixture: 6 "docs" rows across 3 sources with varying scores.
// =========================================================================

/// Seed a small `docs` collection suitable for window + DISTINCT tests.
///
/// | id | source | title    | score |
/// |----|--------|----------|-------|
/// |  1 | web    | Alpha    | 100.0 |
/// |  2 | web    | Bravo    |  90.0 |
/// |  3 | api    | Alpha    |  90.0 |  (same title as id=1 but different source)
/// |  4 | api    | Charlie  |  80.0 |
/// |  5 | kb     | Alpha    |  80.0 |  (same title as id=1 and id=3, third source)
/// |  6 | kb     | Bravo    |  70.0 |
fn setup_docs_collection(db: &Database) {
    execute_sql(
        db,
        "CREATE COLLECTION docs (dimension = 4, metric = 'cosine')",
    )
    .expect("test: create docs");

    let vc = db.get_vector_collection("docs").expect("test: get docs");
    vc.upsert(vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(serde_json::json!({"source": "web", "title": "Alpha", "score": 100.0})),
        ),
        Point::new(
            2,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(serde_json::json!({"source": "web", "title": "Bravo", "score": 90.0})),
        ),
        Point::new(
            3,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(serde_json::json!({"source": "api", "title": "Alpha", "score": 90.0})),
        ),
        Point::new(
            4,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(serde_json::json!({"source": "api", "title": "Charlie", "score": 80.0})),
        ),
        Point::new(
            5,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(serde_json::json!({"source": "kb", "title": "Alpha", "score": 80.0})),
        ),
        Point::new(
            6,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(serde_json::json!({"source": "kb", "title": "Bravo", "score": 70.0})),
        ),
    ])
    .expect("test: upsert docs");
}

fn payload_u64(result: &velesdb_core::SearchResult, field: &str) -> Option<u64> {
    result
        .point
        .payload
        .as_ref()
        .and_then(|p| p.get(field))
        .and_then(serde_json::Value::as_u64)
}

// =========================================================================
// ROW_NUMBER — simple case, verifies the full pipeline wires window values
// into the returned payloads.
// =========================================================================

#[test]
fn test_row_number_over_order_by_score_desc() {
    let (_dir, db) = create_test_db();
    setup_docs_collection(&db);

    // GIVEN: 6 docs with known scores.
    // WHEN: project ROW_NUMBER() OVER (ORDER BY score DESC) AS rn
    let results = execute_sql(
        &db,
        "SELECT id, score, ROW_NUMBER() OVER (ORDER BY score DESC) AS rn FROM docs LIMIT 10",
    )
    .expect("test: query");

    // THEN: every result carries a dense rn in payload. Sorted scores:
    //   100 (id=1), 90 (id=2), 90 (id=3), 80 (id=4), 80 (id=5), 70 (id=6)
    //   → rn = 1, 2, 3, 4, 5, 6 respectively.
    assert_eq!(results.len(), 6);
    let by_id: std::collections::HashMap<u64, u64> = results
        .iter()
        .map(|r| (r.point.id, payload_u64(r, "rn").expect("rn present")))
        .collect();

    assert_eq!(by_id[&1], 1, "score=100 → rn 1");
    assert_eq!(by_id[&6], 6, "score=70 → rn 6");
    // id 2 and 3 share score=90 — both get distinct row numbers (ROW_NUMBER
    // doesn't handle ties). Same for id 4 and 5 at score=80.
    let rns_for_tie_90: Vec<u64> = [2, 3].iter().map(|id| by_id[id]).collect();
    assert!(rns_for_tie_90.contains(&2) && rns_for_tie_90.contains(&3));
    let rns_for_tie_80: Vec<u64> = [4, 5].iter().map(|id| by_id[id]).collect();
    assert!(rns_for_tie_80.contains(&4) && rns_for_tie_80.contains(&5));
}

// =========================================================================
// RANK with PARTITION BY — per-source ranking with ties.
// =========================================================================

#[test]
fn test_rank_partition_by_source_order_by_score_desc() {
    let (_dir, db) = create_test_db();
    setup_docs_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT id, source, score, \
              RANK() OVER (PARTITION BY source ORDER BY score DESC) AS rnk \
              FROM docs LIMIT 10",
    )
    .expect("test: query");

    assert_eq!(results.len(), 6);

    // Partition "web": id=1 (100) → 1, id=2 (90) → 2
    // Partition "api": id=3 (90) → 1, id=4 (80) → 2
    // Partition "kb":  id=5 (80) → 1, id=6 (70) → 2
    let by_id: std::collections::HashMap<u64, u64> = results
        .iter()
        .map(|r| (r.point.id, payload_u64(r, "rnk").expect("rnk present")))
        .collect();
    assert_eq!(by_id[&1], 1);
    assert_eq!(by_id[&2], 2);
    assert_eq!(by_id[&3], 1);
    assert_eq!(by_id[&4], 2);
    assert_eq!(by_id[&5], 1);
    assert_eq!(by_id[&6], 2);
}

// =========================================================================
// DENSE_RANK with ties — no gaps after tie groups.
// =========================================================================

#[test]
fn test_dense_rank_single_partition_with_ties_has_no_gaps() {
    let (_dir, db) = create_test_db();
    setup_docs_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT id, score, DENSE_RANK() OVER (ORDER BY score DESC) AS drnk FROM docs LIMIT 10",
    )
    .expect("test: query");

    // Sorted by score DESC:
    //   100         → drnk 1
    //   90, 90      → drnk 2 (tie, no gap after)
    //   80, 80      → drnk 3 (tie, no gap after)
    //   70          → drnk 4
    assert_eq!(results.len(), 6);
    let by_id: std::collections::HashMap<u64, u64> = results
        .iter()
        .map(|r| (r.point.id, payload_u64(r, "drnk").expect("drnk present")))
        .collect();
    assert_eq!(by_id[&1], 1); // 100
    assert_eq!(by_id[&2], 2); // 90
    assert_eq!(by_id[&3], 2); // 90 tie
    assert_eq!(by_id[&4], 3); // 80
    assert_eq!(by_id[&5], 3); // 80 tie
    assert_eq!(by_id[&6], 4); // 70 (no gap)
}

// =========================================================================
// Pipeline regression: DISTINCT runs BEFORE window functions, so
// `SELECT DISTINCT title, ROW_NUMBER() ...` numbers DEDUPED survivors
// contiguously, without gaps.
//
// This pins the VelesQL-intentional-deviation from SQL standard order.
// =========================================================================

#[test]
fn test_distinct_then_window_numbers_survivors_contiguously() {
    let (_dir, db) = create_test_db();
    setup_docs_collection(&db);

    // 3 distinct titles: Alpha, Bravo, Charlie.
    let results = execute_sql(
        &db,
        "SELECT DISTINCT title, ROW_NUMBER() OVER (ORDER BY title ASC) AS rn \
              FROM docs LIMIT 10",
    )
    .expect("test: query");

    // THEN: exactly 3 survivors (DISTINCT dedup by title) and rn is 1, 2, 3
    // (contiguous — window function numbers DEDUPED rows).
    assert_eq!(
        results.len(),
        3,
        "DISTINCT collapses 6 rows down to 3 unique titles"
    );

    let mut rns: Vec<u64> = results
        .iter()
        .map(|r| payload_u64(r, "rn").expect("rn present"))
        .collect();
    rns.sort_unstable();
    assert_eq!(
        rns,
        vec![1, 2, 3],
        "window function numbers the 3 survivors 1..3, no gaps"
    );
}

// =========================================================================
// Pipeline regression: DISTINCT dedup key now includes qualified-wildcard
// fields. Rows that differ only on a wildcard-expanded field must survive.
//
// This pins the zero-tech-debt fix for the Devin finding on
// `apply_distinct` + `qualified_wildcards`.
// =========================================================================

#[test]
fn test_distinct_with_qualified_wildcard_dedupes_by_full_payload() {
    let (_dir, db) = create_test_db();
    setup_docs_collection(&db);

    // Three rows share title="Alpha" but differ on source/score:
    //   id=1 web 100, id=3 api 90, id=5 kb 80.
    // Without the fix, `DISTINCT docs.*, title` deduped by `title` only
    // and collapsed these three into one. With the fix, every wildcard
    // field participates in the dedup key → all three survive.
    let results = execute_sql(
        &db,
        "SELECT DISTINCT docs.*, title FROM docs WHERE title = 'Alpha' LIMIT 10",
    )
    .expect("test: query");

    assert_eq!(
        results.len(),
        3,
        "rows differ on source + score (wildcard-expanded) so all three survive"
    );

    // All three must have title=\"Alpha\" and unique source values.
    let mut sources: Vec<String> = results
        .iter()
        .map(|r| {
            r.point
                .payload
                .as_ref()
                .and_then(|p| p.get("source"))
                .and_then(serde_json::Value::as_str)
                .expect("source present")
                .to_string()
        })
        .collect();
    sources.sort();
    assert_eq!(sources, vec!["api", "kb", "web"]);
}

// =========================================================================
// Alias-collision regression: RANK() AS score on a column named `score`
// must NOT use the injected ranks for tie detection (the bug Devin flagged
// on the first review pass). Executed here through the full pipeline to
// confirm the evaluator guards against corruption end-to-end, not just in
// isolation.
// =========================================================================

#[test]
fn test_rank_alias_collides_with_payload_score_end_to_end() {
    let (_dir, db) = create_test_db();
    setup_docs_collection(&db);

    // Alias `score` collides with the ORDER BY column. Pre-fix, RANK
    // would write [1,2,3,4,5,6] into `payload.score` and destroy ties.
    let results = execute_sql(
        &db,
        "SELECT id, RANK() OVER (ORDER BY score DESC) AS score FROM docs LIMIT 10",
    )
    .expect("test: query");

    assert_eq!(results.len(), 6);

    // Expected ranks for sorted score [100, 90, 90, 80, 80, 70]: [1, 2, 2, 4, 4, 6].
    // The `score` alias overwrites the payload, so we read from `payload.score`.
    let ranks_by_id: std::collections::HashMap<u64, u64> = results
        .iter()
        .map(|r| (r.point.id, payload_u64(r, "score").expect("score present")))
        .collect();

    assert_eq!(ranks_by_id[&1], 1, "id=1 score=100 rank 1");
    // ids 2 and 3 tie at score=90 → both rank 2 (gap after tie):
    assert_eq!(ranks_by_id[&2], 2);
    assert_eq!(ranks_by_id[&3], 2);
    // ids 4 and 5 tie at score=80 → both rank 4:
    assert_eq!(ranks_by_id[&4], 4);
    assert_eq!(ranks_by_id[&5], 4);
    // id 6 at score=70 → rank 6 (gap preserved):
    assert_eq!(ranks_by_id[&6], 6);
}

// =========================================================================
// Negative-path: a SELECT with a window function on an empty result set
// must not panic and must produce no rows.
// =========================================================================

#[test]
fn test_window_function_on_empty_result_returns_no_rows() {
    let (_dir, db) = create_test_db();
    setup_docs_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT id, ROW_NUMBER() OVER (ORDER BY score DESC) AS rn \
              FROM docs WHERE score > 99999 LIMIT 10",
    )
    .expect("test: query");

    assert!(results.is_empty(), "filter selects nothing → 0 rows");
}

// =========================================================================
// Sanity: window functions coexist with existing similarity() projection.
// =========================================================================

#[test]
fn test_window_function_coexists_with_vector_search_near() {
    use super::helpers::{execute_sql_with_params, vector_param};

    let (_dir, db) = create_test_db();
    setup_docs_collection(&db);

    // Vector search context (NEAR) + window function — verifies the
    // window pipeline slot runs on NEAR-ranked results and injects its
    // alias into every row's payload.
    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let results = execute_sql_with_params(
        &db,
        "SELECT id, ROW_NUMBER() OVER (ORDER BY score DESC) AS rn \
              FROM docs WHERE vector NEAR $v LIMIT 10",
        &params,
    )
    .expect("test: query");

    assert_eq!(
        results.len(),
        6,
        "all 6 docs returned by NEAR (k=10 but only 6 exist)"
    );
    for r in &results {
        assert!(
            payload_u64(r, "rn").is_some(),
            "window alias `rn` in payload even when NEAR runs first"
        );
    }
}
