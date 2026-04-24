//! BDD-style end-to-end tests for Array CONTAINS filter (Issue #510).
//!
//! Each scenario follows GIVEN (setup data) → WHEN (execute SQL) → THEN (verify
//! results). Tests exercise the full pipeline: SQL string → `Parser::parse()`
//! → `Database::execute_query()` → verify returned `SearchResult` values.

#![allow(clippy::doc_link_with_quotes)]

use std::collections::HashSet;

use serde_json::json;
use velesdb_core::{Database, Point};

use super::helpers::{create_test_db, execute_sql, result_ids};

// =========================================================================
// Module-specific setup
// =========================================================================

/// Populate a `hotels` collection with array-valued payload fields for CONTAINS testing.
///
/// | id | amenities                    | tags              | rating | city      |
/// |----|------------------------------|-------------------|--------|-----------|
/// | 1  | ["pool", "gym", "spa"]       | ["luxury", "5*"]  | 4.8    | Paris     |
/// | 2  | ["wifi", "parking"]          | ["budget"]        | 3.2    | London    |
/// | 3  | ["pool", "wifi", "bar"]      | ["family"]        | 4.1    | Paris     |
/// | 4  | []                           | ["budget", "new"] | 2.9    | Berlin    |
/// | 5  | null (no amenities field)    | null              | 4.5    | Tokyo     |
/// | 6  | ["gym"]                      | ["luxury"]        | 3.8    | London    |
/// | 7  | ["pool", "gym", "spa", "wifi"] | ["luxury", "5*", "new"] | 4.9 | Dubai |
fn setup_hotels_collection(db: &Database) {
    execute_sql(
        db,
        "CREATE COLLECTION hotels (dimension = 4, metric = 'cosine');",
    )
    .expect("test: CREATE hotels");

    let vc = db
        .get_vector_collection("hotels")
        .expect("test: get hotels collection");

    vc.upsert(vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(json!({
                "amenities": ["pool", "gym", "spa"],
                "tags": ["luxury", "5*"],
                "rating": 4.8,
                "city": "Paris"
            })),
        ),
        Point::new(
            2,
            vec![0.0, 1.0, 0.0, 0.0],
            Some(json!({
                "amenities": ["wifi", "parking"],
                "tags": ["budget"],
                "rating": 3.2,
                "city": "London"
            })),
        ),
        Point::new(
            3,
            vec![0.0, 0.0, 1.0, 0.0],
            Some(json!({
                "amenities": ["pool", "wifi", "bar"],
                "tags": ["family"],
                "rating": 4.1,
                "city": "Paris"
            })),
        ),
        Point::new(
            4,
            vec![0.0, 0.0, 0.0, 1.0],
            Some(json!({
                "amenities": [],
                "tags": ["budget", "new"],
                "rating": 2.9,
                "city": "Berlin"
            })),
        ),
        Point::new(
            5,
            vec![0.5, 0.5, 0.0, 0.0],
            Some(json!({
                "rating": 4.5,
                "city": "Tokyo"
            })),
        ),
        Point::new(
            6,
            vec![0.5, 0.0, 0.5, 0.0],
            Some(json!({
                "amenities": ["gym"],
                "tags": ["luxury"],
                "rating": 3.8,
                "city": "London"
            })),
        ),
        Point::new(
            7,
            vec![0.0, 0.5, 0.0, 0.5],
            Some(json!({
                "amenities": ["pool", "gym", "spa", "wifi"],
                "tags": ["luxury", "5*", "new"],
                "rating": 4.9,
                "city": "Dubai"
            })),
        ),
    ])
    .expect("test: upsert hotels");
}

// =========================================================================
// Nominal: CONTAINS single value
// =========================================================================

/// GIVEN hotels with array amenities
/// WHEN querying WHERE amenities CONTAINS 'pool'
/// THEN returns hotels 1, 3, 7 (all with pool)
#[test]
fn test_contains_single_value_returns_matching_rows() {
    let (_dir, db) = create_test_db();
    setup_hotels_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM hotels WHERE amenities CONTAINS 'pool' LIMIT 10;",
    )
    .expect("CONTAINS single query");

    let ids = result_ids(&results);
    assert_eq!(ids, HashSet::from([1, 3, 7]), "Hotels with pool: 1, 3, 7");
}

// =========================================================================
// Nominal: CONTAINS ANY
// =========================================================================

/// GIVEN hotels with array amenities
/// WHEN querying WHERE amenities CONTAINS ANY ('spa', 'bar')
/// THEN returns hotels 1, 3, 7 (spa: 1,7; bar: 3)
#[test]
fn test_contains_any_returns_rows_with_at_least_one_match() {
    let (_dir, db) = create_test_db();
    setup_hotels_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM hotels WHERE amenities CONTAINS ANY ('spa', 'bar') LIMIT 10;",
    )
    .expect("CONTAINS ANY query");

    let ids = result_ids(&results);
    assert_eq!(ids, HashSet::from([1, 3, 7]), "Hotels with spa or bar");
}

// =========================================================================
// Nominal: CONTAINS ALL
// =========================================================================

/// GIVEN hotels with array amenities
/// WHEN querying WHERE amenities CONTAINS ALL ('pool', 'gym')
/// THEN returns hotels 1, 7 (both have pool AND gym)
#[test]
fn test_contains_all_returns_rows_with_every_value() {
    let (_dir, db) = create_test_db();
    setup_hotels_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM hotels WHERE amenities CONTAINS ALL ('pool', 'gym') LIMIT 10;",
    )
    .expect("CONTAINS ALL query");

    let ids = result_ids(&results);
    assert_eq!(ids, HashSet::from([1, 7]), "Hotels with pool AND gym");
}

// =========================================================================
// Edge: empty array CONTAINS → no match
// =========================================================================

/// GIVEN hotel 4 has amenities = []
/// WHEN querying WHERE amenities CONTAINS 'pool'
/// THEN hotel 4 is NOT in results
#[test]
fn test_contains_on_empty_array_returns_no_results() {
    let (_dir, db) = create_test_db();
    setup_hotels_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM hotels WHERE amenities CONTAINS 'pool' LIMIT 10;",
    )
    .expect("CONTAINS on empty array");

    let ids = result_ids(&results);
    assert!(!ids.contains(&4), "Empty array hotel should not match");
}

// =========================================================================
// Edge: null/missing array CONTAINS → excluded
// =========================================================================

/// GIVEN hotel 5 has no amenities field
/// WHEN querying WHERE amenities CONTAINS 'pool'
/// THEN hotel 5 is NOT in results
#[test]
fn test_contains_on_null_array_excludes_row() {
    let (_dir, db) = create_test_db();
    setup_hotels_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM hotels WHERE amenities CONTAINS 'wifi' LIMIT 10;",
    )
    .expect("CONTAINS on null array");

    let ids = result_ids(&results);
    assert!(
        !ids.contains(&5),
        "Null/missing array hotel should not match"
    );
}

// =========================================================================
// Edge: single-element array
// =========================================================================

/// GIVEN hotel 6 has amenities = ["gym"]
/// WHEN querying WHERE amenities CONTAINS 'gym'
/// THEN hotel 6 is in results
#[test]
fn test_contains_on_single_element_array() {
    let (_dir, db) = create_test_db();
    setup_hotels_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM hotels WHERE amenities CONTAINS 'gym' LIMIT 10;",
    )
    .expect("CONTAINS on single-element array");

    let ids = result_ids(&results);
    assert!(ids.contains(&6), "Single-element array hotel should match");
    // Also 1 and 7 have gym
    assert_eq!(ids, HashSet::from([1, 6, 7]));
}

// =========================================================================
// Edge: array with duplicates (hotel 7 has "pool" once, not duplicated,
// but test CONTAINS ALL with repeated value)
// =========================================================================

/// GIVEN hotels with various amenities
/// WHEN querying CONTAINS ANY with values that overlap multiple hotels
/// THEN returns the union correctly
#[test]
fn test_contains_any_with_overlapping_values() {
    let (_dir, db) = create_test_db();
    setup_hotels_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM hotels WHERE amenities CONTAINS ANY ('parking', 'gym') LIMIT 10;",
    )
    .expect("CONTAINS ANY overlapping");

    let ids = result_ids(&results);
    // parking: 2, gym: 1, 6, 7
    assert_eq!(ids, HashSet::from([1, 2, 6, 7]));
}

// =========================================================================
// Negative: CONTAINS on non-array scalar field → empty (array semantics)
// =========================================================================

/// GIVEN hotels with scalar city field (not an array)
/// WHEN querying WHERE city CONTAINS 'Paris'
/// THEN returns empty because city is a string, not an array
/// Note: VelesQL CONTAINS uses array containment semantics.
/// For substring matching, use LIKE '%Par%' instead.
#[test]
fn test_contains_on_scalar_string_field_returns_empty() {
    let (_dir, db) = create_test_db();
    setup_hotels_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM hotels WHERE city CONTAINS 'Paris' LIMIT 10;",
    )
    .expect("CONTAINS on scalar string");

    // city is a string, not an array → ArrayContains on non-array returns empty
    assert!(
        results.is_empty(),
        "CONTAINS on scalar string should return empty"
    );
}

// =========================================================================
// Negative: CONTAINS on non-existent field → empty
// =========================================================================

/// GIVEN hotels collection
/// WHEN querying WHERE nonexistent CONTAINS 'value'
/// THEN returns empty results
#[test]
fn test_contains_on_nonexistent_field_returns_empty() {
    let (_dir, db) = create_test_db();
    setup_hotels_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM hotels WHERE nonexistent CONTAINS 'value' LIMIT 10;",
    )
    .expect("CONTAINS on nonexistent field");

    assert!(results.is_empty(), "Non-existent field should return empty");
}

// =========================================================================
// Combination: CONTAINS AND scalar filter
// =========================================================================

/// GIVEN hotels with amenities and rating
/// WHEN querying WHERE amenities CONTAINS 'pool' AND rating > 4.5
/// THEN returns hotels with pool AND rating > 4.5 → 1 (4.8), 7 (4.9)
#[test]
fn test_contains_combined_with_scalar_filter() {
    let (_dir, db) = create_test_db();
    setup_hotels_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM hotels WHERE amenities CONTAINS 'pool' AND rating > 4.5 LIMIT 10;",
    )
    .expect("CONTAINS AND scalar filter");

    let ids = result_ids(&results);
    assert_eq!(ids, HashSet::from([1, 7]), "Pool + rating > 4.5");
}

// =========================================================================
// Combination: CONTAINS with equality filter
// =========================================================================

/// GIVEN hotels with amenities and city
/// WHEN querying WHERE amenities CONTAINS 'wifi' AND city = 'Paris'
/// THEN returns hotel 3 (wifi + Paris)
#[test]
fn test_contains_combined_with_equality_filter() {
    let (_dir, db) = create_test_db();
    setup_hotels_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM hotels WHERE amenities CONTAINS 'wifi' AND city = 'Paris' LIMIT 10;",
    )
    .expect("CONTAINS AND equality");

    let ids = result_ids(&results);
    assert_eq!(ids, HashSet::from([3]), "wifi + Paris = hotel 3");
}

// =========================================================================
// Combination: CONTAINS ALL with scalar filter
// =========================================================================

/// GIVEN hotels with amenities and city
/// WHEN querying WHERE amenities CONTAINS ALL ('pool', 'spa') AND city = 'Paris'
/// THEN returns hotel 1 (pool+spa+Paris)
#[test]
fn test_contains_all_combined_with_equality() {
    let (_dir, db) = create_test_db();
    setup_hotels_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM hotels WHERE amenities CONTAINS ALL ('pool', 'spa') AND city = 'Paris' LIMIT 10;",
    )
    .expect("CONTAINS ALL AND equality");

    let ids = result_ids(&results);
    assert_eq!(ids, HashSet::from([1]), "pool+spa+Paris = hotel 1");
}

// =========================================================================
// Combination: CONTAINS ANY on tags field
// =========================================================================

/// GIVEN hotels with tags array
/// WHEN querying WHERE tags CONTAINS ANY ('luxury', 'family')
/// THEN returns hotels 1, 3, 6, 7
#[test]
fn test_contains_any_on_different_array_field() {
    let (_dir, db) = create_test_db();
    setup_hotels_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM hotels WHERE tags CONTAINS ANY ('luxury', 'family') LIMIT 10;",
    )
    .expect("CONTAINS ANY on tags");

    let ids = result_ids(&results);
    assert_eq!(ids, HashSet::from([1, 3, 6, 7]), "luxury or family tags");
}

// =========================================================================
// Combination: CONTAINS with ORDER BY
// =========================================================================

/// GIVEN hotels with amenities and rating
/// WHEN querying WHERE amenities CONTAINS 'pool' ORDER BY rating DESC
/// THEN returns hotels 7, 1, 3 in descending rating order
#[test]
fn test_contains_with_order_by() {
    let (_dir, db) = create_test_db();
    setup_hotels_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM hotels WHERE amenities CONTAINS 'pool' ORDER BY rating DESC LIMIT 10;",
    )
    .expect("CONTAINS with ORDER BY");

    let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    assert_eq!(ids, vec![7, 1, 3], "Ordered by rating DESC: 4.9, 4.8, 4.1");
}

// =========================================================================
// Parser: CONTAINS single value
// =========================================================================

/// GIVEN a valid CONTAINS single expression
/// WHEN parsing
/// THEN produces correct AST
#[test]
fn test_parse_contains_single_value() {
    use velesdb_core::velesql::{Condition, ContainsMode, Parser};

    let query = Parser::parse("SELECT * FROM c WHERE tags CONTAINS 'pool' LIMIT 5;")
        .expect("parse CONTAINS single");

    let condition = query.select.where_clause.expect("WHERE clause");
    if let Condition::Contains(cc) = condition {
        assert_eq!(cc.column, "tags");
        assert_eq!(cc.mode, ContainsMode::Single);
        assert_eq!(cc.values.len(), 1);
    } else {
        panic!("Expected Condition::Contains, got {condition:?}");
    }
}

// =========================================================================
// Parser: CONTAINS ANY
// =========================================================================

/// GIVEN a valid CONTAINS ANY expression
/// WHEN parsing
/// THEN produces correct AST with Any mode
#[test]
fn test_parse_contains_any_values() {
    use velesdb_core::velesql::{Condition, ContainsMode, Parser};

    let query = Parser::parse("SELECT * FROM c WHERE tags CONTAINS ANY ('pool', 'gym') LIMIT 5;")
        .expect("parse CONTAINS ANY");

    let condition = query.select.where_clause.expect("WHERE clause");
    if let Condition::Contains(cc) = condition {
        assert_eq!(cc.column, "tags");
        assert_eq!(cc.mode, ContainsMode::Any);
        assert_eq!(cc.values.len(), 2);
    } else {
        panic!("Expected Condition::Contains, got {condition:?}");
    }
}

// =========================================================================
// Parser: CONTAINS ALL
// =========================================================================

/// GIVEN a valid CONTAINS ALL expression
/// WHEN parsing
/// THEN produces correct AST with All mode
#[test]
fn test_parse_contains_all_values() {
    use velesdb_core::velesql::{Condition, ContainsMode, Parser};

    let query =
        Parser::parse("SELECT * FROM c WHERE tags CONTAINS ALL ('pool', 'gym', 'spa') LIMIT 5;")
            .expect("parse CONTAINS ALL");

    let condition = query.select.where_clause.expect("WHERE clause");
    if let Condition::Contains(cc) = condition {
        assert_eq!(cc.column, "tags");
        assert_eq!(cc.mode, ContainsMode::All);
        assert_eq!(cc.values.len(), 3);
    } else {
        panic!("Expected Condition::Contains, got {condition:?}");
    }
}

// =========================================================================
// Parser: CONTAINS combined with AND
// =========================================================================

/// GIVEN a CONTAINS AND comparison expression
/// WHEN parsing
/// THEN produces And(Contains, Comparison) AST
#[test]
fn test_parse_contains_and_comparison() {
    use velesdb_core::velesql::{Condition, Parser};

    let query =
        Parser::parse("SELECT * FROM c WHERE tags CONTAINS 'pool' AND rating > 4.0 LIMIT 5;")
            .expect("parse CONTAINS AND comparison");

    let condition = query.select.where_clause.expect("WHERE clause");
    assert!(
        matches!(condition, Condition::And(_, _)),
        "Expected And condition, got {condition:?}"
    );
}

// =========================================================================
// Parser: malformed CONTAINS → error
// =========================================================================

/// GIVEN a malformed CONTAINS expression (missing value)
/// WHEN parsing
/// THEN returns a parse error
#[test]
fn test_parse_malformed_contains_returns_error() {
    use velesdb_core::velesql::Parser;

    // Missing value after CONTAINS
    let result = Parser::parse("SELECT * FROM c WHERE tags CONTAINS LIMIT 5;");
    assert!(result.is_err(), "Malformed CONTAINS should fail to parse");
}

// =========================================================================
// CONTAINS with integer values in array
// =========================================================================

/// GIVEN hotels with numeric array in payload
/// WHEN querying WHERE scores CONTAINS 10
/// THEN returns matching rows
#[test]
fn test_contains_with_integer_array() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION items (dimension = 2, metric = 'cosine');",
    )
    .expect("CREATE items");

    let vc = db.get_vector_collection("items").expect("get items");
    vc.upsert(vec![
        Point::new(1, vec![1.0, 0.0], Some(json!({"scores": [10, 20, 30]}))),
        Point::new(2, vec![0.0, 1.0], Some(json!({"scores": [40, 50]}))),
        Point::new(3, vec![0.5, 0.5], Some(json!({"scores": [10, 50]}))),
    ])
    .expect("upsert items");

    let results = execute_sql(
        &db,
        "SELECT * FROM items WHERE scores CONTAINS 10 LIMIT 10;",
    )
    .expect("CONTAINS integer");

    let ids = result_ids(&results);
    assert_eq!(ids, HashSet::from([1, 3]), "Items with score 10");
}

// =========================================================================
// CONTAINS ALL with no matching rows
// =========================================================================

/// GIVEN hotels collection
/// WHEN querying CONTAINS ALL with values no single hotel has all of
/// THEN returns empty
#[test]
fn test_contains_all_no_match_returns_empty() {
    let (_dir, db) = create_test_db();
    setup_hotels_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM hotels WHERE amenities CONTAINS ALL ('pool', 'parking') LIMIT 10;",
    )
    .expect("CONTAINS ALL no match");

    assert!(results.is_empty(), "No hotel has both pool and parking");
}

// =========================================================================
// CONTAINS ANY with single value (equivalent to CONTAINS single)
// =========================================================================

/// GIVEN hotels collection
/// WHEN querying CONTAINS ANY with a single value
/// THEN returns same as CONTAINS single
#[test]
fn test_contains_any_single_value_equivalent_to_contains() {
    let (_dir, db) = create_test_db();
    setup_hotels_collection(&db);

    let results_any = execute_sql(
        &db,
        "SELECT * FROM hotels WHERE amenities CONTAINS ANY ('spa') LIMIT 10;",
    )
    .expect("CONTAINS ANY single");

    let results_single = execute_sql(
        &db,
        "SELECT * FROM hotels WHERE amenities CONTAINS 'spa' LIMIT 10;",
    )
    .expect("CONTAINS single");

    assert_eq!(
        result_ids(&results_any),
        result_ids(&results_single),
        "CONTAINS ANY ('spa') == CONTAINS 'spa'"
    );
}

// =========================================================================
// Edge: array with duplicate elements
// =========================================================================

/// GIVEN a collection where arrays contain duplicate values
/// WHEN querying CONTAINS for a duplicated value
/// THEN the row matches (duplicates don't cause double-counting or errors)
#[test]
fn test_contains_on_array_with_duplicate_elements() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION dupes (dimension = 2, metric = 'cosine');",
    )
    .expect("CREATE dupes");

    let vc = db.get_vector_collection("dupes").expect("get dupes");
    vc.upsert(vec![
        Point::new(
            1,
            vec![1.0, 0.0],
            Some(json!({"tags": ["a", "b", "a", "c"]})),
        ),
        Point::new(2, vec![0.0, 1.0], Some(json!({"tags": ["x", "y"]}))),
    ])
    .expect("upsert dupes");

    let results = execute_sql(&db, "SELECT * FROM dupes WHERE tags CONTAINS 'a' LIMIT 10;")
        .expect("CONTAINS on duplicated element");

    let ids = result_ids(&results);
    assert_eq!(
        ids,
        HashSet::from([1]),
        "Row with duplicated 'a' matches once"
    );

    // CONTAINS ALL with a value that appears twice — should still match
    let results_all = execute_sql(
        &db,
        "SELECT * FROM dupes WHERE tags CONTAINS ALL ('a', 'c') LIMIT 10;",
    )
    .expect("CONTAINS ALL with duplicated element");

    let ids_all = result_ids(&results_all);
    assert_eq!(ids_all, HashSet::from([1]), "Row has both 'a' and 'c'");
}

// =========================================================================
// Combination: CONTAINS OR vector NEAR (hybrid query)
// =========================================================================

/// GIVEN hotels with amenities and vectors
/// WHEN querying WHERE amenities CONTAINS 'spa' OR vector NEAR $v
/// THEN returns union of CONTAINS matches and vector nearest neighbors
#[test]
fn test_contains_combined_with_vector_search() {
    let (_dir, db) = create_test_db();
    setup_hotels_collection(&db);

    let mut params = std::collections::HashMap::new();
    params.insert("v".to_string(), json!([1.0_f32, 0.0, 0.0, 0.0]));

    // CONTAINS 'spa' matches hotels 1, 7
    // vector NEAR [1,0,0,0] is closest to hotel 1 (exact match)
    // Combined with OR → at least hotels 1, 7 plus vector neighbors
    let results = super::helpers::execute_sql_with_params(
        &db,
        "SELECT * FROM hotels WHERE amenities CONTAINS 'spa' OR vector NEAR $v LIMIT 10;",
        &params,
    )
    .expect("CONTAINS OR vector NEAR");

    let ids = result_ids(&results);
    // Must include spa hotels
    assert!(ids.contains(&1), "Hotel 1 has spa");
    assert!(ids.contains(&7), "Hotel 7 has spa");
    // Should also include vector nearest neighbors
    assert!(!ids.is_empty(), "Should have results from OR combination");
}

// =========================================================================
// Combination: CONTAINS with NOT
// =========================================================================

/// GIVEN hotels with amenities
/// WHEN querying WHERE NOT (amenities CONTAINS 'pool')
/// THEN returns hotels WITHOUT pool: 2, 4, 5, 6
#[test]
fn test_contains_with_not_negation() {
    let (_dir, db) = create_test_db();
    setup_hotels_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM hotels WHERE NOT (amenities CONTAINS 'pool') LIMIT 10;",
    )
    .expect("NOT CONTAINS query");

    let ids = result_ids(&results);
    // Hotels without pool: 2 (wifi,parking), 4 (empty), 5 (null/missing), 6 (gym)
    assert!(!ids.contains(&1), "Hotel 1 has pool — should be excluded");
    assert!(!ids.contains(&3), "Hotel 3 has pool — should be excluded");
    assert!(!ids.contains(&7), "Hotel 7 has pool — should be excluded");
    assert!(ids.contains(&2), "Hotel 2 has no pool");
    assert!(ids.contains(&6), "Hotel 6 has no pool");
}

// =========================================================================
// Combination: CONTAINS with OR (non-vector)
// =========================================================================

/// GIVEN hotels with amenities and city
/// WHEN querying WHERE amenities CONTAINS 'spa' OR city = 'Tokyo'
/// THEN returns hotels with spa (1, 7) OR in Tokyo (5)
#[test]
fn test_contains_or_scalar_filter() {
    let (_dir, db) = create_test_db();
    setup_hotels_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM hotels WHERE amenities CONTAINS 'spa' OR city = 'Tokyo' LIMIT 10;",
    )
    .expect("CONTAINS OR scalar");

    let ids = result_ids(&results);
    assert_eq!(ids, HashSet::from([1, 5, 7]), "spa (1,7) OR Tokyo (5)");
}

// =========================================================================
// Combination: CONTAINS with LIMIT truncation
// =========================================================================

/// GIVEN hotels with amenities (3 have pool)
/// WHEN querying WHERE amenities CONTAINS 'pool' LIMIT 2
/// THEN returns exactly 2 results (not 3)
#[test]
fn test_contains_with_limit_truncates_results() {
    let (_dir, db) = create_test_db();
    setup_hotels_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM hotels WHERE amenities CONTAINS 'pool' LIMIT 2;",
    )
    .expect("CONTAINS with LIMIT 2");

    assert_eq!(results.len(), 2, "LIMIT 2 should cap at 2 results");
    // All returned results must have pool
    for r in &results {
        let amenities = r
            .point
            .payload
            .as_ref()
            .and_then(|p| p.get("amenities"))
            .and_then(|v| v.as_array());
        assert!(
            amenities.is_some_and(|arr| arr.iter().any(|v| v.as_str() == Some("pool"))),
            "All results must have pool in amenities"
        );
    }
}

// =========================================================================
// Complex: CONTAINS ALL + CONTAINS ANY combined
// =========================================================================

/// GIVEN hotels with amenities and tags
/// WHEN querying WHERE amenities CONTAINS ALL ('pool', 'gym') AND tags CONTAINS ANY ('new', 'family')
/// THEN returns only hotel 7 (pool+gym AND tag 'new')
#[test]
fn test_contains_all_and_contains_any_combined() {
    let (_dir, db) = create_test_db();
    setup_hotels_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM hotels WHERE amenities CONTAINS ALL ('pool', 'gym') AND tags CONTAINS ANY ('new', 'family') LIMIT 10;",
    )
    .expect("CONTAINS ALL + CONTAINS ANY");

    let ids = result_ids(&results);
    // pool+gym: hotels 1, 7
    // tags new or family: hotels 3 (family), 4 (new), 7 (new)
    // Intersection: hotel 7
    assert_eq!(
        ids,
        HashSet::from([7]),
        "Only hotel 7 matches both conditions"
    );
}

// =========================================================================
// Complex: nested AND/OR with CONTAINS
// =========================================================================

/// GIVEN hotels with amenities, tags, and city
/// WHEN querying WHERE (amenities CONTAINS 'pool' AND city = 'Paris') OR tags CONTAINS 'budget'
/// THEN returns hotels matching either branch
#[test]
fn test_contains_nested_and_or_grouping() {
    let (_dir, db) = create_test_db();
    setup_hotels_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM hotels WHERE (amenities CONTAINS 'pool' AND city = 'Paris') OR tags CONTAINS 'budget' LIMIT 10;",
    )
    .expect("nested AND/OR with CONTAINS");

    let ids = result_ids(&results);
    // (pool AND Paris): hotels 1, 3
    // tags budget: hotels 2, 4
    // Union: 1, 2, 3, 4
    assert_eq!(ids, HashSet::from([1, 2, 3, 4]), "(pool+Paris) OR budget");
}

// =========================================================================
// Complex: CONTAINS on multiple array fields simultaneously
// =========================================================================

/// GIVEN hotels with amenities AND tags arrays
/// WHEN querying WHERE amenities CONTAINS 'gym' AND tags CONTAINS 'luxury'
/// THEN returns hotels that match BOTH array conditions: 1, 6, 7
#[test]
fn test_contains_on_two_different_array_fields() {
    let (_dir, db) = create_test_db();
    setup_hotels_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM hotels WHERE amenities CONTAINS 'gym' AND tags CONTAINS 'luxury' LIMIT 10;",
    )
    .expect("CONTAINS on two array fields");

    let ids = result_ids(&results);
    // gym: 1, 6, 7
    // luxury: 1, 6, 7
    // Intersection: 1, 6, 7
    assert_eq!(ids, HashSet::from([1, 6, 7]), "gym AND luxury");
}
