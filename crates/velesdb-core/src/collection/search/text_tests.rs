//! Tests for full-text (BM25) and hybrid (vector + text) search methods.

#![cfg(all(test, feature = "persistence"))]

use crate::collection::types::Collection;
use crate::filter::{Condition, Filter};
use crate::test_fixtures::fixtures::{make_point_with_payload, setup_collection};

/// Helper: create a collection with points that have text-rich payloads.
///
/// The BM25 index is populated automatically during `upsert` when payloads
/// contain string fields (Collection::upsert indexes all string values).
fn setup_text_collection() -> (tempfile::TempDir, Collection) {
    let points = vec![
        make_point_with_payload(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            serde_json::json!({
                "title": "rust programming language",
                "category": "tech"
            }),
        ),
        make_point_with_payload(
            2,
            vec![0.0, 1.0, 0.0, 0.0],
            serde_json::json!({
                "title": "python programming tutorial",
                "category": "tech"
            }),
        ),
        make_point_with_payload(
            3,
            vec![0.0, 0.0, 1.0, 0.0],
            serde_json::json!({
                "title": "football world cup results",
                "category": "sports"
            }),
        ),
        make_point_with_payload(
            4,
            vec![0.5, 0.5, 0.0, 0.0],
            serde_json::json!({
                "title": "rust web framework comparison",
                "category": "tech"
            }),
        ),
    ];

    let (dir, col) = setup_collection(4);
    col.upsert(points).expect("test: upsert");
    (dir, col)
}

// -----------------------------------------------------------------------
// Basic BM25 text search
// -----------------------------------------------------------------------

#[test]
fn test_text_search_finds_matching_documents() {
    let (_dir, col) = setup_text_collection();

    let results = col.text_search("rust", 10).expect("text search");

    // Points 1 and 4 both contain "rust" in their title.
    let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    assert!(ids.contains(&1), "should find point 1 (rust programming)");
    assert!(ids.contains(&4), "should find point 4 (rust web framework)");
    assert!(!ids.contains(&3), "should not find point 3 (football)");
}

#[test]
fn test_text_search_no_match_returns_empty() {
    let (_dir, col) = setup_text_collection();

    let results = col
        .text_search("quantum_entanglement_xyz", 10)
        .expect("text search");
    assert!(results.is_empty(), "nonsense query should return no hits");
}

#[test]
fn test_text_search_respects_k_limit() {
    let (_dir, col) = setup_text_collection();

    let results = col.text_search("programming", 1).expect("text search");
    assert!(results.len() <= 1, "should respect k=1 limit");
}

// -----------------------------------------------------------------------
// BM25 text search with metadata filter
// -----------------------------------------------------------------------

#[test]
fn test_text_search_with_filter_narrows_results() {
    let (_dir, col) = setup_text_collection();

    let filter = Filter::new(Condition::eq("category", "sports"));
    let results = col
        .text_search_with_filter("programming", 10, &filter)
        .expect("filtered text search");

    // "programming" appears in tech docs only; sports filter should exclude them.
    assert!(
        results.is_empty(),
        "no sports docs contain 'programming', got {} results",
        results.len()
    );
}

#[test]
fn test_text_search_with_filter_includes_matching_category() {
    let (_dir, col) = setup_text_collection();

    let filter = Filter::new(Condition::eq("category", "tech"));
    let results = col
        .text_search_with_filter("rust", 10, &filter)
        .expect("filtered text search");

    // Both rust docs (id=1, id=4) are category=tech.
    for r in &results {
        let cat = r
            .point
            .payload
            .as_ref()
            .and_then(|p| p.get("category"))
            .and_then(|v| v.as_str());
        assert_eq!(cat, Some("tech"), "all filtered results must be tech");
    }
    assert!(!results.is_empty(), "should find tech rust docs");
}

// -----------------------------------------------------------------------
// Hybrid search (vector + text)
// -----------------------------------------------------------------------

#[test]
fn test_hybrid_search_combines_vector_and_text() {
    let (_dir, col) = setup_text_collection();

    // Vector query close to point 1 ([1,0,0,0]) + text query "rust"
    let vector_query = vec![0.9, 0.1, 0.0, 0.0];
    let results = col
        .hybrid_search(&vector_query, "rust", 5, None, None)
        .expect("hybrid search");

    // Point 1 should rank high (close vector + "rust" in title).
    assert!(!results.is_empty(), "hybrid search should return results");
    let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    assert!(
        ids.contains(&1),
        "point 1 should appear (vector + text match)"
    );
}

#[test]
fn test_hybrid_search_dimension_mismatch_error() {
    let (_dir, col) = setup_text_collection();

    let bad_vec = vec![1.0, 0.0]; // dim=2, expected=4
    let result = col.hybrid_search(&bad_vec, "rust", 5, None, None);
    assert!(result.is_err(), "wrong dimension should error");
}
