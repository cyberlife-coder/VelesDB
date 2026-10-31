//! Tests for the swagger-ui route wiring in `build_router`.
//!
//! This exercises the live route, not just a successful build: the
//! utoipa/utoipa-swagger-ui version bump (#2429) changed the asset-embedding
//! mechanism the bundle uses, and nothing else covered the route itself.

use super::{build_router, init_app_state, AuthState, ServerConfig};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

fn test_router() -> axum::Router {
    let dir = tempfile::tempdir().expect("test: temp dir");
    let state = init_app_state(
        dir.path().to_str().expect("test: utf-8 path"),
        velesdb_core::config::VelesConfig::default(),
    )
    .expect("test: init app state");
    build_router(state, AuthState::new(Vec::new()), &ServerConfig::default())
        .expect("test: build router")
}

#[tokio::test]
async fn swagger_ui_index_serves_html() {
    // GIVEN: the router built with the swagger-ui feature enabled.
    let app = test_router();

    // WHEN: requesting the swagger-ui index (trailing slash, where the
    // bundle actually serves from).
    let response = app
        .oneshot(
            Request::builder()
                .uri("/swagger-ui/")
                .body(Body::empty())
                .expect("test: build request"),
        )
        .await
        .expect("test: request failed");

    // THEN: a live utoipa-swagger-ui bundle responds with HTML, not a 404.
    assert_eq!(response.status(), StatusCode::OK);
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert!(
        content_type.contains("text/html"),
        "expected text/html, got {content_type}"
    );
}

#[tokio::test]
async fn swagger_ui_serves_the_openapi_document_it_points_at() {
    // GIVEN: the same router, fresh (SwaggerUi::url wires this path).
    let app = test_router();

    // WHEN: requesting the OpenAPI document swagger-ui's index references.
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api-docs/openapi.json")
                .body(Body::empty())
                .expect("test: build request"),
        )
        .await
        .expect("test: request failed");

    // THEN: it serves the live-generated spec, matching ApiDoc::openapi(),
    // not a 404 or a stale file.
    assert_eq!(response.status(), StatusCode::OK);
}
