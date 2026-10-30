//! The `Host` allowlist: a request whose `Host` the server does not answer
//! to is refused with `421 Misdirected Request`, whatever its method.
//!
//! The attack it stops is DNS rebinding. A page on `evil.example.com` points
//! that name at `127.0.0.1` once loaded; its requests then go to a server on
//! the loopback, *same-origin* as far as the browser is concerned, so CORS and
//! the origin guard both wave them through — and a default server, bound to
//! `127.0.0.1:8080` without API keys, would answer reads and writes alike.
//! What the attacker cannot change is the `Host` header, which still names
//! `evil.example.com`.
//!
//! Driven through `config::apply_browser_policy`, the function `main.rs`
//! installs every guard and the CORS layer with.

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use tempfile::TempDir;
use tower::ServiceExt;
use velesdb_server::config::{apply_browser_policy, ServerConfig};

const REBOUND: &str = "evil.example.com:8080";

fn app(temp_dir: &TempDir, config: &ServerConfig) -> Router {
    let (_, state) = common::create_test_app_with_state(temp_dir);
    apply_browser_policy(
        velesdb_server::routes::api_routes().with_state(state),
        config,
    )
}

fn default_app(temp_dir: &TempDir) -> Router {
    app(temp_dir, &ServerConfig::default())
}

async fn call(app: &Router, method: &str, uri: &str, headers: &[(&str, &str)]) -> StatusCode {
    let mut request = Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    app.clone()
        .oneshot(request.body(Body::empty()).expect("test: build"))
        .await
        .expect("test: request")
        .status()
}

async fn list(app: &Router, host: &str) -> StatusCode {
    call(app, "GET", "/collections", &[("host", host)]).await
}

#[tokio::test]
async fn test_a_rebound_host_is_refused_for_reads_and_writes() {
    let temp_dir = TempDir::new().expect("test: tempdir");
    let app = default_app(&temp_dir);

    assert_eq!(list(&app, REBOUND).await, StatusCode::MISDIRECTED_REQUEST);
    // A rebound page is same-origin for the browser: Origin matches Host.
    let write = call(
        &app,
        "POST",
        "/collections/missing/flush",
        &[
            ("host", REBOUND),
            ("origin", "http://evil.example.com:8080"),
        ],
    )
    .await;
    assert_eq!(write, StatusCode::MISDIRECTED_REQUEST);
}

#[tokio::test]
async fn test_loopback_names_pass_without_configuration() {
    let temp_dir = TempDir::new().expect("test: tempdir");
    let app = default_app(&temp_dir);

    for host in [
        "localhost:8080",
        "LocalHost:8080",
        "localhost",
        "127.0.0.1:8080",
        "127.0.0.5",
        "[::1]:8080",
        "[::1]",
    ] {
        assert_eq!(list(&app, host).await, StatusCode::OK, "{host}");
    }
}

#[tokio::test]
async fn test_x_forwarded_host_is_never_read() {
    let temp_dir = TempDir::new().expect("test: tempdir");
    let app = default_app(&temp_dir);

    let spoofed_in = call(
        &app,
        "GET",
        "/collections",
        &[("host", REBOUND), ("x-forwarded-host", "localhost:8080")],
    )
    .await;
    assert_eq!(spoofed_in, StatusCode::MISDIRECTED_REQUEST);
    let spoofed_out = call(
        &app,
        "GET",
        "/collections",
        &[("host", "localhost:8080"), ("x-forwarded-host", REBOUND)],
    )
    .await;
    assert_eq!(spoofed_out, StatusCode::OK);
}

#[tokio::test]
async fn test_allowed_hosts_and_the_bind_host_are_accepted() {
    let temp_dir = TempDir::new().expect("test: tempdir");
    let config = ServerConfig {
        host: "192.168.1.10".to_string(),
        allowed_hosts: vec!["db.example.com".to_string()],
        ..ServerConfig::default()
    };
    let app = app(&temp_dir, &config);

    for host in [
        "db.example.com:8080",
        "DB.Example.COM",
        "192.168.1.10:8080",
        "localhost",
    ] {
        assert_eq!(list(&app, host).await, StatusCode::OK, "{host}");
    }
    assert_eq!(list(&app, REBOUND).await, StatusCode::MISDIRECTED_REQUEST);
}

#[tokio::test]
async fn test_a_wildcard_bind_host_admits_no_name_by_itself() {
    let temp_dir = TempDir::new().expect("test: tempdir");
    let config = ServerConfig {
        host: "0.0.0.0".to_string(),
        ..ServerConfig::default()
    };
    let app = app(&temp_dir, &config);

    assert_eq!(
        list(&app, "0.0.0.0:8080").await,
        StatusCode::MISDIRECTED_REQUEST
    );
    assert_eq!(
        list(&app, "10.1.2.3:8080").await,
        StatusCode::MISDIRECTED_REQUEST
    );
    assert_eq!(list(&app, "localhost:8080").await, StatusCode::OK);
}

#[tokio::test]
async fn test_an_explicit_wildcard_turns_the_check_off() {
    let temp_dir = TempDir::new().expect("test: tempdir");
    let config = ServerConfig {
        allowed_hosts: vec!["*".to_string()],
        ..ServerConfig::default()
    };
    let app = app(&temp_dir, &config);

    assert_eq!(list(&app, REBOUND).await, StatusCode::OK);
}

/// Liveness and readiness probes come from orchestrators that address the
/// pod by IP; they reveal nothing but up/down, so the probes stay open.
#[tokio::test]
async fn test_health_probes_answer_any_host() {
    let temp_dir = TempDir::new().expect("test: tempdir");
    let app = default_app(&temp_dir);

    for probe in ["/health", "/ready"] {
        let status = call(&app, "GET", probe, &[("host", "10.1.2.3:8080")]).await;
        assert_eq!(status, StatusCode::OK, "{probe}");
    }
}

/// `Origin: null` (a sandboxed iframe, a `file://` page) is no origin the
/// policy lists, so a state change from it is refused.
#[tokio::test]
async fn test_origin_null_cannot_change_state() {
    let temp_dir = TempDir::new().expect("test: tempdir");
    let app = default_app(&temp_dir);

    let status = call(
        &app,
        "POST",
        "/collections/missing/flush",
        &[("host", "localhost:8080"), ("origin", "null")],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

/// `http://localhost` and `Host: localhost:80` are the same origin: the
/// scheme's default port is spelt out or left out on either side.
#[tokio::test]
async fn test_same_origin_normalises_default_ports() {
    let temp_dir = TempDir::new().expect("test: tempdir");
    let app = default_app(&temp_dir);

    for (origin, host) in [
        ("http://localhost", "localhost:80"),
        ("http://localhost:80", "localhost"),
        ("https://localhost", "localhost:443"),
    ] {
        let status = call(
            &app,
            "POST",
            "/collections/missing/flush",
            &[("host", host), ("origin", origin)],
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{origin} / {host}");
    }
}

/// The refusal says what to do about it: a compose service reached as
/// `velesdb:8080` must learn the host name it was refused under and every
/// way to allow it.
#[tokio::test]
async fn test_the_refusal_names_the_host_and_every_way_to_allow_it() {
    let temp_dir = TempDir::new().expect("test: tempdir");
    let app = default_app(&temp_dir);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/collections")
                .header("host", "velesdb:8080")
                .body(Body::empty())
                .expect("test: build"),
        )
        .await
        .expect("test: request");
    assert_eq!(response.status(), StatusCode::MISDIRECTED_REQUEST);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("test: body");
    let message = String::from_utf8_lossy(&body);
    for needle in [
        "velesdb",
        "[server] allowed_hosts",
        "--allowed-hosts",
        "VELESDB_ALLOWED_HOSTS",
    ] {
        assert!(
            message.contains(needle),
            "{needle:?} missing from {message}"
        );
    }
}
