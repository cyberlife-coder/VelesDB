#![allow(clippy::doc_markdown)]
//! Shared VelesQL contract conformance tests (server runtime).

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use common::create_test_app;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use tempfile::TempDir;
use tower::ServiceExt;

#[derive(Debug, Deserialize)]
struct ConformanceFixture {
    contract_version: String,
    cases: Vec<ConformanceCase>,
}

#[derive(Debug, Deserialize)]
struct ConformanceCase {
    id: String,
    runtimes: Vec<String>,
    method: String,
    path: String,
    body: Value,
    expected_status: u16,
    expected_error_code: Option<String>,
    expect_contract_meta: Option<bool>,
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../conformance/velesql_contract_cases.json")
}

fn load_fixture() -> ConformanceFixture {
    let content = std::fs::read_to_string(fixture_path())
        .expect("failed to read velesql conformance fixture");
    serde_json::from_str(&content).expect("invalid velesql conformance fixture json")
}

async fn seed_docs_collection(app: &axum::Router) {
    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "docs_conformance",
                        "dimension": 4,
                        "metric": "cosine"
                    })
                    .to_string(),
                ))
                .expect("build request"),
        )
        .await
        .expect("create collection request failed");
    assert_eq!(create.status(), StatusCode::CREATED);

    let upsert = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections/docs_conformance/points")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "points": [
                            {
                                "id": 1,
                                "vector": [1.0, 0.0, 0.0, 0.0],
                                "payload": { "_labels": ["Doc"], "category": "tech", "title": "doc-1" }
                            },
                            {
                                "id": 2,
                                "vector": [0.0, 1.0, 0.0, 0.0],
                                "payload": { "_labels": ["Doc"], "category": "science", "title": "doc-2" }
                            }
                        ]
                    })
                    .to_string(),
                ))
                .expect("build request"),
        )
        .await
        .expect("upsert request failed");
    assert_eq!(upsert.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_velesql_contract_conformance_fixture_cases() {
    let fixture = load_fixture();
    let temp_dir = TempDir::new().expect("temp dir");
    let app = create_test_app(&temp_dir);

    seed_docs_collection(&app).await;

    for case in fixture
        .cases
        .iter()
        .filter(|case| case.runtimes.iter().any(|rt| rt == "server"))
    {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(case.method.as_str())
                    .uri(&case.path)
                    .header("Content-Type", "application/json")
                    .body(Body::from(case.body.to_string()))
                    .expect("build request"),
            )
            .await
            .expect("request failed");

        let expected_status =
            StatusCode::from_u16(case.expected_status).expect("expected status must be valid");
        assert_eq!(
            response.status(),
            expected_status,
            "case {} returned unexpected status",
            case.id
        );

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");
        let json: Value = serde_json::from_slice(&body).expect("body should be valid json");

        if let Some(error_code) = &case.expected_error_code {
            assert_eq!(
                json["error"]["code"],
                error_code.as_str(),
                "case {} returned unexpected error code",
                case.id
            );
        }

        if case.expect_contract_meta.unwrap_or(false) {
            assert_eq!(
                json["meta"]["velesql_contract_version"], fixture.contract_version,
                "case {} returned unexpected contract version",
                case.id
            );
        }
    }
}
