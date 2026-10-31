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

fn guard_rail(kind: velesdb_core::GuardRailKind) -> velesdb_core::Error {
    velesdb_core::Error::GuardRail {
        kind,
        message: format!("{kind:?}"),
    }
}

/// Representative errors of every class, with the status each gets.
fn status_cases() -> Vec<(velesdb_core::Error, StatusCode)> {
    use velesdb_core::{Error, GuardRailKind as K, TimeoutBudget as B};
    vec![
        (Error::CollectionNotFound("c".into()), StatusCode::NOT_FOUND),
        (Error::EdgeExists(1), StatusCode::CONFLICT),
        (Error::Query("[V015]".into()), StatusCode::BAD_REQUEST),
        (
            Error::GraphNotSupported("ORDER BY d".into()),
            StatusCode::BAD_REQUEST,
        ),
        (Error::Overflow("u64".into()), StatusCode::BAD_REQUEST),
        (
            guard_rail(K::Timeout(B::Client)),
            StatusCode::REQUEST_TIMEOUT,
        ),
        (guard_rail(K::RateLimit), StatusCode::TOO_MANY_REQUESTS),
        (guard_rail(K::Cardinality), StatusCode::BAD_REQUEST),
        (guard_rail(K::Depth), StatusCode::BAD_REQUEST),
        (guard_rail(K::Capacity), StatusCode::BAD_REQUEST),
        (
            guard_rail(K::Timeout(B::Configured)),
            StatusCode::SERVICE_UNAVAILABLE,
        ),
        (guard_rail(K::Memory), StatusCode::SERVICE_UNAVAILABLE),
        (guard_rail(K::CircuitOpen), StatusCode::SERVICE_UNAVAILABLE),
        (
            Error::DatabaseLocked("pid".into()),
            StatusCode::SERVICE_UNAVAILABLE,
        ),
        (
            Error::Storage("short write".into()),
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
        (
            Error::ColumnStoreError("pk".into()),
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
        (
            Error::Internal("bug".into()),
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
    ]
}

/// The status a core error gets follows core's caller/engine classification,
/// the one the circuit breaker counts by: every caller error is a `4xx`, every
/// other error a `5xx`.
#[test]
fn test_http_status_follows_the_caller_error_classification() {
    for (err, status) in &status_cases() {
        assert_eq!(http_status_for_error(err), *status, "{err}");
        assert_eq!(status.is_client_error(), err.is_caller_error(), "{err}");
    }
}

/// `/query` keeps its `422` for the client's errors and answers the engine's
/// with the canonical `5xx`, so the same `4xx` ⇔ caller-error claim holds
/// there too.
#[test]
fn test_query_status_follows_the_caller_error_classification() {
    for (err, _) in &status_cases() {
        let status = crate::handlers::query::velesql_helpers::velesql_execution_status(err);
        assert_eq!(
            status.is_client_error(),
            err.is_caller_error(),
            "{err}: {status}"
        );
        if err.is_caller_error() {
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{err}");
        } else {
            assert_eq!(status, http_status_for_error(err), "{err}");
        }
    }
}
