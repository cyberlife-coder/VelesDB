//! `velesdb config path|show|validate|edit` (#2725).
//!
//! Every case runs in its own temporary working directory with
//! `VELESDB_CONFIG` removed, so the implicit `./velesdb.toml` lookup is the
//! one under test and never the developer's own.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn config_cmd(workdir: &Path) -> Command {
    let mut cmd = Command::cargo_bin("velesdb").unwrap();
    cmd.current_dir(workdir)
        .env_remove("VELESDB_CONFIG")
        .env_remove("VISUAL")
        .env_remove("EDITOR")
        .env("VELESDB_NO_UPDATE_CHECK", "1")
        .arg("config");
    cmd
}

const SHARED_FILE: &str = "[server]\nport = 443\n\n[hnsw]\nm = 24\n";
const INVALID_FILE: &str = "[limits]\nmax_collections = 0\n";

#[test]
fn path_prints_nothing_on_stdout_without_a_file() {
    let dir = TempDir::new().unwrap();
    config_cmd(dir.path())
        .arg("path")
        .assert()
        .success()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("no config file"));
}

#[test]
fn path_prints_the_implicit_file() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("velesdb.toml"), SHARED_FILE).unwrap();
    let expected = fs::canonicalize(dir.path().join("velesdb.toml")).unwrap();
    config_cmd(dir.path())
        .arg("path")
        .assert()
        .success()
        .stdout(format!("{}\n", expected.display()));
}

#[test]
fn path_fails_on_a_missing_explicit_file() {
    let dir = TempDir::new().unwrap();
    config_cmd(dir.path())
        .args(["path", "--config", "missing.toml"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "config file not found: missing.toml",
        ));
}

#[test]
fn show_prints_only_engine_tables_with_env_on_top() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("velesdb.toml"), SHARED_FILE).unwrap();
    let out = config_cmd(dir.path())
        .env("VELESDB_HNSW_M", "40")
        .args(["show", "--format", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(json["hnsw"]["m"], 40, "env must override the file");
    assert!(
        json.get("server").is_none(),
        "non-engine table leaked: {json}"
    );
    assert!(
        json.get("logging").is_none(),
        "non-engine table leaked: {json}"
    );
    assert!(json.get("search").is_some(), "engine table missing: {json}");
}

#[test]
fn show_toml_round_trips_through_validate() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("velesdb.toml"), SHARED_FILE).unwrap();
    let out = config_cmd(dir.path())
        .arg("show")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let shown = String::from_utf8(out).unwrap();
    assert!(shown.contains("[hnsw]\nm = 24\n"), "{shown}");
    fs::write(dir.path().join("shown.toml"), &shown).unwrap();
    config_cmd(dir.path())
        .args(["validate", "shown.toml"])
        .assert()
        .success()
        .stdout(predicate::str::contains("OK:"))
        .stdout(predicate::str::contains("not read by the CLI engine").not());
}

#[test]
fn validate_reports_tables_the_cli_does_not_read() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("velesdb.toml"), SHARED_FILE).unwrap();
    config_cmd(dir.path())
        .arg("validate")
        .assert()
        .success()
        .stdout(predicate::str::contains("engine tables read: [hnsw]"))
        .stdout(predicate::str::contains(
            "not read by the CLI engine: [server]",
        ));
}

#[test]
fn validate_fails_on_an_invalid_file() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("bad.toml"), INVALID_FILE).unwrap();
    config_cmd(dir.path())
        .args(["validate", "bad.toml"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("limits.max_collections"));
}

#[test]
fn edit_validates_what_the_editor_saved() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("velesdb.toml"), SHARED_FILE).unwrap();
    fs::write(dir.path().join("bad.toml"), INVALID_FILE).unwrap();
    config_cmd(dir.path())
        .env("EDITOR", "true")
        .arg("edit")
        .assert()
        .success()
        .stdout(predicate::str::contains("OK:"));
    // `cp bad.toml <file>` stands in for an editor that saves an invalid value.
    config_cmd(dir.path())
        .env("EDITOR", "cp bad.toml")
        .arg("edit")
        .assert()
        .failure()
        .stderr(predicate::str::contains("limits.max_collections"));
}

#[test]
fn edit_fails_without_a_file_or_on_editor_failure() {
    let dir = TempDir::new().unwrap();
    config_cmd(dir.path())
        .env("EDITOR", "true")
        .arg("edit")
        .assert()
        .failure()
        .stderr(predicate::str::contains("no config file to edit"));
    fs::write(dir.path().join("velesdb.toml"), SHARED_FILE).unwrap();
    config_cmd(dir.path())
        .env("EDITOR", "false")
        .arg("edit")
        .assert()
        .failure()
        .stderr(predicate::str::contains("the file was not re-validated"));
    config_cmd(dir.path())
        .arg("edit")
        .assert()
        .failure()
        .stderr(predicate::str::contains("set $VISUAL or $EDITOR"));
}
