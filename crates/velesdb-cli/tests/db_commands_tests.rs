//! `velesdb db stats|vacuum|snapshot|restore` (#2725).
//!
//! Each case builds its own database under a fresh temporary directory.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn velesdb() -> Command {
    let mut cmd = Command::cargo_bin("velesdb").unwrap();
    cmd.env_remove("VELESDB_CONFIG")
        .env("VELESDB_NO_UPDATE_CHECK", "1");
    cmd
}

/// A database with one vector collection (`docs`, one point) and one
/// metadata collection (`meta`), in `dir/db`.
fn make_db(dir: &Path) -> PathBuf {
    let db = dir.join("db");
    velesdb()
        .args(["collection", "create"])
        .arg(&db)
        .args(["docs", "--dimension", "4"])
        .assert()
        .success();
    velesdb()
        .args(["data", "upsert"])
        .arg(&db)
        .args(["docs", "--id", "1", "--vector", "[0.1,0.2,0.3,0.4]"])
        .assert()
        .success();
    velesdb()
        .args(["collection", "create-metadata"])
        .arg(&db)
        .arg("meta")
        .assert()
        .success();
    db
}

#[test]
fn stats_reports_every_collection_in_table_and_json() {
    let dir = TempDir::new().unwrap();
    let db = make_db(dir.path());
    velesdb()
        .args(["db", "stats"])
        .arg(&db)
        .assert()
        .success()
        .stdout(predicate::str::contains("docs"))
        .stdout(predicate::str::contains("Vector"))
        .stdout(predicate::str::contains("meta"));
    let out = velesdb()
        .args(["db", "stats", "-f", "json"])
        .arg(&db)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(json["collections"][0]["name"], "docs", "{json}");
    assert_eq!(json["collections"][0]["points"], 1, "{json}");
    assert!(json["total_disk_bytes"].as_u64().unwrap() > 0, "{json}");
}

#[test]
fn vacuum_rebuilds_vector_collections_and_skips_the_rest() {
    let dir = TempDir::new().unwrap();
    let db = make_db(dir.path());
    velesdb()
        .args(["db", "vacuum"])
        .arg(&db)
        .assert()
        .success()
        .stdout(predicate::str::contains("docs: index rebuilt"))
        .stdout(predicate::str::contains("meta").not());
    velesdb()
        .args(["db", "vacuum", "--collection", "docs", "--compact"])
        .arg(&db)
        .assert()
        .success()
        .stdout(predicate::str::contains("storage compacted"));
    velesdb()
        .args(["db", "vacuum", "--collection", "meta"])
        .arg(&db)
        .assert()
        .failure()
        .stderr(predicate::str::contains("no HNSW index"));
    velesdb()
        .args(["db", "vacuum", "--collection", "nope"])
        .arg(&db)
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

#[test]
fn snapshot_then_restore_yields_an_identical_database() {
    let dir = TempDir::new().unwrap();
    let db = make_db(dir.path());
    let snap = dir.path().join("snap");
    velesdb()
        .args(["db", "snapshot"])
        .arg(&db)
        .arg("-o")
        .arg(&snap)
        .assert()
        .success()
        .stdout(predicate::str::contains("collection(s)"));
    // A snapshot is complete only when the manifest is the last file written.
    assert!(snap.join("snapshot.manifest.json").is_file());
    assert!(!snap.join("velesdb.lock").exists(), "lock file copied");

    let restored = dir.path().join("restored");
    velesdb()
        .args(["db", "restore"])
        .arg(&snap)
        .arg("-o")
        .arg(&restored)
        .assert()
        .success()
        .stdout(predicate::str::contains("Verified"));
    velesdb()
        .args(["collection", "list"])
        .arg(&restored)
        .assert()
        .success()
        .stdout(predicate::str::contains("docs"))
        .stdout(predicate::str::contains("meta"));
    // The point itself survived the round trip.
    velesdb()
        .args(["data", "get"])
        .arg(&restored)
        .args(["docs", "1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"id\": 1"));
}

#[test]
fn snapshot_and_restore_refuse_unsafe_shapes() {
    let dir = TempDir::new().unwrap();
    let db = make_db(dir.path());
    // Destination inside the source tree.
    velesdb()
        .args(["db", "snapshot"])
        .arg(&db)
        .args(["-o", db.join("inner").to_string_lossy().as_ref()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("its own directory"));
    // Non-empty destination.
    let snap = dir.path().join("snap");
    fs::create_dir_all(&snap).unwrap();
    fs::write(snap.join("x"), "x").unwrap();
    velesdb()
        .args(["db", "snapshot"])
        .arg(&db)
        .args(["-o", snap.to_string_lossy().as_ref()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not empty"));
    // Restore without a manifest: incomplete copy, not a snapshot.
    let plain = dir.path().join("plain");
    fs::create_dir_all(&plain).unwrap();
    velesdb()
        .args(["db", "restore"])
        .arg(&plain)
        .args(["-o", dir.path().join("out").to_string_lossy().as_ref()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not a velesdb snapshot"));
}
