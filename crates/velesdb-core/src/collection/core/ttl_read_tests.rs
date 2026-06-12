//! Durable point-TTL enforcement on every read surface (AM-1).
//!
//! A point whose payload carries a past `_veles_expires_at` (epoch seconds)
//! must be invisible in search, get, scroll, text/hybrid search, SELECT and
//! MATCH; `get_raw` must still return it (TTL rebuild / snapshot paths).

#![cfg(all(test, feature = "persistence"))]

use crate::collection::expiry::now_unix_secs;
use crate::collection::types::Collection;
use crate::test_fixtures::fixtures::{make_point_with_payload, setup_collection};
use std::collections::HashMap;

/// Collection with one live point (1), one expired point (2), and one
/// point with a future TTL (3). All share the BM25 token "rust" and the
/// metadata field `category = "tech"`.
fn setup_ttl_collection() -> (tempfile::TempDir, Collection) {
    let future = now_unix_secs() + 100_000;
    let points = vec![
        make_point_with_payload(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            serde_json::json!({"title": "rust programming", "category": "tech"}),
        ),
        make_point_with_payload(
            2,
            vec![0.9, 0.1, 0.0, 0.0],
            serde_json::json!({
                "title": "rust tutorial",
                "category": "tech",
                "_veles_expires_at": 1_000_u64
            }),
        ),
        make_point_with_payload(
            3,
            vec![0.8, 0.2, 0.0, 0.0],
            serde_json::json!({
                "title": "rust handbook",
                "category": "tech",
                "_veles_expires_at": future
            }),
        ),
    ];
    let (dir, col) = setup_collection(4);
    col.upsert(points).expect("test: upsert");
    (dir, col)
}

fn ids_of(results: &[crate::point::SearchResult]) -> Vec<u64> {
    let mut ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    ids.sort_unstable();
    ids
}

#[test]
fn test_vector_search_excludes_expired_point() {
    let (_dir, col) = setup_ttl_collection();
    let results = col.search(&[1.0, 0.0, 0.0, 0.0], 10).expect("test: search");
    assert_eq!(
        ids_of(&results),
        vec![1, 3],
        "expired point 2 must be invisible; future-TTL point 3 visible"
    );
}

#[test]
fn test_get_filters_expired_but_get_raw_returns_it() {
    let (_dir, col) = setup_ttl_collection();

    let got = col.get(&[1, 2, 3]);
    assert!(got[0].is_some(), "live point visible via get");
    assert!(got[1].is_none(), "expired point hidden via get");
    assert!(got[2].is_some(), "future-TTL point visible via get");

    let raw = col.get_raw(&[2]);
    let point = raw[0].as_ref().expect("get_raw must return expired point");
    assert_eq!(
        point
            .payload
            .as_ref()
            .and_then(|p| p.get("_veles_expires_at"))
            .and_then(serde_json::Value::as_u64),
        Some(1_000),
        "get_raw must preserve the expiry payload field"
    );
}

#[test]
fn test_get_boundary_expiry_equal_to_now_is_expired() {
    let (_dir, col) = setup_collection(4);
    col.upsert(vec![make_point_with_payload(
        7,
        vec![1.0, 0.0, 0.0, 0.0],
        serde_json::json!({"_veles_expires_at": now_unix_secs()}),
    )])
    .expect("test: upsert");

    // `exp <= now`: a TTL of 0 seconds expires immediately.
    assert!(
        col.get(&[7])[0].is_none(),
        "exp == now must already be expired"
    );
}

#[test]
fn test_scroll_skips_expired_point() {
    let (_dir, col) = setup_ttl_collection();
    let batch = col.scroll_batch(None, 10, None).expect("test: scroll");
    let ids: Vec<u64> = batch.points.iter().map(|p| p.id).collect();
    assert_eq!(ids, vec![1, 3], "scroll must skip the expired point");
}

#[test]
fn test_text_search_excludes_expired_point() {
    let (_dir, col) = setup_ttl_collection();
    let results = col.text_search("rust", 10).expect("test: text search");
    assert_eq!(ids_of(&results), vec![1, 3]);
}

#[test]
fn test_hybrid_search_excludes_expired_point() {
    let (_dir, col) = setup_ttl_collection();
    let results = col
        .hybrid_search(&[1.0, 0.0, 0.0, 0.0], "rust", 10, None, None)
        .expect("test: hybrid search");
    assert_eq!(ids_of(&results), vec![1, 3]);
}

#[test]
fn test_select_where_scan_excludes_expired_point() {
    let (_dir, col) = setup_ttl_collection();
    let results = col
        .execute_query_str(
            "SELECT * FROM c WHERE category = 'tech' LIMIT 10",
            &HashMap::new(),
        )
        .expect("test: SELECT scan");
    assert_eq!(ids_of(&results), vec![1, 3]);
}

#[test]
fn test_select_near_excludes_expired_point() {
    let (_dir, col) = setup_ttl_collection();
    let mut params = HashMap::new();
    params.insert("v".to_string(), serde_json::json!([1.0, 0.0, 0.0, 0.0]));
    let results = col
        .execute_query_str("SELECT * FROM c WHERE vector NEAR $v LIMIT 10", &params)
        .expect("test: SELECT NEAR");
    assert_eq!(ids_of(&results), vec![1, 3]);
}

#[test]
fn test_match_excludes_expired_point() {
    let (_dir, col) = setup_ttl_collection();
    let results = col
        .execute_query_str("MATCH (a) RETURN a LIMIT 10", &HashMap::new())
        .expect("test: MATCH");
    assert_eq!(
        ids_of(&results),
        vec![1, 3],
        "MATCH regression: expired node must stay invisible"
    );
}
