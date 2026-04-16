//! BDD-style end-to-end tests for complex `VelesQL` query compositions.
//!
//! These tests exercise **real-world usage patterns** that combine vector search
//! with metadata filters, text search, aggregation, ordering, and mutations in
//! a single query.  Every scenario uses the **full pipeline**: SQL string ->
//! `Parser::parse()` -> `Database::execute_query()` -> verify results.

use std::collections::HashSet;

use serde_json::json;
use velesdb_core::{Database, Point};

use super::helpers::{
    create_test_db, execute_sql, execute_sql_with_params, payload_f64, payload_str, result_ids,
    vector_param,
};

// =========================================================================
// Module-specific setup
// =========================================================================

/// Populate an "articles" collection with diverse data for complex queries.
///
/// Three clusters plus cross-cutting metadata for filter composition:
///
/// | id | vector direction | category | year | price | author     | active | title                  | tags                       |
/// |----|------------------|----------|------|-------|------------|--------|------------------------|----------------------------|
/// | 1  | `[1,0,0,0]`     | science  | 2024 | 29.99 | Dr. Smith  | true   | Quantum Physics Today  | physics quantum            |
/// | 2  | `[.95,.05,0,0]`  | science  | 2023 | 19.99 | Dr. Jones  | true   | Chemistry for Beginners| chemistry intro            |
/// | 3  | `[.9,.1,0,0]`    | science  | 2024 | 39.99 | Dr. Smith  | false  | Biology Advances       | biology research           |
/// | 4  | `[0,1,0,0]`      | tech     | 2024 | 49.99 | Alice      | true   | Rust Programming Guide | rust programming language  |
/// | 5  | `[.05,.95,0,0]`  | tech     | 2023 | 34.99 | Bob        | true   | Python Data Science    | python data science        |
/// | 6  | `[.1,.9,0,0]`    | tech     | 2022 | 24.99 | Alice      | true   | JavaScript Web Dev     | javascript web frontend    |
/// | 7  | `[0,0,1,0]`      | history  | 2021 | 15.99 | Prof. Lee  | true   | World War II History   | history war                |
/// | 8  | `[0,0,.9,.1]`    | history  | 2020 | 12.99 | Prof. Lee  | false  | Ancient Rome           | history rome ancient       |
fn setup_rich_collection(db: &Database) {
    execute_sql(
        db,
        "CREATE COLLECTION articles (dimension = 4, metric = 'cosine');",
    )
    .expect("test: CREATE articles");

    let vc = db
        .get_vector_collection("articles")
        .expect("test: articles collection must exist");

    vc.upsert(vec![
        Point::new(1, vec![1.0, 0.0, 0.0, 0.0], Some(json!({
            "title": "Quantum Physics Today", "category": "science", "year": 2024,
            "price": 29.99, "author": "Dr. Smith", "active": true, "tags": "physics quantum"
        }))),
        Point::new(2, vec![0.95, 0.05, 0.0, 0.0], Some(json!({
            "title": "Chemistry for Beginners", "category": "science", "year": 2023,
            "price": 19.99, "author": "Dr. Jones", "active": true, "tags": "chemistry intro"
        }))),
        Point::new(3, vec![0.9, 0.1, 0.0, 0.0], Some(json!({
            "title": "Biology Advances", "category": "science", "year": 2024,
            "price": 39.99, "author": "Dr. Smith", "active": false, "tags": "biology research"
        }))),
        Point::new(4, vec![0.0, 1.0, 0.0, 0.0], Some(json!({
            "title": "Rust Programming Guide", "category": "tech", "year": 2024,
            "price": 49.99, "author": "Alice", "active": true, "tags": "rust programming language"
        }))),
        Point::new(5, vec![0.05, 0.95, 0.0, 0.0], Some(json!({
            "title": "Python Data Science", "category": "tech", "year": 2023,
            "price": 34.99, "author": "Bob", "active": true, "tags": "python data science"
        }))),
        Point::new(6, vec![0.1, 0.9, 0.0, 0.0], Some(json!({
            "title": "JavaScript Web Dev", "category": "tech", "year": 2022,
            "price": 24.99, "author": "Alice", "active": true, "tags": "javascript web frontend"
        }))),
        Point::new(7, vec![0.0, 0.0, 1.0, 0.0], Some(json!({
            "title": "World War II History", "category": "history", "year": 2021,
            "price": 15.99, "author": "Prof. Lee", "active": true, "tags": "history war"
        }))),
        Point::new(8, vec![0.0, 0.0, 0.9, 0.1], Some(json!({
            "title": "Ancient Rome", "category": "history", "year": 2020,
            "price": 12.99, "author": "Prof. Lee", "active": false, "tags": "history rome ancient"
        }))),
    ])
    .expect("test: upsert articles corpus");
}

// =========================================================================
// Scenario 1: NEAR + three metadata filters
// =========================================================================

/// GIVEN a rich articles collection
/// WHEN querying NEAR `[1,0,0,0]` AND category='science' AND price>20 AND active=true
/// THEN only id 1 matches (Quantum Physics, science, 29.99, active).
#[test]
fn test_near_with_three_metadata_filters() {
    let (_dir, db) = create_test_db();
    setup_rich_collection(&db);

    let sql = "SELECT * FROM articles WHERE vector NEAR $v \
               AND category = 'science' AND price > 20 AND active = true LIMIT 5";
    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let results =
        execute_sql_with_params(&db, sql, &params).expect("test: NEAR + 3 metadata filters");

    let ids = result_ids(&results);
    assert!(
        ids.contains(&1),
        "id=1 should match all three filters, got {:?}",
        ids
    );

    for r in &results {
        let cat = payload_str(r, "category");
        assert_eq!(
            cat,
            Some("science"),
            "category must be 'science' for id={}",
            r.point.id
        );

        let price = payload_f64(r, "price").expect("test: price must exist");
        assert!(
            price > 20.0,
            "price must be > 20, got {} for id={}",
            price,
            r.point.id
        );

        let active = r
            .point
            .payload
            .as_ref()
            .and_then(|p| p.get("active"))
            .and_then(serde_json::Value::as_bool);
        assert_eq!(
            active,
            Some(true),
            "active must be true for id={}",
            r.point.id
        );
    }
}

// =========================================================================
// Scenario 2: NEAR + LIKE pattern
// =========================================================================

/// GIVEN a rich articles collection
/// WHEN querying NEAR `[0,1,0,0]` AND title LIKE '%Programming%'
/// THEN id 4 (Rust Programming Guide) is returned.
#[test]
fn test_near_with_like_pattern() {
    let (_dir, db) = create_test_db();
    setup_rich_collection(&db);

    let sql = "SELECT * FROM articles WHERE vector NEAR $v \
               AND title LIKE '%Programming%' LIMIT 5";
    let params = vector_param(&[0.0, 1.0, 0.0, 0.0]);
    let results = execute_sql_with_params(&db, sql, &params).expect("test: NEAR + LIKE pattern");

    let ids = result_ids(&results);
    assert!(
        ids.contains(&4),
        "id=4 ('Rust Programming Guide') must match LIKE '%%Programming%%', got {:?}",
        ids
    );

    for r in &results {
        let title = payload_str(r, "title").expect("test: title must exist");
        assert!(
            title.contains("Programming"),
            "title must contain 'Programming', got '{}' for id={}",
            title,
            r.point.id
        );
    }
}

// =========================================================================
// Scenario 3: NEAR + ILIKE (case-insensitive)
// =========================================================================

/// GIVEN a rich articles collection
/// WHEN querying NEAR `[0,1,0,0]` AND title ILIKE '%rust%'
/// THEN id 4 matches (ILIKE matches "Rust" case-insensitively).
#[test]
fn test_near_with_ilike_case_insensitive() {
    let (_dir, db) = create_test_db();
    setup_rich_collection(&db);

    let sql = "SELECT * FROM articles WHERE vector NEAR $v \
               AND title ILIKE '%rust%' LIMIT 5";
    let params = vector_param(&[0.0, 1.0, 0.0, 0.0]);
    let results = execute_sql_with_params(&db, sql, &params).expect("test: NEAR + ILIKE");

    let ids = result_ids(&results);
    assert!(
        ids.contains(&4),
        "id=4 ('Rust Programming Guide') must match ILIKE '%%rust%%', got {:?}",
        ids
    );

    for r in &results {
        let title = payload_str(r, "title").expect("test: title must exist");
        assert!(
            title.to_lowercase().contains("rust"),
            "title must contain 'rust' (case-insensitive), got '{}' for id={}",
            title,
            r.point.id
        );
    }
}

// =========================================================================
// Scenario 4: NEAR + BETWEEN + IS NOT NULL
// =========================================================================

/// GIVEN a rich articles collection
/// WHEN querying NEAR `[1,0,0,0]` AND price BETWEEN 20 AND 40 AND author IS NOT NULL
/// THEN ids 1 (29.99) and 3 (39.99) match -- both science cluster, priced 20-40.
#[test]
fn test_near_with_between_and_not_null() {
    let (_dir, db) = create_test_db();
    setup_rich_collection(&db);

    let sql = "SELECT * FROM articles WHERE vector NEAR $v \
               AND price BETWEEN 20 AND 40 AND author IS NOT NULL LIMIT 10";
    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let results =
        execute_sql_with_params(&db, sql, &params).expect("test: NEAR + BETWEEN + IS NOT NULL");

    let ids = result_ids(&results);
    assert!(
        ids.contains(&1),
        "id=1 (price=29.99) should match BETWEEN 20 AND 40, got {:?}",
        ids
    );
    assert!(
        ids.contains(&3),
        "id=3 (price=39.99) should match BETWEEN 20 AND 40, got {:?}",
        ids
    );

    for r in &results {
        let price = payload_f64(r, "price").expect("test: price must exist");
        assert!(
            (20.0..=40.0).contains(&price),
            "price must be in [20, 40], got {} for id={}",
            price,
            r.point.id
        );

        let author = payload_str(r, "author");
        assert!(
            author.is_some(),
            "author must be non-null for id={}",
            r.point.id
        );
    }
}

// =========================================================================
// Scenario 5: NEAR + IN + ORDER BY similarity() DESC
// =========================================================================

/// GIVEN a rich articles collection
/// WHEN querying NEAR `[1,0,0,0]` AND category IN ('science','tech')
///      ORDER BY `similarity()` DESC
/// THEN science items appear first (closer to query), tech items last.
#[test]
fn test_near_with_in_and_order_by_similarity() {
    let (_dir, db) = create_test_db();
    setup_rich_collection(&db);

    let sql = "SELECT * FROM articles WHERE vector NEAR $v \
               AND category IN ('science', 'tech') \
               ORDER BY similarity() DESC LIMIT 10";
    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let results = execute_sql_with_params(&db, sql, &params)
        .expect("test: NEAR + IN + ORDER BY similarity()");

    assert!(!results.is_empty(), "Should return at least one result");

    for r in &results {
        let cat = payload_str(r, "category").expect("test: category must exist");
        assert!(
            cat == "science" || cat == "tech",
            "category must be science or tech, got '{}' for id={}",
            cat,
            r.point.id
        );
    }

    for w in results.windows(2) {
        assert!(
            w[0].score >= w[1].score,
            "Scores must be non-increasing: {} (id={}) >= {} (id={})",
            w[0].score,
            w[0].point.id,
            w[1].score,
            w[1].point.id
        );
    }
}

// =========================================================================
// Scenario 6: NEAR + NOT + OR
// =========================================================================

/// GIVEN a rich articles collection
/// WHEN querying NEAR `[1,0,0,0]` AND NOT(category='history')
///      AND (active=true OR price < 20)
/// THEN history items are excluded; remaining must satisfy the OR clause.
#[test]
fn test_near_with_not_and_or() {
    let (_dir, db) = create_test_db();
    setup_rich_collection(&db);

    let sql = "SELECT * FROM articles WHERE vector NEAR $v \
               AND NOT (category = 'history') \
               AND (active = true OR price < 20) LIMIT 10";
    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let results = execute_sql_with_params(&db, sql, &params).expect("test: NEAR + NOT + OR");

    assert!(!results.is_empty(), "Should return at least one result");

    for r in &results {
        let cat = payload_str(r, "category").expect("test: category must exist");
        assert_ne!(
            cat, "history",
            "history must be excluded, got id={}",
            r.point.id
        );

        let active = r
            .point
            .payload
            .as_ref()
            .and_then(|p| p.get("active"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let price = payload_f64(r, "price").unwrap_or(f64::MAX);
        assert!(
            active || price < 20.0,
            "must satisfy (active=true OR price<20), got active={}, price={} for id={}",
            active,
            price,
            r.point.id
        );
    }
}

// =========================================================================
// Scenario 7: NEAR + temporal filter (year >= 2023)
// =========================================================================

/// GIVEN a rich articles collection
/// WHEN querying NEAR `[1,0,0,0]` AND year >= 2023
/// THEN only 2023+ articles from the science cluster are returned.
#[test]
fn test_near_with_temporal_filter() {
    let (_dir, db) = create_test_db();
    setup_rich_collection(&db);

    let sql = "SELECT * FROM articles WHERE vector NEAR $v AND year >= 2023 LIMIT 5";
    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let results = execute_sql_with_params(&db, sql, &params).expect("test: NEAR + year >= 2023");

    assert!(!results.is_empty(), "Should return at least one result");

    for r in &results {
        let year = r
            .point
            .payload
            .as_ref()
            .and_then(|p| p.get("year"))
            .and_then(serde_json::Value::as_i64)
            .expect("test: year must exist");
        assert!(
            year >= 2023,
            "year must be >= 2023, got {} for id={}",
            year,
            r.point.id
        );
    }

    let top_cat = payload_str(&results[0], "category");
    assert_eq!(
        top_cat,
        Some("science"),
        "Top result for [1,0,0,0] with year>=2023 should be science"
    );
}

// =========================================================================
// Scenario 8: DISTINCT + NEAR
// =========================================================================

/// GIVEN a rich articles collection
/// WHEN querying SELECT DISTINCT category with NEAR `[0.5,0.5,0,0]`
/// THEN returns deduplicated categories from the nearest results.
#[test]
fn test_distinct_with_near() {
    let (_dir, db) = create_test_db();
    setup_rich_collection(&db);

    let sql = "SELECT DISTINCT category FROM articles WHERE vector NEAR $v LIMIT 10";
    let params = vector_param(&[0.5, 0.5, 0.0, 0.0]);
    let results = execute_sql_with_params(&db, sql, &params).expect("test: DISTINCT + NEAR");

    let categories: HashSet<&str> = results
        .iter()
        .filter_map(|r| payload_str(r, "category"))
        .collect();

    assert!(
        categories.len() >= 2,
        "DISTINCT should yield at least 2 unique categories, got {:?}",
        categories
    );

    assert_eq!(
        categories.len(),
        results.len(),
        "DISTINCT must produce one row per unique category: {} categories vs {} rows",
        categories.len(),
        results.len()
    );
}

// =========================================================================
// Scenario 9: NEAR + ORDER BY payload field (not similarity)
// =========================================================================

/// GIVEN a rich articles collection
/// WHEN querying NEAR `[1,0,0,0]` ORDER BY price ASC
/// THEN results are ordered by ascending price, not by similarity score.
#[test]
fn test_near_order_by_field_not_similarity() {
    let (_dir, db) = create_test_db();
    setup_rich_collection(&db);

    let sql = "SELECT * FROM articles WHERE vector NEAR $v \
               ORDER BY price ASC LIMIT 5";
    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let results =
        execute_sql_with_params(&db, sql, &params).expect("test: NEAR + ORDER BY price ASC");

    assert!(
        results.len() >= 2,
        "Need at least 2 results to verify ordering"
    );

    for w in results.windows(2) {
        let a = payload_f64(&w[0], "price").expect("test: price field");
        let b = payload_f64(&w[1], "price").expect("test: price field");
        assert!(
            a <= b,
            "Results should be in ascending price order: {} <= {}, ids {} vs {}",
            a,
            b,
            w[0].point.id,
            w[1].point.id
        );
    }
}

// =========================================================================
// Scenario 10: Multi-column ORDER BY
// =========================================================================

/// GIVEN a rich articles collection
/// WHEN querying ORDER BY category ASC, price DESC
/// THEN results are alphabetically sorted by category, then by descending
///      price within each category.
#[test]
fn test_multi_column_order_by() {
    let (_dir, db) = create_test_db();
    setup_rich_collection(&db);

    let sql = "SELECT * FROM articles ORDER BY category ASC, price DESC LIMIT 10";
    let results = execute_sql(&db, sql).expect("test: multi-column ORDER BY");

    assert_eq!(results.len(), 8, "All 8 articles should be returned");

    for w in results.windows(2) {
        let cat_a = payload_str(&w[0], "category").expect("test: category");
        let cat_b = payload_str(&w[1], "category").expect("test: category");
        assert!(
            cat_a <= cat_b,
            "Primary sort: category must be ASC: '{}' <= '{}' (ids {} vs {})",
            cat_a,
            cat_b,
            w[0].point.id,
            w[1].point.id
        );

        if cat_a == cat_b {
            let price_a = payload_f64(&w[0], "price").expect("test: price");
            let price_b = payload_f64(&w[1], "price").expect("test: price");
            assert!(
                price_a >= price_b,
                "Secondary sort: price must be DESC within '{}': {} >= {} (ids {} vs {})",
                cat_a,
                price_a,
                price_b,
                w[0].point.id,
                w[1].point.id
            );
        }
    }
}

// =========================================================================
// Scenario 11: NEAR + MATCH text + scalar filter (triple hybrid)
// =========================================================================

/// GIVEN a rich articles collection with BM25-indexed tags
/// WHEN querying NEAR `[0,1,0,0]` AND tags MATCH 'programming'
/// THEN returns hybrid (vector + BM25 RRF) fused results containing the
///      best-matching article (id=4, "rust programming language").
#[test]
fn test_near_match_bm25_with_vector_affinity() {
    let (_dir, db) = create_test_db();
    setup_rich_collection(&db);

    let sql = "SELECT * FROM articles WHERE vector NEAR $v \
               AND tags MATCH 'programming' LIMIT 5";
    let params = vector_param(&[0.0, 1.0, 0.0, 0.0]);
    let results = execute_sql_with_params(&db, sql, &params).expect("test: NEAR + MATCH hybrid");

    assert!(
        !results.is_empty(),
        "Hybrid search should return at least one result"
    );

    let ids = result_ids(&results);
    assert!(
        ids.contains(&4),
        "id=4 ('Rust Programming Guide', tags='rust programming language') \
         must appear in hybrid results, got {:?}",
        ids
    );

    for r in &results {
        assert!(
            r.score > 0.0,
            "Hybrid result must have positive fused score, got {} for id={}",
            r.score,
            r.point.id
        );
    }
}

// =========================================================================
// Scenario 12: Five scalar conditions combined
// =========================================================================

/// GIVEN a rich articles collection
/// WHEN filtering with 5 conditions: category, price range, active, year
/// THEN exactly the tech items matching all 5 conditions are returned.
#[test]
fn test_where_complex_five_conditions() {
    let (_dir, db) = create_test_db();
    setup_rich_collection(&db);

    let sql = "SELECT * FROM articles \
               WHERE category = 'tech' AND price > 20 AND price < 50 \
               AND active = true AND year >= 2022 LIMIT 10";
    let results = execute_sql(&db, sql).expect("test: 5 conditions combined");

    let ids = result_ids(&results);
    let expected: HashSet<u64> = [4, 5, 6].into_iter().collect();
    assert_eq!(
        ids, expected,
        "Should return tech ids 4, 5, 6 matching all 5 conditions, got {:?}",
        ids
    );

    for r in &results {
        let price = payload_f64(r, "price").expect("test: price");
        assert!(
            price > 20.0 && price < 50.0,
            "price must be in (20,50) for id={}",
            r.point.id
        );
    }
}

// =========================================================================
// Scenario 13: UPDATE then SELECT verifies state
// =========================================================================

/// GIVEN a rich articles collection with id=4 active=true
/// WHEN UPDATE articles SET active = false WHERE id = 4
/// THEN SELECT WHERE active = true no longer returns id=4.
#[test]
fn test_update_then_select_verifies_state() {
    let (_dir, db) = create_test_db();
    setup_rich_collection(&db);

    let before = execute_sql(&db, "SELECT * FROM articles WHERE active = true LIMIT 10")
        .expect("test: SELECT active before update");
    let before_ids = result_ids(&before);
    assert!(
        before_ids.contains(&4),
        "id=4 should be active before UPDATE, got {:?}",
        before_ids
    );

    execute_sql(&db, "UPDATE articles SET active = false WHERE id = 4;")
        .expect("test: UPDATE should succeed");

    let after = execute_sql(&db, "SELECT * FROM articles WHERE active = true LIMIT 10")
        .expect("test: SELECT active after update");
    let after_ids = result_ids(&after);
    assert!(
        !after_ids.contains(&4),
        "id=4 should NOT be active after UPDATE, got {:?}",
        after_ids
    );
}

// =========================================================================
// Scenario 14: DELETE then NEAR excludes deleted
// =========================================================================

/// GIVEN a rich articles collection with id=1 at `[1,0,0,0]`
/// WHEN DELETE FROM articles WHERE id = 1
/// THEN NEAR `[1,0,0,0]` no longer returns id=1.
#[test]
fn test_delete_then_near_excludes_deleted() {
    let (_dir, db) = create_test_db();
    setup_rich_collection(&db);

    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let before = execute_sql_with_params(
        &db,
        "SELECT * FROM articles WHERE vector NEAR $v LIMIT 3",
        &params,
    )
    .expect("test: NEAR before delete");
    assert_eq!(
        before[0].point.id, 1,
        "id=1 should be top result before DELETE"
    );

    execute_sql(&db, "DELETE FROM articles WHERE id = 1;").expect("test: DELETE should succeed");

    let after = execute_sql_with_params(
        &db,
        "SELECT * FROM articles WHERE vector NEAR $v LIMIT 5",
        &params,
    )
    .expect("test: NEAR after delete");
    let after_ids = result_ids(&after);
    assert!(
        !after_ids.contains(&1),
        "id=1 must not appear after DELETE, got {:?}",
        after_ids
    );

    for w in after.windows(2) {
        assert!(
            w[0].score >= w[1].score,
            "Post-delete scores must be non-increasing: {} >= {}",
            w[0].score,
            w[1].score
        );
    }
}

// =========================================================================
// S4-16: BM25 + vector RRF pure fusion BDD
// -------------------------------------------------------------------------
// DESIGN: "Pure" hybrid = RRF fusion of BM25 text search and vector kNN
// with no graph pattern. VelesQL exposes this pattern via the public
// `VectorCollection::hybrid_search` API, which is the canonical entry
// point for BM25+vector fusion. The surface-level alternative
// `NEAR $v AND column MATCH 'term'` routes through the same fusion
// pipeline (see `search/query/execution_paths.rs::dispatch_near_with_filter`)
// and is covered separately by `test_near_match_bm25_with_vector_affinity`.
//
// We assert against the public engine API because it yields fully
// deterministic input to the RRF formula and lets us hand-compute the
// expected fused scores from the two branch rank orders. Formula:
//   score(doc) = w_v * 1/(rank_v + k)  +  w_t * 1/(rank_t + k)
// with defaults w_v = w_t = 0.5 and k = 60 (per FusionStrategy::rrf_default).
// =========================================================================

/// Populates a "hybrid_pure" collection with 4 documents having distinct
/// vector directions and distinct text content. Vectors are aligned on the
/// unit axes so cosine ranks are unambiguous.
fn setup_hybrid_pure_collection(db: &Database) {
    db.create_vector_collection("hybrid_pure", 4, velesdb_core::DistanceMetric::Cosine)
        .expect("test: create hybrid_pure collection");
    let vc = db
        .get_vector_collection("hybrid_pure")
        .expect("test: get hybrid_pure collection");

    vc.upsert(vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(json!({"title": "rust programming language"})),
        ),
        Point::new(
            2,
            vec![0.0, 1.0, 0.0, 0.0],
            Some(json!({"title": "programming programming guide"})),
        ),
        Point::new(
            3,
            vec![0.0, 0.0, 1.0, 0.0],
            Some(json!({"title": "cooking recipes"})),
        ),
        Point::new(
            4,
            vec![0.0, 0.0, 0.0, 1.0],
            Some(json!({"title": "garden flowers"})),
        ),
    ])
    .expect("test: upsert hybrid_pure corpus");
}

/// Computes the expected RRF fused score for a given document ID from the
/// empirical rank orders of the vector and text branches.
///
/// Returns 0.0 for branches where the doc does not appear.
fn rrf_score(vector_rank: Option<usize>, text_rank: Option<usize>) -> f32 {
    const VECTOR_WEIGHT: f32 = 0.5;
    const TEXT_WEIGHT: f32 = 0.5;
    const RRF_K: f32 = 60.0;

    // Reason: empirical ranks from test fixtures are < 16; usize->f32 exact.
    #[allow(clippy::cast_precision_loss)]
    let v_term = vector_rank.map_or(0.0, |r| VECTOR_WEIGHT / (r as f32 + RRF_K));
    #[allow(clippy::cast_precision_loss)]
    let t_term = text_rank.map_or(0.0, |r| TEXT_WEIGHT / (r as f32 + RRF_K));
    v_term + t_term
}

/// Returns the position (0-indexed rank) of `id` in `ranking`, or `None`.
fn rank_of(ranking: &[u64], id: u64) -> Option<usize> {
    ranking.iter().position(|&x| x == id)
}

/// GIVEN a collection of 4 documents with distinct vectors and text payloads
/// WHEN `hybrid_search` runs with the canonical RRF settings (alpha=None =>
///      0.5/0.5 weights, rrf_k=60) for top-5
/// THEN the fused score for every returned document matches the hand-computed
///      formula `0.5/(rv+60) + 0.5/(rt+60)` using the empirical ranks from
///      each branch, and the order is fused-score descending.
///
/// This validates that k=60 and the `1/(k+rank+1)` (0-indexed +1 internally,
/// which matches `rank_as_f32 + rrf_k`) formula are actually applied.
#[test]
fn test_hybrid_rrf_fused_score_matches_hand_computed_formula() {
    let (_dir, db) = create_test_db();
    setup_hybrid_pure_collection(&db);
    let vc = db
        .get_vector_collection("hybrid_pure")
        .expect("test: get hybrid_pure");

    let query_vec = [1.0, 0.0, 0.0, 0.0];
    let query_text = "programming";

    // Snapshot per-branch ranks to derive expected RRF scores.
    // Over-fetch (k * 2) matches the hybrid_search internal gate.
    let vector_only = vc.search(&query_vec, 10).expect("test: vector-only search");
    let vector_ranking: Vec<u64> = vector_only.iter().map(|r| r.point.id).collect();

    let text_only = vc
        .text_search(query_text, 10)
        .expect("test: text-only search");
    let text_ranking: Vec<u64> = text_only.iter().map(|r| r.point.id).collect();

    // Sanity: the corpus contains the query tokens on known ids.
    assert!(
        !text_ranking.is_empty(),
        "text branch must find at least one match for 'programming'"
    );

    let fused = vc
        .hybrid_search(&query_vec, query_text, 5, None)
        .expect("test: hybrid_search should succeed");

    assert!(!fused.is_empty(), "hybrid search must return fused results");

    // Verify every fused score equals the hand-computed RRF formula.
    for hit in &fused {
        let expected = rrf_score(
            rank_of(&vector_ranking, hit.point.id),
            rank_of(&text_ranking, hit.point.id),
        );
        assert!(
            (hit.score - expected).abs() < 1e-6,
            "RRF fused score mismatch for id={}: actual={}, expected={} \
             (vector_rank={:?}, text_rank={:?})",
            hit.point.id,
            hit.score,
            expected,
            rank_of(&vector_ranking, hit.point.id),
            rank_of(&text_ranking, hit.point.id),
        );
    }

    // Verify the results are sorted by fused score descending.
    for w in fused.windows(2) {
        assert!(
            w[0].score >= w[1].score,
            "fused scores must be non-increasing: {} >= {} (ids {} vs {})",
            w[0].score,
            w[1].score,
            w[0].point.id,
            w[1].point.id
        );
    }
}

/// GIVEN a corpus where two docs have rank assignments that are a
///       permutation of each other across the two branches:
///       doc A: vector rank 0, text rank 1
///       doc B: vector rank 1, text rank 0
/// WHEN `hybrid_search` fuses the branches with equal weights
/// THEN both docs receive identical fused scores
///      (`0.5/60 + 0.5/61 = 0.5/61 + 0.5/60`), and both appear in the
///      final top-k with fused-score ties broken deterministically.
#[test]
fn test_hybrid_rrf_permuted_ranks_produce_identical_fused_scores() {
    let (_dir, db) = create_test_db();
    db.create_vector_collection("tied", 4, velesdb_core::DistanceMetric::Cosine)
        .expect("test: create tied collection");
    let vc = db.get_vector_collection("tied").expect("test: get tied");

    // Two docs, both matching the text query at TF=1. The vector ranking
    // is determined by cosine to [1, 0, 0, 0]: id=1 (exact) ranks above
    // id=2 (orthogonal, similarity 0). BM25 ranks: id=2's payload is
    // shorter, so it ranks above id=1.
    vc.upsert(vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(json!({"title": "programming rust language guide book"})),
        ),
        Point::new(
            2,
            vec![0.0, 0.0, 0.0, 1.0],
            Some(json!({"title": "programming"})),
        ),
    ])
    .expect("test: upsert tied corpus");

    let query_vec = [1.0, 0.0, 0.0, 0.0];
    let query_text = "programming";

    // Snapshot empirical per-branch ranks so the test stays robust to the
    // exact BM25 parameters used by the index.
    let vector_only = vc.search(&query_vec, 10).expect("test: vector-only search");
    let text_only = vc
        .text_search(query_text, 10)
        .expect("test: text-only search");
    let vector_rank_1 = rank_of(
        &vector_only.iter().map(|r| r.point.id).collect::<Vec<_>>(),
        1,
    );
    let vector_rank_2 = rank_of(
        &vector_only.iter().map(|r| r.point.id).collect::<Vec<_>>(),
        2,
    );
    let text_rank_1 = rank_of(&text_only.iter().map(|r| r.point.id).collect::<Vec<_>>(), 1);
    let text_rank_2 = rank_of(&text_only.iter().map(|r| r.point.id).collect::<Vec<_>>(), 2);

    // Precondition: ranks must be a permutation (swap pattern) for the
    // fused scores to tie. If BM25 tuning ever changes this, the test
    // diagnoses the setup issue instead of failing cryptically.
    assert_eq!(
        (vector_rank_1, text_rank_1),
        (Some(0), Some(1)),
        "fixture precondition: id=1 must rank top in vector, second in text"
    );
    assert_eq!(
        (vector_rank_2, text_rank_2),
        (Some(1), Some(0)),
        "fixture precondition: id=2 must rank second in vector, top in text"
    );

    let fused = vc
        .hybrid_search(&query_vec, query_text, 10, None)
        .expect("test: hybrid_search should succeed");
    let fused_map: std::collections::HashMap<u64, f32> =
        fused.iter().map(|r| (r.point.id, r.score)).collect();

    let score_1 = fused_map
        .get(&1)
        .copied()
        .expect("test: id=1 must be in fused results");
    let score_2 = fused_map
        .get(&2)
        .copied()
        .expect("test: id=2 must be in fused results");

    let expected = 0.5_f32 / 60.0 + 0.5_f32 / 61.0;
    assert!(
        (score_1 - expected).abs() < 1e-6,
        "id=1 permuted-rank RRF score mismatch: {} != {}",
        score_1,
        expected
    );
    assert!(
        (score_2 - expected).abs() < 1e-6,
        "id=2 permuted-rank RRF score mismatch: {} != {}",
        score_2,
        expected
    );
    assert!(
        (score_1 - score_2).abs() < f32::EPSILON,
        "permuted rank pair must yield identical fused scores: {score_1} vs {score_2}"
    );
}

/// GIVEN a corpus where the text query matches zero documents
/// WHEN `hybrid_search` runs
/// THEN RRF collapses to the pure vector ranking: fused scores equal
///      `0.5 / (rank + 60)` for each doc in vector order, and the result
///      order matches the vector-only ranking exactly.
#[test]
fn test_hybrid_rrf_empty_text_side_collapses_to_vector_ranking() {
    let (_dir, db) = create_test_db();
    setup_hybrid_pure_collection(&db);
    let vc = db
        .get_vector_collection("hybrid_pure")
        .expect("test: get hybrid_pure");

    let query_vec = [1.0, 0.0, 0.0, 0.0];
    // This term does not appear in any payload.
    let query_text = "zzzzzzzzzzzzzzzz_no_match";

    let vector_only = vc.search(&query_vec, 10).expect("test: vector-only search");
    let fused = vc
        .hybrid_search(&query_vec, query_text, 4, None)
        .expect("test: hybrid_search should succeed");

    assert!(
        !fused.is_empty(),
        "empty text side must not zero out the vector branch"
    );

    // Order must match the vector-only ranking for the prefix of length |fused|.
    let fused_ids: Vec<u64> = fused.iter().map(|r| r.point.id).collect();
    let vector_prefix: Vec<u64> = vector_only
        .iter()
        .take(fused_ids.len())
        .map(|r| r.point.id)
        .collect();
    assert_eq!(
        fused_ids, vector_prefix,
        "empty BM25 side must collapse to vector ranking"
    );

    // Every fused score must equal 0.5 / (rank + 60) with no text contribution.
    for (rank, hit) in fused.iter().enumerate() {
        let expected = rrf_score(Some(rank), None);
        assert!(
            (hit.score - expected).abs() < 1e-6,
            "rank {} id {} fused score {} != expected vector-only {}",
            rank,
            hit.point.id,
            hit.score,
            expected,
        );
    }
}
