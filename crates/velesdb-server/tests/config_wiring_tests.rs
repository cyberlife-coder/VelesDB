//! Integration tests for the server's `--config`/`VELESDB_CONFIG` wiring
//! (issue #1549: "VelesConfig (TOML) cannot be passed at `Database::open`").
//!
//! Before this fix, `main::init_app_state` always called `Database::open`,
//! so the `--config` flag only ever reached the server-transport settings
//! (`[server]`/`[auth]`/`[tls]`/`[cors]`) and the core engine
//! (`[search]`/`[hnsw]`/`[storage]`/`[limits]`/`[wal_batch]`) silently ran
//! on defaults no matter what the TOML said.
//!
//! These tests exercise `config::load_core_config` +
//! `Database::open_with_config` through the exact helper
//! (`common::create_test_app_with_core_config`) that mirrors
//! `main::init_app_state`, so a regression here means the real binary
//! regressed too.

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt;

async fn create_collection(app: axum::Router, name: &str) -> (StatusCode, serde_json::Value) {
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collections")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({ "name": name, "dimension": 4, "metric": "cosine" }).to_string(),
                ))
                .expect("Failed to build request"),
        )
        .await
        .expect("Request failed");
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("Failed to read body");
    let json: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    (status, json)
}

/// The core proof required by issue #1549: a `limits.max_collections` set
/// in the TOML consumed via `--config` must be *enforced* by the running
/// server, not merely parsed and discarded. First collection succeeds
/// (within the cap), second is refused by the engine.
#[tokio::test]
async fn test_custom_toml_limit_is_enforced_by_the_running_server() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_dir = TempDir::new().expect("Failed to create config dir");
    let config_path = config_dir.path().join("velesdb.toml");
    std::fs::write(&config_path, "[limits]\nmax_collections = 1\n")
        .expect("Failed to write config");

    let app = common::create_test_app_with_core_config(&temp_dir, &config_path);
    let (status, _body) = create_collection(app.clone(), "first").await;
    assert_eq!(status, StatusCode::CREATED, "first collection must succeed");

    let (status, body) = create_collection(app, "second").await;
    assert_ne!(
        status,
        StatusCode::CREATED,
        "second collection must be refused by the configured limit"
    );
    let message = body["error"].as_str().unwrap_or_default();
    assert!(
        message.contains("max_collections"),
        "expected the limit error to mention max_collections, got: {message}"
    );
}

/// Without `--config`, behaviour is unchanged: core defaults apply, so
/// creating a handful of collections still works.
#[tokio::test]
async fn test_no_config_path_keeps_core_defaults() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let core_config = velesdb_server::config::load_core_config(&None)
        .expect("Failed to load default core config");
    assert_eq!(
        core_config.limits.max_collections,
        velesdb_core::config::LimitsConfig::default().max_collections
    );

    let db = velesdb_core::Database::open_with_config(temp_dir.path(), core_config)
        .expect("Failed to open database");
    db.create_vector_collection_with_options(
        "a",
        4,
        velesdb_core::DistanceMetric::Cosine,
        velesdb_core::StorageMode::Full,
    )
    .expect("collection creation should succeed under default limits");
}

/// An explicit `--config` path that does not exist must fail fast — the
/// caller (`main::main`) never reaches `Database::open_with_config` with a
/// silently-defaulted config.
#[tokio::test]
async fn test_explicit_missing_config_path_fails_fast_before_database_open() {
    let missing = std::path::PathBuf::from("/nonexistent/velesdb-issue-1549-server.toml");
    let err = velesdb_server::config::load_core_config(&Some(missing))
        .expect_err("a missing explicit config path must error, not silently default");
    assert!(
        err.to_string().contains("config file not found"),
        "unexpected error: {err}"
    );
}

/// An explicit `--config` path with an out-of-range value must fail fast
/// with the typed core `ConfigError`, not a generic/opaque failure.
#[tokio::test]
async fn test_explicit_invalid_value_fails_fast_with_typed_config_error() {
    let config_dir = TempDir::new().expect("Failed to create config dir");
    let config_path = config_dir.path().join("velesdb.toml");
    // max_collections = 0 is out of range (validate_limits requires >= 1).
    std::fs::write(&config_path, "[limits]\nmax_collections = 0\n")
        .expect("Failed to write config");

    let err = velesdb_server::config::load_core_config(&Some(config_path))
        .expect_err("an invalid value must fail fast with a typed ConfigError");
    assert!(
        err.to_string().contains("limits.max_collections"),
        "unexpected error: {err}"
    );
}
