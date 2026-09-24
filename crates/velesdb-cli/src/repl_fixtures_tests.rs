//! Databases the REPL tests share: a `docs` collection of 2-D points, in a
//! database opened the way each test needs.

use tempfile::TempDir;
use velesdb_core::{Database, DistanceMetric, Point};

/// Opens a fresh database with a single `docs` collection seeded with `n`
/// 2-D points, used by the projection/limit/param regression tests.
pub(crate) fn seed_docs(dir: &TempDir, n: u64) -> Database {
    seed(Database::open(dir.path()).expect("open db"), n)
}

/// As [`seed_docs`], in a database that refuses `perfect` over more than one
/// vector: which quality a REPL search runs at becomes observable.
pub(crate) fn seed_docs_refusing_perfect(dir: &TempDir, n: u64) -> Database {
    let mut config = velesdb_core::VelesConfig::default();
    config.limits.max_perfect_mode_vectors = 1;
    seed(
        Database::open_with_config(dir.path(), config).expect("open db"),
        n,
    )
}

/// As [`seed_docs`], in a database opened through the CLI's `--config` path
/// with a `velesdb.toml` holding `toml`.
pub(crate) fn seed_docs_configured(dir: &TempDir, toml: &str, n: u64) -> Database {
    let path = dir.path().join("velesdb.toml");
    std::fs::write(&path, toml).expect("write config");
    let db = crate::helpers::open_database_with_config(&dir.path().join("data"), Some(&path))
        .expect("open with config");
    seed(db, n)
}

/// Adds the `docs` collection of `n` 2-D points to `db`.
fn seed(db: Database, n: u64) -> Database {
    db.create_collection("docs", 2, DistanceMetric::Cosine)
        .expect("create collection");
    let coll = db.get_vector_collection("docs").expect("vector collection");
    let points: Vec<Point> = (1..=n)
        .map(|i| {
            Point::new(
                i,
                vec![1.0, i as f32],
                Some(serde_json::json!({"category": "x"})),
            )
        })
        .collect();
    coll.upsert(points).expect("upsert");
    db
}
