use super::*;

// -----------------------------------------------------------------
// `open_database_with_config` (issue #1549 — CLI `--config` wiring)
// -----------------------------------------------------------------
//
// These exercise `open_database_with_config` directly (the
// process-global `open_database`/`set_config_path` pair is a thin
// wrapper covered indirectly through the binary's `--config` flag).

#[test]
fn test_no_config_path_opens_with_core_defaults() {
    let dir = tempfile::tempdir().expect("test: temp dir");
    let db = open_database_with_config(dir.path(), None).expect("test: open without config");
    assert_eq!(
        db.config().limits.max_collections,
        velesdb_core::config::LimitsConfig::default().max_collections
    );
}

#[test]
fn test_custom_toml_limit_is_actually_enforced_not_just_parsed() {
    let dir = tempfile::tempdir().expect("test: temp dir");
    let toml_dir = tempfile::tempdir().expect("test: config dir");
    let config_path = toml_dir.path().join("velesdb.toml");
    std::fs::write(&config_path, "[limits]\nmax_collections = 1\n").expect("test: write config");

    let db = open_database_with_config(dir.path(), Some(&config_path))
        .expect("test: open with custom config");

    // Sanity: the value really was parsed onto the running config.
    assert_eq!(db.config().limits.max_collections, 1);

    // Proof it's *enforced*, not just parsed: first collection succeeds,
    // second is refused by the engine because of the configured cap.
    db.create_vector_collection_with_options(
        "first",
        4,
        velesdb_core::DistanceMetric::Cosine,
        velesdb_core::StorageMode::Full,
    )
    .expect("test: first collection under the limit should succeed");

    let err = db
        .create_vector_collection_with_options(
            "second",
            4,
            velesdb_core::DistanceMetric::Cosine,
            velesdb_core::StorageMode::Full,
        )
        .expect_err("test: second collection should be refused by the configured limit");
    assert!(
        err.to_string().contains("max_collections"),
        "unexpected error: {err}"
    );
}

/// Regression test (Fable review finding): a `velesdb.toml` shared with
/// `velesdb-server` may legitimately have `[server] port = 443` (that
/// binary's own HTTP bind port). Before the fix this also landed in
/// `VelesConfig`'s own unrelated `server.port` field and was rejected
/// by its `>= 1024` rule, so opening the *same* file from the CLI
/// failed even though the CLI never reads `[server]` at all.
#[test]
fn test_shell_owned_server_section_does_not_block_cli_open() {
    let dir = tempfile::tempdir().expect("test: temp dir");
    let toml_dir = tempfile::tempdir().expect("test: config dir");
    let config_path = toml_dir.path().join("velesdb.toml");
    std::fs::write(
        &config_path,
        "[server]\nport = 443\n\n[limits]\nmax_collections = 5\n",
    )
    .expect("test: write config");

    let db = open_database_with_config(dir.path(), Some(&config_path))
        .expect("a shell-owned [server] port=443 must not block CLI database open");
    assert_eq!(db.config().limits.max_collections, 5);
}

#[test]
fn test_explicit_missing_config_path_fails_fast_no_silent_default() {
    let dir = tempfile::tempdir().expect("test: temp dir");
    let missing = std::path::Path::new("/nonexistent/velesdb-issue-1549.toml");

    let err = match open_database_with_config(dir.path(), Some(missing)) {
        Err(e) => e,
        Ok(_) => panic!("test: missing explicit config path must error, not fall back"),
    };
    assert!(
        err.to_string().contains("config file not found"),
        "unexpected error: {err}"
    );
}

#[test]
fn test_invalid_config_value_surfaces_typed_config_error_fail_fast() {
    let dir = tempfile::tempdir().expect("test: temp dir");
    let toml_dir = tempfile::tempdir().expect("test: config dir");
    let config_path = toml_dir.path().join("velesdb.toml");
    // max_collections = 0 is out of range (validate_limits requires >= 1).
    std::fs::write(&config_path, "[limits]\nmax_collections = 0\n").expect("test: write config");

    let err = match open_database_with_config(dir.path(), Some(&config_path)) {
        Err(e) => e,
        Ok(_) => panic!("test: invalid value must fail fast, not silently default"),
    };
    assert!(
        err.to_string().contains("limits.max_collections"),
        "unexpected error: {err}"
    );
}

#[test]
fn test_point_payload_to_row_with_payload() {
    let payload = Some(serde_json::json!({
        "title": "Hello",
        "score": 0.95
    }));

    let row = point_payload_to_row(42, &payload);

    assert_eq!(row.get("id"), Some(&serde_json::json!(42)));
    assert_eq!(row.get("title"), Some(&serde_json::json!("Hello")));
    assert_eq!(row.get("score"), Some(&serde_json::json!(0.95)));
    assert_eq!(row.len(), 3);
}

#[test]
fn test_point_payload_to_row_without_payload() {
    let row = point_payload_to_row(7, &None);

    assert_eq!(row.get("id"), Some(&serde_json::json!(7)));
    assert_eq!(row.len(), 1);
}

#[test]
fn test_point_payload_to_browse_row_truncates() {
    let long_string = "a".repeat(80);
    let payload = Some(serde_json::json!({
        "content": long_string,
        "short": "ok"
    }));

    let row = point_payload_to_browse_row(1, &payload);

    assert_eq!(row.get("id"), Some(&serde_json::json!(1)));
    // "short" stays unchanged
    assert_eq!(row.get("short"), Some(&serde_json::json!("ok")));
    // "content" is truncated to 47 chars + "..."
    let content = row.get("content").unwrap().as_str().unwrap();
    assert_eq!(content.len(), 50);
    assert!(content.ends_with("..."));
}

#[test]
fn test_truncate_display_value_short_string() {
    let val = serde_json::json!("short text");
    let result = truncate_display_value(&val);
    assert_eq!(result, serde_json::json!("short text"));
}

#[test]
fn test_truncate_display_value_long_string() {
    let long = "x".repeat(100);
    let result = truncate_display_value(&serde_json::json!(long));
    let s = result.as_str().unwrap();
    assert_eq!(s.len(), 50);
    assert!(s.ends_with("..."));
    assert!(s.starts_with("xxxxxxx"));
}
