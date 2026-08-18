use super::timeout_response;
use axum::http::StatusCode;

/// Backlog #13(c): the timeout response must carry the canonical
/// `VELES-027` guard-rail code, not the malformed `VELES-QUERY-TIMEOUT`
/// that lived outside the `VELES-NNN` namespace.
#[tokio::test]
async fn test_timeout_response_uses_veles_027() {
    let response = timeout_response("docs", 250);
    assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read timeout body");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("valid JSON");
    assert_eq!(json["code"], "VELES-027");
}
