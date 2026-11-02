//! A cross-origin page cannot make the server change state, body or not.
//!
//! CORS only decides what a page may *read*; a request without a preflight
//! is still sent. The routes that take a body are out of reach through
//! their `Content-Type` (`content_type_guard_tests.rs`), but the body-less
//! `POST`s — flush, analyze, index rebuild, vacuum, compact, locality
//! reorder — were not: a page on any site could fire them blind. The origin
//! guard refuses, with `403`, every request with a non-safe method whose
//! `Origin` the effective CORS policy does not allow. Requests without
//! `Origin` (curl, the SDKs, server to server) never meet it.
//!
//! Driven through `routes::api_routes()` and `config::apply_browser_policy`,
//! exactly the pair `main.rs` assembles — the origin guard is only ever
//! installed behind the `Host` check, which its same-origin rule relies on.

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use tempfile::TempDir;
use tower::ServiceExt;
use velesdb_server::config::{apply_browser_policy, CorsConfig, ServerConfig};

const FOREIGN: &str = "https://evil.example.com";
const FRONT_END: &str = "https://app.example.com";

fn app(temp_dir: &TempDir, origins: &[&str]) -> Router {
    let (_, state) = common::create_test_app_with_state(temp_dir);
    let cors = CorsConfig {
        allowed_origins: origins.iter().map(ToString::to_string).collect(),
        ..CorsConfig::default()
    };
    apply_browser_policy(
        velesdb_server::routes::api_routes().with_state(state),
        &policy(cors),
    )
}

/// The server config these tests run under: `cors`, and `db.internal` as a
/// host the server answers to, so the `Host` check stays out of the way.
fn policy(cors: CorsConfig) -> ServerConfig {
    ServerConfig {
        cors,
        allowed_hosts: vec!["db.internal".to_string()],
        ..ServerConfig::default()
    }
}

/// `POST /collections/missing/flush`: a `404` means the request reached the
/// handler, a `403` that the guard stopped it.
async fn flush(app: &Router, origin: Option<&str>, host: &str) -> StatusCode {
    let mut request = Request::builder()
        .method("POST")
        .uri("/collections/missing/flush")
        .header("host", host);
    if let Some(origin) = origin {
        request = request.header("origin", origin);
    }
    app.clone()
        .oneshot(request.body(Body::empty()).expect("test: build"))
        .await
        .expect("test: request")
        .status()
}

#[tokio::test]
async fn test_closed_default_refuses_a_cross_origin_state_change() {
    let temp_dir = TempDir::new().expect("test: tempdir");
    let app = app(&temp_dir, &[]);

    assert_eq!(
        flush(&app, Some(FOREIGN), "localhost:8080").await,
        StatusCode::FORBIDDEN
    );
    // Positive controls: no Origin (curl, SDKs) and a same-origin page.
    assert_eq!(
        flush(&app, None, "localhost:8080").await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        flush(&app, Some("http://localhost:8080"), "localhost:8080").await,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn test_an_explicit_list_admits_its_origins_only() {
    let temp_dir = TempDir::new().expect("test: tempdir");
    let app = app(&temp_dir, &[FRONT_END]);

    assert_eq!(
        flush(&app, Some(FRONT_END), "db.internal:8080").await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        flush(&app, Some(FOREIGN), "db.internal:8080").await,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn test_an_explicit_wildcard_admits_every_origin() {
    let temp_dir = TempDir::new().expect("test: tempdir");
    let app = app(&temp_dir, &["*"]);

    assert_eq!(
        flush(&app, Some(FOREIGN), "localhost:8080").await,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn test_safe_methods_and_preflights_are_not_the_guards_business() {
    let temp_dir = TempDir::new().expect("test: tempdir");
    let app = app(&temp_dir, &[FRONT_END]);

    let get = Request::builder()
        .uri("/health")
        .header("origin", FOREIGN)
        .body(Body::empty())
        .expect("test: build");
    let response = app.clone().oneshot(get).await.expect("test: request");
    assert_eq!(response.status(), StatusCode::OK);

    let preflight = Request::builder()
        .method("OPTIONS")
        .uri("/collections/c/flush")
        .header("origin", FRONT_END)
        .header("access-control-request-method", "POST")
        .body(Body::empty())
        .expect("test: build");
    let response = app.clone().oneshot(preflight).await.expect("test: request");
    assert!(response.status().is_success(), "{}", response.status());
    assert_eq!(
        response
            .headers()
            .get("access-control-allow-origin")
            .map(|v| v.to_str().ok()),
        Some(Some(FRONT_END))
    );
}

/// `cors_enabled = false` closes the guard too, whatever the list says.
#[tokio::test]
async fn test_cors_disabled_refuses_even_a_listed_origin() {
    let temp_dir = TempDir::new().expect("test: tempdir");
    let (_, state) = common::create_test_app_with_state(&temp_dir);
    let cors = CorsConfig {
        enabled: false,
        allowed_origins: vec![FRONT_END.to_string()],
        ..CorsConfig::default()
    };
    let app = apply_browser_policy(
        velesdb_server::routes::api_routes().with_state(state),
        &policy(cors),
    );

    assert_eq!(
        flush(&app, Some(FRONT_END), "db.internal:8080").await,
        StatusCode::FORBIDDEN
    );
}
