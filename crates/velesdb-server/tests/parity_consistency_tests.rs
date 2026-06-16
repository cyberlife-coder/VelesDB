#![allow(clippy::doc_markdown)]
//! Consistency parity tests for API-parity PR-4 (gaps 6.9, 6.3-server, 6.10):
//!
//! - `POST /collections/{name}/search/ids`        — `search_ids` fast path
//! - `POST /collections/{name}/search/batch`      — `search_batch_parallel`
//! - `POST /collections/{name}/search/multi/ids`  — `multi_query_search_ids`
//!
//! Each new ids-only / parallel path must return the SAME ranking as the
//! full-result endpoint it optimizes — these tests pin that equivalence so a
//! future divergence (a recall regression) fails CI. Coverage mixes nominal
//! parity (~60%), an edge fallback (filtered ids search), and negatives
//! (filter rejection, unknown collection).

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use common::create_test_app;
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt;

const COLLECTION: &str = "parity_consistency";
const DIM: usize = 4;
const QUERY: [f32; DIM] = [1.0, 0.5, 0.25, 0.1];
const QUERY2: [f32; DIM] = [6.0, 2.5, 1.25, 0.5];

async fn post(app: &axum::Router, uri: &str, body: Value) -> axum::response::Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("Content-Type", "application/json")
                .body(Body::from(body.to_string()))
                .expect("test: build request"),
        )
        .await
        .expect("test: request")
}

async fn read_json(response: axum::response::Response) -> Value {
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("test: read body");
    serde_json::from_slice(&body).expect("test: valid JSON")
}

/// Create the collection and seed six deterministic points with even/odd
/// `bucket` payloads (even => ids 1, 3, 5).
async fn seed(app: &axum::Router) {
    let resp = post(
        app,
        "/collections",
        json!({"name": COLLECTION, "dimension": DIM, "metric": "cosine"}),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED, "test setup: create");

    let points: Vec<Value> = (0..6u64)
        .map(|i| {
            #[allow(clippy::cast_precision_loss)]
            let f = i as f32;
            json!({
                "id": i + 1,
                "vector": [1.0 + f, 0.5 * f, 0.25 * f, 0.1 * f],
                "payload": {"bucket": if i % 2 == 0 { "even" } else { "odd" }},
            })
        })
        .collect();
    let resp = post(
        app,
        &format!("/collections/{COLLECTION}/points"),
        json!({ "points": points }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK, "test setup: seed");
}

/// Extract `(id, score)` pairs from a `{results:[{id,score,...}]}` body.
///
/// Ids are serialized as strings (u64 precision-safe for JS clients), so we
/// keep them as `String` for comparison.
fn id_score_pairs(json: &Value) -> Vec<(String, f64)> {
    json["results"]
        .as_array()
        .expect("test: results array")
        .iter()
        .map(|r| {
            (
                r["id"].as_str().expect("test: id is a string").to_string(),
                r["score"].as_f64().expect("test: score"),
            )
        })
        .collect()
}

fn eq_filter(field: &str, value: &str) -> Value {
    json!({ "condition": { "type": "eq", "field": field, "value": value } })
}

// ---------------------------------------------------------------------
// 6.3-server — /search/ids fast path
// ---------------------------------------------------------------------

#[tokio::test]
async fn search_ids_matches_search_ranking() {
    let dir = TempDir::new().expect("test: dir");
    let app = create_test_app(&dir);
    seed(&app).await;

    let req = json!({ "vector": QUERY, "top_k": 4 });
    let full = read_json(
        post(
            &app,
            &format!("/collections/{COLLECTION}/search"),
            req.clone(),
        )
        .await,
    )
    .await;
    let ids =
        read_json(post(&app, &format!("/collections/{COLLECTION}/search/ids"), req).await).await;

    let full_pairs = id_score_pairs(&full);
    assert_eq!(full_pairs.len(), 4, "expected top_k results from /search");
    assert_eq!(
        full_pairs,
        id_score_pairs(&ids),
        "search_ids fast path must match /search ranking and scores exactly"
    );

    // The ids endpoint must not hydrate payloads.
    let first = &ids["results"][0];
    assert!(
        first.get("payload").is_none(),
        "/search/ids must not include payloads"
    );
}

#[tokio::test]
async fn search_ids_with_filter_falls_back_and_filters() {
    let dir = TempDir::new().expect("test: dir");
    let app = create_test_app(&dir);
    seed(&app).await;

    // A filtered ids request is ineligible for the fast path and falls back to
    // the generic pipeline, which honours the filter.
    let req = json!({ "vector": QUERY, "top_k": 6, "filter": eq_filter("bucket", "even") });
    let resp = post(&app, &format!("/collections/{COLLECTION}/search/ids"), req).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let pairs = id_score_pairs(&read_json(resp).await);
    assert!(
        !pairs.is_empty(),
        "filtered ids search should return matches"
    );
    for (id, _) in &pairs {
        assert!(
            ["1", "3", "5"].contains(&id.as_str()),
            "filter must exclude odd-bucket ids, got {id}"
        );
    }
}

// ---------------------------------------------------------------------
// 6.9 — /search/batch parallel path
// ---------------------------------------------------------------------

#[tokio::test]
async fn batch_search_parallel_matches_individual_search() {
    let dir = TempDir::new().expect("test: dir");
    let app = create_test_app(&dir);
    seed(&app).await;

    // No filters => batch takes the parallel kernel; results must match the
    // serial per-query search exactly.
    let single = |q: [f32; DIM]| {
        let app = app.clone();
        async move {
            let body = read_json(
                post(
                    &app,
                    &format!("/collections/{COLLECTION}/search"),
                    json!({ "vector": q, "top_k": 3 }),
                )
                .await,
            )
            .await;
            id_score_pairs(&body)
        }
    };
    let s1 = single(QUERY).await;
    let s2 = single(QUERY2).await;

    let batch = read_json(
        post(
            &app,
            &format!("/collections/{COLLECTION}/search/batch"),
            json!({ "searches": [
                { "vector": QUERY, "top_k": 3 },
                { "vector": QUERY2, "top_k": 3 },
            ] }),
        )
        .await,
    )
    .await;

    let results = batch["results"].as_array().expect("test: batch results");
    assert_eq!(results.len(), 2);
    assert_eq!(id_score_pairs(&results[0]), s1, "batch query 0 must match");
    assert_eq!(id_score_pairs(&results[1]), s2, "batch query 1 must match");
}

// ---------------------------------------------------------------------
// 6.10 — /search/multi/ids
// ---------------------------------------------------------------------

#[tokio::test]
async fn multi_query_search_ids_matches_multi_query_search() {
    let dir = TempDir::new().expect("test: dir");
    let app = create_test_app(&dir);
    seed(&app).await;

    let req = json!({ "vectors": [QUERY, QUERY2], "top_k": 3, "strategy": "rrf" });
    let full = read_json(
        post(
            &app,
            &format!("/collections/{COLLECTION}/search/multi"),
            req.clone(),
        )
        .await,
    )
    .await;
    let ids = read_json(
        post(
            &app,
            &format!("/collections/{COLLECTION}/search/multi/ids"),
            req,
        )
        .await,
    )
    .await;

    assert_eq!(
        id_score_pairs(&full),
        id_score_pairs(&ids),
        "multi_query_search_ids must match multi_query_search fused ranking"
    );
    assert!(
        ids["results"][0].get("payload").is_none(),
        "/search/multi/ids must not include payloads"
    );
}

#[tokio::test]
async fn multi_query_search_ids_rejects_filter() {
    let dir = TempDir::new().expect("test: dir");
    let app = create_test_app(&dir);
    seed(&app).await;

    let req = json!({
        "vectors": [QUERY],
        "top_k": 3,
        "strategy": "rrf",
        "filter": eq_filter("bucket", "even"),
    });
    let resp = post(
        &app,
        &format!("/collections/{COLLECTION}/search/multi/ids"),
        req,
    )
    .await;
    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "filters are unsupported on the ids-only fusion endpoint"
    );
}

#[tokio::test]
async fn multi_query_search_ids_unknown_collection_returns_404() {
    let dir = TempDir::new().expect("test: dir");
    let app = create_test_app(&dir);

    let resp = post(
        &app,
        "/collections/missing/search/multi/ids",
        json!({ "vectors": [QUERY], "top_k": 3, "strategy": "rrf" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
