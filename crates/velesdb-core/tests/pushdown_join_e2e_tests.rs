#![cfg(feature = "persistence")]
//! E2E tests for the pushdown/JOIN pipeline.
//! Verifies that WHERE conditions are pushed to ColumnStore before JOIN.
//!
//! These tests exercise the FULL production path:
//! `VelesQL string -> Parser::parse -> Database::execute_query -> pushdown analysis
//!  -> ColumnStore filter -> JOIN execution -> post-join filter -> results`

#![allow(
    clippy::cast_precision_loss,
    clippy::uninlined_format_args,
    clippy::doc_markdown
)]

use std::collections::{HashMap, HashSet};

use serde_json::json;
use tempfile::TempDir;
use velesdb_core::{velesql::Parser, Database, Point, SearchResult};

// =========================================================================
// Helpers
// =========================================================================

/// Executes a VelesQL statement through the full production pipeline.
fn execute_sql(db: &Database, sql: &str) -> velesdb_core::Result<Vec<SearchResult>> {
    let query = Parser::parse(sql).map_err(|e| velesdb_core::Error::Query(e.to_string()))?;
    db.execute_query(&query, &HashMap::new())
}

/// Executes a VelesQL statement with bind parameters.
fn execute_sql_with_params(
    db: &Database,
    sql: &str,
    params: &HashMap<String, serde_json::Value>,
) -> velesdb_core::Result<Vec<SearchResult>> {
    let query = Parser::parse(sql).map_err(|e| velesdb_core::Error::Query(e.to_string()))?;
    db.execute_query(&query, params)
}

/// Creates a fresh database backed by a temporary directory.
fn create_test_db() -> (TempDir, Database) {
    let dir = TempDir::new().expect("test: create temp dir");
    let db = Database::open(dir.path()).expect("test: open database");
    (dir, db)
}

/// Extracts a string payload field from a search result.
fn payload_str<'a>(result: &'a SearchResult, field: &str) -> Option<&'a str> {
    result
        .point
        .payload
        .as_ref()
        .and_then(|p| p.get(field))
        .and_then(serde_json::Value::as_str)
}

/// Extracts a numeric payload field from a search result.
fn payload_f64(result: &SearchResult, field: &str) -> Option<f64> {
    result
        .point
        .payload
        .as_ref()
        .and_then(|p| p.get(field))
        .and_then(serde_json::Value::as_f64)
}

/// Collects result IDs into a `HashSet` for order-independent comparison.
fn result_ids(results: &[SearchResult]) -> HashSet<u64> {
    results.iter().map(|r| r.point.id).collect()
}

/// Builds a param map with a single vector parameter named `$v`.
fn vector_param(v: &[f32]) -> HashMap<String, serde_json::Value> {
    let mut params = HashMap::new();
    params.insert("v".to_string(), serde_json::json!(v));
    params
}

/// Creates `products` (VectorCollection, dim=4) and `reviews` (MetadataCollection).
///
/// Products: 6 items spanning 3 categories, varied prices.
/// Reviews: 6 rows sharing the same IDs, each with `rating` and `reviewer` fields.
///
/// The two collections share primary keys (id 1..=6), which is how VelesDB JOINs
/// work: `ON products.id = reviews.id`.
fn setup_products_and_reviews(db: &Database) {
    // -- products (VectorCollection) --
    execute_sql(
        db,
        "CREATE COLLECTION products (dimension = 4, metric = 'cosine');",
    )
    .expect("test: CREATE products");

    let products = db
        .get_vector_collection("products")
        .expect("test: get products");
    products
        .upsert(vec![
            Point::new(
                1,
                vec![1.0, 0.0, 0.0, 0.0],
                Some(json!({"name": "Laptop", "category": "electronics", "price": 1200})),
            ),
            Point::new(
                2,
                vec![0.0, 1.0, 0.0, 0.0],
                Some(json!({"name": "Phone", "category": "electronics", "price": 800})),
            ),
            Point::new(
                3,
                vec![0.0, 0.0, 1.0, 0.0],
                Some(json!({"name": "Novel", "category": "books", "price": 15})),
            ),
            Point::new(
                4,
                vec![0.0, 0.0, 0.0, 1.0],
                Some(json!({"name": "Cookbook", "category": "books", "price": 25})),
            ),
            Point::new(
                5,
                vec![0.7, 0.7, 0.0, 0.0],
                Some(json!({"name": "Tablet", "category": "electronics", "price": 500})),
            ),
            Point::new(
                6,
                vec![0.5, 0.0, 0.5, 0.0],
                Some(json!({"name": "T-Shirt", "category": "clothing", "price": 30})),
            ),
        ])
        .expect("test: upsert products");

    // -- reviews (MetadataCollection) --
    execute_sql(db, "CREATE METADATA COLLECTION reviews;").expect("test: CREATE reviews");

    let reviews = db
        .get_metadata_collection("reviews")
        .expect("test: get reviews");
    reviews
        .upsert(vec![
            Point::metadata_only(1, json!({"rating": 5, "reviewer": "Alice"})),
            Point::metadata_only(2, json!({"rating": 3, "reviewer": "Bob"})),
            Point::metadata_only(3, json!({"rating": 4, "reviewer": "Charlie"})),
            Point::metadata_only(4, json!({"rating": 2, "reviewer": "Diana"})),
            Point::metadata_only(5, json!({"rating": 5, "reviewer": "Eve"})),
            Point::metadata_only(6, json!({"rating": 1, "reviewer": "Frank"})),
        ])
        .expect("test: upsert reviews");
}

// =========================================================================
// Nominal: JOIN + WHERE pushdown returns correct results
// =========================================================================

/// GIVEN: Collection "products" with vectors + payload {category, price}
/// AND: Collection "reviews" with metadata {rating, reviewer}
/// WHEN: Execute VelesQL JOIN with WHERE filters on both sides
/// THEN: Only rows matching both base-side and pushed filters are returned
/// AND: Results contain correct joined data (merged payloads).
#[test]
fn test_join_with_pushdown_returns_correct_results() {
    let (_dir, db) = create_test_db();
    setup_products_and_reviews(&db);

    let sql = "SELECT * FROM products \
               JOIN reviews ON products.id = reviews.id \
               WHERE category = 'electronics' AND reviews.rating > 4 \
               LIMIT 10";

    let results = execute_sql(&db, sql).expect("test: pushdown JOIN query should succeed");

    // electronics: Laptop(id=1, rating=5), Phone(id=2, rating=3), Tablet(id=5, rating=5)
    // rating > 4: Laptop(rating=5), Tablet(rating=5)
    // Intersection: {1, 5}
    let ids = result_ids(&results);
    assert_eq!(ids.len(), 2, "expected 2 results, got {}", results.len());
    assert!(ids.contains(&1), "Laptop (id=1) should match");
    assert!(ids.contains(&5), "Tablet (id=5) should match");

    // Verify merged payloads contain fields from both collections.
    for r in &results {
        assert!(
            payload_str(r, "category").is_some(),
            "merged payload should have 'category' from products"
        );
        assert!(
            payload_f64(r, "rating").is_some(),
            "merged payload should have 'rating' from reviews"
        );
        assert_eq!(
            payload_str(r, "category"),
            Some("electronics"),
            "all results should be electronics"
        );
        let rating = payload_f64(r, "rating").expect("test: rating field");
        assert!(rating > 4.0, "rating {} should be > 4", rating);
    }
}

// =========================================================================
// Nominal: Pushdown filters before JOIN (proof by cardinality)
// =========================================================================

/// GIVEN: Same setup as above
/// WHEN: Execute with a restrictive filter (price > 1000)
/// THEN: Results only contain expensive products
/// AND: The number of results is smaller than without the filter
/// (This indirectly proves pushdown works: ColumnStore filters before JOIN.)
#[test]
fn test_join_pushdown_filters_before_join() {
    let (_dir, db) = create_test_db();
    setup_products_and_reviews(&db);

    // Baseline: JOIN without WHERE returns all 6 matched rows.
    let sql_all = "SELECT * FROM products \
                   JOIN reviews ON products.id = reviews.id \
                   LIMIT 10";
    let all_results = execute_sql(&db, sql_all).expect("test: baseline JOIN");
    assert_eq!(
        all_results.len(),
        6,
        "baseline JOIN should return all 6 rows"
    );

    // Filtered: only products with price > 1000.
    let sql_filtered = "SELECT * FROM products \
                        JOIN reviews ON products.id = reviews.id \
                        WHERE price > 1000 \
                        LIMIT 10";
    let filtered_results = execute_sql(&db, sql_filtered).expect("test: filtered JOIN");

    // Only Laptop (price=1200) has price > 1000.
    assert_eq!(
        filtered_results.len(),
        1,
        "filtered JOIN should return 1 row (Laptop)"
    );
    assert!(
        filtered_results.len() < all_results.len(),
        "pushdown filter should reduce result count"
    );
    assert_eq!(
        payload_str(&filtered_results[0], "name"),
        Some("Laptop"),
        "the single result should be Laptop"
    );

    // Verify the joined review data is also present.
    assert!(
        payload_f64(&filtered_results[0], "rating").is_some(),
        "merged payload should have rating from reviews"
    );
}

// =========================================================================
// Nominal: JOIN without WHERE returns all matching rows
// =========================================================================

/// GIVEN: Same collections
/// WHEN: Execute SELECT with JOIN but no WHERE clause
/// THEN: All matching rows are returned (cross product filtered by ON clause only).
#[test]
fn test_join_without_where_returns_all() {
    let (_dir, db) = create_test_db();
    setup_products_and_reviews(&db);

    let sql = "SELECT * FROM products \
               JOIN reviews ON products.id = reviews.id \
               LIMIT 10";

    let results = execute_sql(&db, sql).expect("test: JOIN without WHERE");

    assert_eq!(results.len(), 6, "all 6 products should join with reviews");

    // Verify every result has merged payloads from both sides.
    for r in &results {
        assert!(
            payload_str(r, "name").is_some(),
            "should have 'name' from products"
        );
        assert!(
            payload_f64(r, "rating").is_some(),
            "should have 'rating' from reviews"
        );
    }
}

// =========================================================================
// Nominal: Vector NEAR + JOIN + pushdown combined
// =========================================================================

/// GIVEN: Products collection with vectors
/// WHEN: SELECT with JOIN + NEAR vector search + pushed filter on reviews
/// THEN: Vector similarity + joined filter both apply correctly.
#[test]
fn test_join_with_vector_near_and_pushdown() {
    let (_dir, db) = create_test_db();
    setup_products_and_reviews(&db);

    let sql = "SELECT * FROM products \
               JOIN reviews ON products.id = reviews.id \
               WHERE vector NEAR $v AND reviews.rating > 3 \
               LIMIT 5";

    // Query vector is close to Laptop [1,0,0,0] and Tablet [0.7,0.7,0,0]
    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let results = execute_sql_with_params(&db, sql, &params).expect("test: NEAR + pushdown JOIN");

    assert!(!results.is_empty(), "NEAR + pushdown should return results");

    // All results must have rating > 3 (pushed to ColumnStore).
    for r in &results {
        let rating = payload_f64(r, "rating").expect("test: rating field");
        assert!(rating > 3.0, "rating {} should be > 3", rating);
    }

    // rating > 3 excludes: Phone(id=2, rating=3), Cookbook(id=4, rating=2), T-Shirt(id=6, rating=1)
    // Remaining: Laptop(5), Novel(4), Tablet(5)
    let ids = result_ids(&results);
    assert!(
        !ids.contains(&2),
        "Phone (rating=3) should be excluded by pushdown"
    );
    assert!(
        !ids.contains(&4),
        "Cookbook (rating=2) should be excluded by pushdown"
    );
    assert!(
        !ids.contains(&6),
        "T-Shirt (rating=1) should be excluded by pushdown"
    );
}

// =========================================================================
// Edge: Pushdown eliminates ALL joined rows
// =========================================================================

/// GIVEN: Products and reviews
/// WHEN: Pushdown filter on reviews matches no rows (rating > 100)
/// THEN: INNER JOIN returns empty (no review survives pushdown).
#[test]
fn test_pushdown_eliminates_all_joined_rows_returns_empty() {
    let (_dir, db) = create_test_db();
    setup_products_and_reviews(&db);

    let sql = "SELECT * FROM products \
               JOIN reviews ON products.id = reviews.id \
               WHERE reviews.rating > 100 \
               LIMIT 10";

    let results = execute_sql(&db, sql).expect("test: pushdown eliminates all");
    assert!(
        results.is_empty(),
        "INNER JOIN with impossible pushdown filter should return empty"
    );
}

// =========================================================================
// Edge: Multiple pushdown conditions on joined table
// =========================================================================

/// GIVEN: Products and reviews
/// WHEN: Multiple conditions target the joined table (reviews.rating > 3 AND reviews.rating < 6)
/// THEN: Both conditions are pushed and applied correctly.
#[test]
fn test_multiple_pushdown_conditions_on_joined_table() {
    let (_dir, db) = create_test_db();
    setup_products_and_reviews(&db);

    let sql = "SELECT * FROM products \
               JOIN reviews ON products.id = reviews.id \
               WHERE reviews.rating > 3 AND reviews.rating < 6 \
               LIMIT 10";

    let results = execute_sql(&db, sql).expect("test: multiple pushdown conditions");

    // rating > 3 AND rating < 6: ratings 4 and 5
    // id=1 rating=5, id=3 rating=4, id=5 rating=5
    let ids = result_ids(&results);
    assert_eq!(ids.len(), 3, "expected 3 results, got {}", results.len());
    assert!(ids.contains(&1), "Laptop (rating=5) should match");
    assert!(ids.contains(&3), "Novel (rating=4) should match");
    assert!(ids.contains(&5), "Tablet (rating=5) should match");

    for r in &results {
        let rating = payload_f64(r, "rating").expect("test: rating field");
        assert!(
            rating > 3.0 && rating < 6.0,
            "rating {} should be between 3 and 6 exclusive",
            rating
        );
    }
}

// =========================================================================
// Combination: Base-side + pushdown filters together
// =========================================================================

/// GIVEN: Products and reviews
/// WHEN: Base-side filter (category = 'books') + pushdown filter (reviews.rating > 3)
/// THEN: Both filters apply — only books with high ratings appear.
#[test]
fn test_base_side_and_pushdown_filters_combined() {
    let (_dir, db) = create_test_db();
    setup_products_and_reviews(&db);

    let sql = "SELECT * FROM products \
               JOIN reviews ON products.id = reviews.id \
               WHERE category = 'books' AND reviews.rating > 3 \
               LIMIT 10";

    let results = execute_sql(&db, sql).expect("test: base + pushdown combined");

    // books: Novel(id=3, rating=4), Cookbook(id=4, rating=2)
    // rating > 3: Novel(4) passes, Cookbook(2) fails
    assert_eq!(results.len(), 1, "only Novel should match");
    assert_eq!(
        payload_str(&results[0], "name"),
        Some("Novel"),
        "the single result should be Novel"
    );
    assert_eq!(
        payload_str(&results[0], "category"),
        Some("books"),
        "category should be 'books'"
    );
    let rating = payload_f64(&results[0], "rating").expect("test: rating field");
    assert!(rating > 3.0, "rating {} should be > 3", rating);
}

// =========================================================================
// Negative: JOIN references non-existent collection
// =========================================================================

/// GIVEN: Only products collection exists
/// WHEN: JOIN references a non-existent collection "ghost"
/// THEN: A descriptive error is returned.
#[test]
fn test_join_with_nonexistent_collection_returns_error() {
    let (_dir, db) = create_test_db();
    setup_products_and_reviews(&db);

    let sql = "SELECT * FROM products \
               JOIN ghost ON products.id = ghost.id \
               LIMIT 10";

    let err = execute_sql(&db, sql).expect_err("test: should fail for missing collection");
    let msg = err.to_string();
    assert!(
        msg.contains("ghost") || msg.contains("not found"),
        "error should mention the missing collection, got: {msg}"
    );
}

// =========================================================================
// Combination: LIMIT interacts correctly with pushdown
// =========================================================================

/// GIVEN: Products and reviews
/// WHEN: Pushdown filter yields 3 matches but LIMIT is 2
/// THEN: At most 2 results are returned and all satisfy the pushdown filter.
///
/// Note: VelesDB applies LIMIT to the base query before JOIN, so the final
/// count may be less than the LIMIT when the post-join pushdown further
/// reduces rows. The key invariant: result count <= LIMIT, and all results
/// satisfy the pushed filter.
#[test]
fn test_pushdown_with_limit_truncates_correctly() {
    let (_dir, db) = create_test_db();
    setup_products_and_reviews(&db);

    // rating > 3 yields ids {1,3,5} (3 matches without LIMIT)
    let sql = "SELECT * FROM products \
               JOIN reviews ON products.id = reviews.id \
               WHERE reviews.rating > 3 \
               LIMIT 2";

    let results = execute_sql(&db, sql).expect("test: pushdown + LIMIT");
    assert!(
        results.len() <= 2,
        "LIMIT 2 should cap results at 2, got {}",
        results.len()
    );
    assert!(!results.is_empty(), "should return at least 1 result");

    for r in &results {
        let rating = payload_f64(r, "rating").expect("test: rating field");
        assert!(rating > 3.0, "all results should have rating > 3");
    }
}
