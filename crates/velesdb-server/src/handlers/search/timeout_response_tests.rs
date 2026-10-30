use super::client_timeout_response;
use axum::http::StatusCode;

/// Backlog #13(c): the timeout response must carry the canonical
/// `VELES-027` guard-rail code, not the malformed `VELES-QUERY-TIMEOUT`
/// that lived outside the `VELES-NNN` namespace. The budget was the
/// request's own, so it is a `408` and the breaker counts nothing.
#[tokio::test]
async fn test_timeout_response_uses_veles_027() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db = velesdb_core::Database::open(dir.path()).expect("open database");
    db.create_vector_collection("docs", 4, velesdb_core::DistanceMetric::Cosine)
        .expect("create collection");
    let docs = db.get_vector_collection("docs").expect("collection exists");
    let threshold = docs.guard_rails().limits().circuit_failure_threshold;

    for _ in 0..threshold {
        let response = client_timeout_response(&docs, "docs", 250);
        assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read timeout body");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("valid JSON");
        assert_eq!(json["code"], "VELES-027");
        assert!(
            json["error"].as_str().unwrap_or_default().contains("250ms"),
            "{json}"
        );
    }
    assert!(docs.guard_rails().circuit_breaker.check().is_ok());
}
