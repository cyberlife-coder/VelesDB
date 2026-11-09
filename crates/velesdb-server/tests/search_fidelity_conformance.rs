#![allow(clippy::doc_markdown)]
//! Cross-binding search fidelity (issue #2095 item C) — the REST leg.
//!
//! Loads the same `conformance/search_fidelity_cases.json` as the core leg,
//! seeds the dataset through the real REST endpoints (`POST /collections`,
//! `POST /collections/{name}/points`), and asserts that `/search` and
//! `/search/hybrid` reproduce the pinned ids AND scores. REST must match the
//! canonical golden exactly — the engine underneath is the same, and any
//! serde/handler divergence is precisely what this net exists to catch (the
//! weighted-fusion default fork fixed in #2093 was this class).

mod common;

use std::collections::HashMap;
use std::path::PathBuf;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use common::create_test_app;
use serde::Deserialize;
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt;

#[derive(Deserialize)]
struct Dataset {
    collection: String,
    dimension: usize,
    metric: String,
    points: Vec<FixturePoint>,
}

#[derive(Deserialize)]
struct FixturePoint {
    id: u64,
    vector: Vec<f32>,
    payload: Value,
}

#[derive(Deserialize)]
struct SurfaceExpect {
    #[allow(dead_code)]
    expect_ids: Vec<u64>,
    #[allow(dead_code)]
    expect_scores: Vec<f32>,
}

#[derive(Deserialize)]
struct Case {
    id: String,
    kind: String,
    collection: String,
    vector: String,
    #[serde(default)]
    text: Option<String>,
    top_k: usize,
    #[serde(default)]
    vector_weight: Option<f32>,
    #[serde(default)]
    rrf_k: Option<u32>,
    expect_ids: Vec<u64>,
    expect_scores: Vec<f32>,
    #[serde(default)]
    #[allow(dead_code)]
    surface_expect: HashMap<String, SurfaceExpect>,
}

#[derive(Deserialize)]
struct Fixture {
    dataset: Dataset,
    #[serde(default)]
    extra_collections: Vec<Dataset>,
    query_vectors: HashMap<String, Vec<f32>>,
    cases: Vec<Case>,
    score_tolerance: f32,
}

fn load_fixture() -> Fixture {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../conformance/search_fidelity_cases.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("read search fidelity fixture"))
        .expect("parse search fidelity fixture")
}

async fn post(app: &axum::Router, uri: &str, body: Value) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("Content-Type", "application/json")
                .body(Body::from(body.to_string()))
                .expect("build request"),
        )
        .await
        .expect("request failed");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("read body");
    let parsed = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, parsed)
}

async fn seed_dataset(app: &axum::Router, ds: &Dataset) {
    let (status, _) = post(
        app,
        "/collections",
        json!({
            "name": ds.collection,
            "dimension": ds.dimension,
            "metric": ds.metric,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create {}", ds.collection);
    let points: Vec<Value> = ds
        .points
        .iter()
        .map(|p| json!({"id": p.id, "vector": p.vector, "payload": p.payload}))
        .collect();
    let (status, _) = post(
        app,
        &format!("/collections/{}/points", ds.collection),
        json!({ "points": points }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "upsert {}", ds.collection);
}

fn case_body(case: &Case, query: &[f32]) -> (String, Value) {
    match case.kind.as_str() {
        "near" => (
            format!("/collections/{}/search", case.collection),
            json!({ "vector": query, "top_k": case.top_k }),
        ),
        "hybrid_rrf" => (
            format!("/collections/{}/search/hybrid", case.collection),
            json!({
                "vector": query,
                "query": case.text,
                "top_k": case.top_k,
                "vector_weight": case.vector_weight,
                "rrf_k": case.rrf_k,
            }),
        ),
        other => panic!("unknown case kind {other:?}"),
    }
}

fn parse_results(body: &Value, case_id: &str) -> Vec<(u64, f64)> {
    body["results"]
        .as_array()
        .unwrap_or_else(|| panic!("case {case_id}: no results array: {body}"))
        .iter()
        .map(|r| {
            let id = r["id"]
                .as_str()
                .and_then(|s| s.parse().ok())
                .or_else(|| r["id"].as_u64())
                .unwrap_or_else(|| panic!("case {case_id}: unparseable id {r}"));
            let score = r["score"].as_f64().expect("score");
            (id, score)
        })
        .collect()
}

#[tokio::test]
async fn test_search_fidelity_fixture_cases() {
    let fixture = load_fixture();
    let temp_dir = TempDir::new().expect("tempdir");
    let app = create_test_app(&temp_dir);
    seed_dataset(&app, &fixture.dataset).await;
    for ds in &fixture.extra_collections {
        seed_dataset(&app, ds).await;
    }
    for case in &fixture.cases {
        let query = &fixture.query_vectors[&case.vector];
        let (uri, body) = case_body(case, query);
        let (status, response) = post(&app, &uri, body).await;
        assert_eq!(status, StatusCode::OK, "case {}: {response}", case.id);
        let actual = parse_results(&response, &case.id);
        let ids: Vec<u64> = actual.iter().map(|r| r.0).collect();
        assert_eq!(ids, case.expect_ids, "case {}: ids differ", case.id);
        for ((id, got), want) in actual.iter().zip(&case.expect_scores) {
            assert!(
                (got - f64::from(*want)).abs() <= f64::from(fixture.score_tolerance),
                "case {}: score of id {id} differs: got {got}, want {want}",
                case.id
            );
        }
    }
}
