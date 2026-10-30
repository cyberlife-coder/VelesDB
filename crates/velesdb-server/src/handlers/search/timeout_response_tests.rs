use super::search_timeout_response;
use axum::http::StatusCode;

/// A `docs` collection under the default limits (`timeout_ms` 30 000).
fn docs(dir: &tempfile::TempDir) -> velesdb_core::collection::VectorCollection {
    let db = velesdb_core::Database::open(dir.path()).expect("open database");
    db.create_vector_collection("docs", 4, velesdb_core::DistanceMetric::Cosine)
        .expect("create collection");
    db.get_vector_collection("docs").expect("collection exists")
}

async fn body(response: axum::response::Response) -> serde_json::Value {
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read timeout body");
    serde_json::from_slice(&body).expect("valid JSON")
}

/// Backlog #13(c): the timeout response must carry the canonical
/// `VELES-027` guard-rail code, not the malformed `VELES-QUERY-TIMEOUT`
/// that lived outside the `VELES-NNN` namespace. A budget tighter than the
/// server's limit is the client's: a `408`, and the breaker counts nothing.
#[tokio::test]
async fn test_a_client_budget_timeout_is_a_408_the_breaker_does_not_count() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let docs = docs(&dir);
    let threshold = docs.guard_rails().limits().circuit_failure_threshold;

    for _ in 0..threshold {
        let response = search_timeout_response(&docs, "docs", 250);
        assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
        let json = body(response).await;
        assert_eq!(json["code"], "VELES-027");
        assert!(
            json["error"].as_str().unwrap_or_default().contains("250ms"),
            "{json}"
        );
    }
    assert!(docs.guard_rails().circuit_breaker.check().is_ok());
}

/// A budget at or above the server's own `timeout_ms` means the search ran
/// that long: load on the engine, a `503` that the breaker counts — the same
/// classification `VelesQL` `WITH (timeout_ms)` uses.
#[tokio::test]
async fn test_a_budget_not_tighter_than_the_limit_is_a_503_the_breaker_counts() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let docs = docs(&dir);
    let limits = docs.guard_rails().limits();

    for _ in 0..limits.circuit_failure_threshold {
        let response = search_timeout_response(&docs, "docs", limits.timeout_ms);
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body(response).await["code"], "VELES-027");
    }
    assert!(docs.guard_rails().circuit_breaker.check().is_err());
}
