//! API Key authentication middleware for VelesDB server.
//!
//! When `VELESDB_API_KEY` is set, all requests (except `/health` and `/swagger-ui`)
//! must include a valid `Authorization: Bearer <key>` or `X-Api-Key: <key>` header.
//!
//! When `VELESDB_API_KEY` is NOT set, authentication is disabled (development mode).

use axum::{
    body::Body,
    extract::Request,
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};

/// Paths that bypass authentication (health checks, docs).
const BYPASS_PATHS: &[&str] = &["/health", "/swagger-ui", "/api-docs"];

/// API key authentication middleware.
///
/// Checks `Authorization: Bearer <key>` or `X-Api-Key: <key>` headers.
/// Skips authentication for health/docs endpoints and when no API key is configured.
pub async fn api_key_auth(
    request: Request<Body>,
    next: Next,
) -> Result<Response, impl IntoResponse> {
    let api_key = std::env::var("VELESDB_API_KEY").ok();

    // No API key configured → development mode, skip auth
    let Some(expected_key) = api_key else {
        return Ok(next.run(request).await);
    };

    // Skip auth for bypass paths
    let path = request.uri().path();
    if BYPASS_PATHS.iter().any(|bp| path.starts_with(bp)) {
        return Ok(next.run(request).await);
    }

    // Check Authorization: Bearer <key>
    let auth_header = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    // Check X-Api-Key: <key>
    let api_key_header = request
        .headers()
        .get("x-api-key")
        .and_then(|v| v.to_str().ok());

    let provided_key = auth_header.or(api_key_header);

    match provided_key {
        Some(key) if key == expected_key => Ok(next.run(request).await),
        Some(_) => Err((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "error": "Invalid API key"
            })),
        )),
        None => Err((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "error": "Missing API key. Set Authorization: Bearer <key> or X-Api-Key: <key>"
            })),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, middleware, routing::get, Router};
    use serial_test::serial;
    use tower::ServiceExt;

    async fn test_handler() -> &'static str {
        "ok"
    }

    fn build_app() -> Router {
        Router::new()
            .route("/collections", get(test_handler))
            .route("/health", get(test_handler))
            .layer(middleware::from_fn(api_key_auth))
    }

    #[tokio::test]
    #[serial]
    async fn test_no_api_key_env_allows_all() {
        // Ensure VELESDB_API_KEY is not set
        std::env::remove_var("VELESDB_API_KEY");

        let app = build_app();
        let req = Request::builder()
            .uri("/collections")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    #[serial]
    async fn test_health_bypasses_auth() {
        std::env::set_var("VELESDB_API_KEY", "test-key-123");

        let app = build_app();
        let req = Request::builder()
            .uri("/health")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        std::env::remove_var("VELESDB_API_KEY");
    }

    #[tokio::test]
    #[serial]
    async fn test_missing_key_returns_401() {
        std::env::set_var("VELESDB_API_KEY", "test-key-456");

        let app = build_app();
        let req = Request::builder()
            .uri("/collections")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        std::env::remove_var("VELESDB_API_KEY");
    }

    #[tokio::test]
    #[serial]
    async fn test_valid_bearer_token() {
        std::env::set_var("VELESDB_API_KEY", "test-key-789");

        let app = build_app();
        let req = Request::builder()
            .uri("/collections")
            .header("Authorization", "Bearer test-key-789")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        std::env::remove_var("VELESDB_API_KEY");
    }

    #[tokio::test]
    #[serial]
    async fn test_valid_x_api_key() {
        std::env::set_var("VELESDB_API_KEY", "test-key-abc");

        let app = build_app();
        let req = Request::builder()
            .uri("/collections")
            .header("x-api-key", "test-key-abc")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        std::env::remove_var("VELESDB_API_KEY");
    }

    #[tokio::test]
    #[serial]
    async fn test_invalid_key_returns_401() {
        std::env::set_var("VELESDB_API_KEY", "correct-key");

        let app = build_app();
        let req = Request::builder()
            .uri("/collections")
            .header("Authorization", "Bearer wrong-key")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        std::env::remove_var("VELESDB_API_KEY");
    }
}
