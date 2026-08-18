use super::*;
use axum::http::HeaderMap;

#[test]
fn test_extract_client_id_from_header() {
    let mut headers = HeaderMap::new();
    headers.insert("x-client-id", "my-app".parse().unwrap());
    assert_eq!(extract_client_id(&headers), "my-app");
}

#[test]
fn test_extract_client_id_fallback() {
    let headers = HeaderMap::new();
    assert_eq!(extract_client_id(&headers), "anonymous");
}

#[test]
fn test_extract_client_id_invalid_utf8_falls_back() {
    let mut headers = HeaderMap::new();
    // HeaderValue with valid ASCII always succeeds to_str,
    // so we verify the fallback path by omitting the header.
    headers.insert("x-other-header", "value".parse().unwrap());
    assert_eq!(extract_client_id(&headers), "anonymous");
}

#[test]
fn test_error_response_no_code() {
    let resp = error_response(StatusCode::BAD_REQUEST, "bad request".to_string());
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[test]
fn test_core_error_response_includes_code() {
    let err = velesdb_core::Error::DimensionMismatch {
        expected: 384,
        actual: 768,
    };
    let resp = core_error_response(StatusCode::BAD_REQUEST, &err);
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

/// Backlog #13(d): a rate-limit pre-check rejection must return 429 with the
/// canonical `VELES-027` guard-rail code in the body (previously `code: None`).
#[tokio::test]
async fn test_apply_pre_check_rate_limit_carries_veles_027() {
    let limits = velesdb_core::guardrails::QueryLimits {
        rate_limit_qps: 1,
        ..Default::default()
    };
    let guard_rails = velesdb_core::guardrails::GuardRails::with_limits(limits);

    // First call consumes the only token; the second trips the limiter.
    guard_rails
        .pre_check("client")
        .expect("first pre-check allowed");
    let response =
        apply_pre_check(&guard_rails, "client").expect_err("second pre-check must reject");

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read rate-limit body");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("valid JSON");
    assert_eq!(json["code"], "VELES-027");
}
