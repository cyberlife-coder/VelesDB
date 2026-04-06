//! BDD-style end-to-end tests for strict text filter `CONTAINS_TEXT` (Issue #446).
//!
//! Each scenario follows GIVEN (setup data) → WHEN (execute SQL) → THEN (verify
//! results). Tests exercise the full pipeline: SQL string → `Parser::parse()`
//! → `Database::execute_query()` → verify returned `SearchResult` values.
//!
//! `CONTAINS_TEXT` performs case-sensitive substring matching on string payload
//! fields. Unlike `MATCH` (RRF boost), it is a strict post-filter that excludes
//! any result whose target field does not contain the specified substring.

use std::collections::HashSet;

use serde_json::json;
use velesdb_core::{Database, Point};

use super::helpers::{
    create_test_db, execute_sql, execute_sql_with_params, payload_str, result_ids, vector_param,
};

// =========================================================================
// Module-specific setup
// =========================================================================

/// Populate a `docs` collection with diverse text payloads for CONTAINS_TEXT testing.
///
/// | id | vector           | content                          | category | lang   |
/// |----|------------------|----------------------------------|----------|--------|
/// | 1  | `[1,0,0,0]`     | "learn rust programming today"   | tech     | en     |
/// | 2  | `[0.9,0.1,0,0]` | "python data science guide"      | tech     | en     |
/// | 3  | `[0,1,0,0]`     | "rust and python comparison"     | tech     | en     |
/// | 4  | `[0,0,1,0]`     | "cooking italian pasta recipes"  | food     | en     |
/// | 5  | `[0,0,0,1]`     | "日本語テキスト rust プログラミング" | tech   | ja     |
/// | 6  | `[0.5,0.5,0,0]` | (missing content field)          | misc     | en     |
/// | 7  | `[0.5,0,0.5,0]` | 42 (non-string content)          | misc     | en     |
/// | 8  | `[0,0.5,0,0.5]` | ""  (empty string)               | misc     | en     |
fn setup_docs_collection(db: &Database) {
    execute_sql(
        db,
        "CREATE COLLECTION docs (dimension = 4, metric = 'cosine');",
    )
    .expect("test: CREATE docs");

    let vc = db
        .get_vector_collection("docs")
        .expect("test: get docs collection");

    vc.upsert(vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(json!({"content": "learn rust programming today", "category": "tech", "lang": "en"})),
        ),
        Point::new(
            2,
            vec![0.9, 0.1, 0.0, 0.0],
            Some(json!({"content": "python data science guide", "category": "tech", "lang": "en"})),
        ),
        Point::new(
            3,
            vec![0.0, 1.0, 0.0, 0.0],
            Some(json!({"content": "rust and python comparison", "category": "tech", "lang": "en"})),
        ),
        Point::new(
            4,
            vec![0.0, 0.0, 1.0, 0.0],
            Some(json!({"content": "cooking italian pasta recipes", "category": "food", "lang": "en"})),
        ),
        Point::new(
            5,
            vec![0.0, 0.0, 0.0, 1.0],
            Some(json!({"content": "日本語テキスト rust プログラミング", "category": "tech", "lang": "ja"})),
        ),
        Point::new(
            6,
            vec![0.5, 0.5, 0.0, 0.0],
            Some(json!({"category": "misc", "lang": "en"})),
        ),
        Point::new(
            7,
            vec![0.5, 0.0, 0.5, 0.0],
            Some(json!({"content": 42, "category": "misc", "lang": "en"})),
        ),
        Point::new(
            8,
            vec![0.0, 0.5, 0.0, 0.5],
            Some(json!({"content": "", "category": "misc", "lang": "en"})),
        ),
    ])
    .expect("test: upsert docs");
}

// =========================================================================
// Nominal: CONTAINS_TEXT with vector NEAR returns only matching results
// =========================================================================

/// GIVEN docs with text content and vectors
/// WHEN querying `vector NEAR $v AND content CONTAINS_TEXT 'rust'`
/// THEN returns only results whose content contains "rust"
#[test]
fn test_contains_text_with_near_returns_only_matching() {
    let (_dir, db) = create_test_db();
    setup_docs_collection(&db);

    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let results = execute_sql_with_params(
        &db,
        "SELECT * FROM docs WHERE vector NEAR $v AND content CONTAINS_TEXT 'rust' LIMIT 10;",
        &params,
    )
    .expect("test: NEAR + CONTAINS_TEXT");

    assert!(!results.is_empty(), "Should return at least one result");

    for r in &results {
        let content = payload_str(r, "content").unwrap_or("");
        assert!(
            content.contains("rust"),
            "Every result must contain 'rust', got '{}' for id={}",
            content,
            r.point.id
        );
    }

    let ids = result_ids(&results);
    // ids 1, 3, 5 have "rust" in content; id 2 does not
    assert!(ids.contains(&1), "id=1 has 'rust' in content");
    assert!(!ids.contains(&2), "id=2 has no 'rust' in content");
}

// =========================================================================
// Nominal: CONTAINS_TEXT without vector search as metadata filter
// =========================================================================

/// GIVEN docs with text content
/// WHEN querying `content CONTAINS_TEXT 'rust'` without vector search
/// THEN applies as metadata filter, returning all docs containing "rust"
#[test]
fn test_contains_text_without_vector_search_as_metadata_filter() {
    let (_dir, db) = create_test_db();
    setup_docs_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM docs WHERE content CONTAINS_TEXT 'rust' LIMIT 10;",
    )
    .expect("test: CONTAINS_TEXT metadata filter");

    let ids = result_ids(&results);
    // ids 1, 3, 5 have "rust" in content
    assert_eq!(
        ids,
        HashSet::from([1, 3, 5]),
        "All docs containing 'rust': 1, 3, 5"
    );
}

// =========================================================================
// Nominal: MATCH + CONTAINS_TEXT combined (RRF boost + strict filter)
// =========================================================================

/// GIVEN docs with text content and vectors
/// WHEN querying `content MATCH 'rust' AND content CONTAINS_TEXT 'rust'`
/// THEN MATCH applies RRF boost, CONTAINS_TEXT applies strict filter independently
#[test]
fn test_match_and_contains_text_combined() {
    let (_dir, db) = create_test_db();
    setup_docs_collection(&db);

    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let results = execute_sql_with_params(
        &db,
        "SELECT * FROM docs WHERE vector NEAR $v AND content MATCH 'rust' AND content CONTAINS_TEXT 'rust' LIMIT 10;",
        &params,
    )
    .expect("test: NEAR + MATCH + CONTAINS_TEXT");

    // Every result must strictly contain "rust" (CONTAINS_TEXT guarantee)
    for r in &results {
        let content = payload_str(r, "content").unwrap_or("");
        assert!(
            content.contains("rust"),
            "Strict filter: every result must contain 'rust', got '{}' for id={}",
            content,
            r.point.id
        );
    }
}

// =========================================================================
// Edge: CONTAINS_TEXT on missing field returns empty
// =========================================================================

/// GIVEN docs where id=6 has no "content" field
/// WHEN querying `nonexistent_field CONTAINS_TEXT 'anything'`
/// THEN returns empty result set
#[test]
fn test_contains_text_on_missing_field_returns_empty() {
    let (_dir, db) = create_test_db();
    setup_docs_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM docs WHERE nonexistent_field CONTAINS_TEXT 'anything' LIMIT 10;",
    )
    .expect("test: CONTAINS_TEXT on missing field");

    assert!(
        results.is_empty(),
        "Missing field should match nothing, got {} results",
        results.len()
    );
}

// =========================================================================
// Edge: CONTAINS_TEXT on non-string field returns empty
// =========================================================================

/// GIVEN doc id=7 has content = 42 (integer, not string)
/// WHEN querying `content CONTAINS_TEXT '42'`
/// THEN id=7 is NOT in results (non-string field → false)
#[test]
fn test_contains_text_on_non_string_field_returns_empty() {
    let (_dir, db) = create_test_db();
    setup_docs_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM docs WHERE content CONTAINS_TEXT '42' LIMIT 10;",
    )
    .expect("test: CONTAINS_TEXT on non-string field");

    let ids = result_ids(&results);
    assert!(
        !ids.contains(&7),
        "id=7 has content=42 (integer), should not match CONTAINS_TEXT '42'"
    );
}

// =========================================================================
// Edge: CONTAINS_TEXT '' (empty string) matches all string fields
// =========================================================================

/// GIVEN docs with various content types
/// WHEN querying `content CONTAINS_TEXT ''`
/// THEN matches all docs where content is a string (every string contains "")
#[test]
fn test_contains_text_empty_string_matches_all_string_fields() {
    let (_dir, db) = create_test_db();
    setup_docs_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM docs WHERE content CONTAINS_TEXT '' LIMIT 20;",
    )
    .expect("test: CONTAINS_TEXT empty string");

    let ids = result_ids(&results);
    // ids 1,2,3,4,5 have string content; id 8 has "" (still a string)
    // id 6 has no content field → excluded
    // id 7 has content=42 (integer) → excluded
    assert!(ids.contains(&1), "id=1 string content matches empty");
    assert!(ids.contains(&2), "id=2 string content matches empty");
    assert!(ids.contains(&3), "id=3 string content matches empty");
    assert!(ids.contains(&4), "id=4 string content matches empty");
    assert!(ids.contains(&5), "id=5 string content matches empty");
    assert!(ids.contains(&8), "id=8 empty string matches empty");
    assert!(!ids.contains(&6), "id=6 missing content → excluded");
    assert!(!ids.contains(&7), "id=7 integer content → excluded");
}

// =========================================================================
// Edge: CONTAINS_TEXT with Unicode text
// =========================================================================

/// GIVEN doc id=5 has content with Japanese text
/// WHEN querying `content CONTAINS_TEXT '日本語'`
/// THEN returns id=5
#[test]
fn test_contains_text_with_unicode() {
    let (_dir, db) = create_test_db();
    setup_docs_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM docs WHERE content CONTAINS_TEXT '日本語' LIMIT 10;",
    )
    .expect("test: CONTAINS_TEXT Unicode");

    let ids = result_ids(&results);
    assert_eq!(ids, HashSet::from([5]), "Only id=5 contains '日本語'");
}

// =========================================================================
// Edge: All results filtered out → empty result set
// =========================================================================

/// GIVEN docs collection
/// WHEN querying `vector NEAR $v AND content CONTAINS_TEXT 'nonexistent_keyword_xyz'`
/// THEN returns empty result set without error
#[test]
fn test_contains_text_all_filtered_out_returns_empty() {
    let (_dir, db) = create_test_db();
    setup_docs_collection(&db);

    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let results = execute_sql_with_params(
        &db,
        "SELECT * FROM docs WHERE vector NEAR $v AND content CONTAINS_TEXT 'nonexistent_keyword_xyz' LIMIT 10;",
        &params,
    )
    .expect("test: CONTAINS_TEXT all filtered out");

    assert!(
        results.is_empty(),
        "No doc contains 'nonexistent_keyword_xyz', should return empty"
    );
}

// =========================================================================
// Backward compat: MATCH still works as RRF boost
// =========================================================================

/// GIVEN docs with text content and vectors
/// WHEN querying `vector NEAR $v AND content MATCH 'rust'`
/// THEN MATCH still produces RRF-boosted results (not strict filter)
#[test]
fn test_match_still_works_as_rrf_boost() {
    let (_dir, db) = create_test_db();
    setup_docs_collection(&db);

    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let results = execute_sql_with_params(
        &db,
        "SELECT * FROM docs WHERE vector NEAR $v AND content MATCH 'rust' LIMIT 10;",
        &params,
    )
    .expect("test: MATCH backward compat");

    assert!(
        !results.is_empty(),
        "MATCH + NEAR should return hybrid results"
    );

    // MATCH is a boost, not a strict filter — results without "rust" may appear
    // We just verify the query executes successfully and returns results
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
// Backward compat: CONTAINS (array) still works
// =========================================================================

/// GIVEN a collection with array fields
/// WHEN querying `tags CONTAINS 'tech'`
/// THEN CONTAINS still performs array containment (not text substring)
#[test]
fn test_contains_array_still_works() {
    let (_dir, db) = create_test_db();

    execute_sql(
        &db,
        "CREATE COLLECTION tagged (dimension = 2, metric = 'cosine');",
    )
    .expect("test: CREATE tagged");

    let vc = db
        .get_vector_collection("tagged")
        .expect("test: get tagged");

    vc.upsert(vec![
        Point::new(
            1,
            vec![1.0, 0.0],
            Some(json!({"tags": ["tech", "rust"], "name": "article1"})),
        ),
        Point::new(
            2,
            vec![0.0, 1.0],
            Some(json!({"tags": ["food", "cooking"], "name": "article2"})),
        ),
    ])
    .expect("test: upsert tagged");

    let results = execute_sql(
        &db,
        "SELECT * FROM tagged WHERE tags CONTAINS 'tech' LIMIT 10;",
    )
    .expect("test: CONTAINS array backward compat");

    let ids = result_ids(&results);
    assert_eq!(
        ids,
        HashSet::from([1]),
        "CONTAINS (array) should still work: only id=1 has 'tech' tag"
    );
}
