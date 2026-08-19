use super::*;

/// Documents why `_engine_only` exists: `[server] port = 443` is a
/// legitimate low HTTP bind port for a hosting shell (e.g.
/// `velesdb-server` behind `setcap`), but fed through the
/// whole-struct loader it lands in *this* crate's own `server.port`
/// and trips `validate_server`'s `>= 1024` rule — a real, reproducible
/// bug when a shell shares its `velesdb.toml` with `VelesConfig`
/// as-is (not a regression test to "fix" — `load_from_path` is
/// correct for standalone/embedded use where `[server]` truly
/// belongs to `VelesConfig`).
#[test]
fn test_load_from_path_whole_struct_rejects_shell_owned_low_port() {
    let dir = tempfile::tempdir().expect("test: temp dir");
    let path = dir.path().join("velesdb.toml");
    std::fs::write(
        &path,
        "[server]\nport = 443\n\n[limits]\nmax_collections = 5\n",
    )
    .expect("test: write toml");

    let err = VelesConfig::load_from_path(&path)
        .expect_err("whole-struct loader must still reject port=443 via its own server section");
    assert!(
        err.to_string().contains("server.port"),
        "unexpected error: {err}"
    );
}

/// The actual fix: a shell-owned `[server] port = 443` no longer
/// leaks into `VelesConfig`'s own `server` section, and the genuine
/// engine section (`[limits]`) is still applied.
#[test]
fn test_load_from_path_engine_only_ignores_shell_owned_server_section() {
    let dir = tempfile::tempdir().expect("test: temp dir");
    let path = dir.path().join("velesdb.toml");
    std::fs::write(
        &path,
        "[server]\nport = 443\n\n[limits]\nmax_collections = 5\n",
    )
    .expect("test: write toml");

    let config = VelesConfig::load_from_path_engine_only(&path)
        .expect("engine-only loader must ignore the shell-owned [server] section");

    // The engine section came through.
    assert_eq!(config.limits.max_collections, 5);
    // The shell-owned [server] section did NOT — the struct's own
    // `server.port` stays at its default, proving the table was
    // dropped rather than parsed-then-happening-to-pass-validation.
    assert_eq!(config.server.port, ServerConfig::default().port);
}

#[test]
fn test_from_toml_engine_only_ignores_shell_owned_server_section() {
    let config = VelesConfig::from_toml_engine_only(
        "[server]\nport = 443\n\n[limits]\nmax_collections = 7\n",
    )
    .expect("engine-only parser must ignore the shell-owned [server] section");

    assert_eq!(config.limits.max_collections, 7);
    assert_eq!(config.server.port, ServerConfig::default().port);
}

#[test]
fn test_load_from_path_engine_only_still_applies_non_server_engine_sections() {
    let dir = tempfile::tempdir().expect("test: temp dir");
    let path = dir.path().join("velesdb.toml");
    std::fs::write(
        &path,
        "[hnsw]\nm = 24\n\n[wal_batch]\nenabled = true\ncommit_delay_us = 250\n",
    )
    .expect("test: write toml");

    let config = VelesConfig::load_from_path_engine_only(&path)
        .expect("engine-only loader must still apply hnsw/wal_batch");

    assert_eq!(config.hnsw.m, Some(24));
    assert!(config.wal_batch.enabled);
    assert_eq!(config.wal_batch.commit_delay_us, 250);
}

#[test]
fn test_load_from_path_engine_only_missing_file_errors() {
    let missing = std::path::Path::new("/nonexistent/velesdb-issue-1549-engine-only.toml");
    assert!(VelesConfig::load_from_path_engine_only(missing).is_err());
}

#[test]
fn test_from_toml_engine_only_invalid_value_still_fails_typed() {
    // max_collections = 0 is out of range — the fix must not silently
    // swallow real validation errors, only shell-owned sections.
    let err = VelesConfig::from_toml_engine_only("[limits]\nmax_collections = 0\n")
        .expect_err("out-of-range engine value must still fail");
    assert!(
        err.to_string().contains("limits.max_collections"),
        "unexpected error: {err}"
    );
}
