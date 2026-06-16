#![cfg(all(test, feature = "persistence"))]
//! EPIC-081 phase 3d (Layer A) — secondary metadata indexes survive a restart.
//!
//! Before phase 3d, `Collection::open` reset `secondary_indexes` to an empty
//! map and nothing re-created them: a `CREATE INDEX` was lost on restart, so
//! the ordered-index `ORDER BY` fast path, the bitmap pre-filter, `EXPLAIN`
//! `IndexLookup`, and the index advisor all silently changed behaviour (results
//! stayed correct via the exhaustive fallback). Layer A persists the indexed
//! field names in `config.json` (the authority) and rebuilds each index from
//! the recovered payloads on open. These tests lock that contract end to end —
//! the strongest gate is restart **equivalence**: the same query returns the
//! same rows after a reopen as a fresh no-index collection holding the same
//! data.

use serde_json::json;
use std::collections::HashMap;
use tempfile::TempDir;
use velesdb_core::velesql::Parser;
use velesdb_core::{Database, DistanceMetric, Point, StorageMode, VectorCollection};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Creates a fresh "docs" vector collection rooted under `dir`.
fn create_docs(dir: &TempDir) -> VectorCollection {
    VectorCollection::create(
        dir.path().join("docs"),
        "docs",
        2,
        DistanceMetric::Cosine,
        StorageMode::Full,
    )
    .expect("create collection")
}

/// Reopens the "docs" collection from disk (the restart under test).
fn reopen_docs(dir: &TempDir) -> VectorCollection {
    VectorCollection::open(dir.path().join("docs")).expect("reopen collection")
}

/// Builds points from `(id, year)` rows; the vector is irrelevant to a scalar
/// `ORDER BY`, and storage/id order deliberately differs from year order.
fn year_points(rows: &[(u64, i64)]) -> Vec<Point> {
    rows.iter()
        .map(|&(id, year)| Point::new(id, vec![1.0, 0.0], Some(json!({ "year": year }))))
        .collect()
}

fn ids(results: &[velesdb_core::point::SearchResult]) -> Vec<u64> {
    results.iter().map(|r| r.point.id).collect()
}

fn query_ids(c: &VectorCollection, sql: &str) -> Vec<u64> {
    ids(&c.execute_query_str(sql, &HashMap::new()).expect("query"))
}

/// Result of the same query on a fresh no-index collection holding `points` —
/// the exhaustive baseline a restored index must match.
fn exhaustive_baseline(points: &[Point], sql: &str) -> Vec<u64> {
    let dir = TempDir::new().expect("temp dir");
    let c = create_docs(&dir);
    c.upsert(points.to_vec()).expect("upsert baseline");
    query_ids(&c, sql)
}

/// Builds a fresh "docs" collection from `points`, indexes `"year"`, flushes,
/// and drops the handle — returning the dir so the caller can reopen it.
/// Centralises the create → upsert → `create_index` → flush → drop restart setup.
fn build_indexed_then_drop(points: Vec<Point>) -> TempDir {
    let dir = TempDir::new().expect("temp dir");
    let c = create_docs(&dir);
    c.upsert(points).expect("upsert");
    c.create_index("year").expect("create index");
    c.flush_full().expect("flush");
    dir
}

const ROWS: &[(u64, i64)] = &[
    (1, 2020),
    (2, 2022),
    (3, 2021),
    (4, 2023),
    (5, 2019),
    (6, 2022),
    (7, 2020),
    (8, 2019),
];

// ---------------------------------------------------------------------------
// #4 — config is the persisted authority
// ---------------------------------------------------------------------------

#[test]
fn config_indexed_fields_round_trips_through_restart() {
    let dir = TempDir::new().expect("temp dir");
    {
        let c = create_docs(&dir);
        c.upsert(year_points(ROWS)).expect("upsert");
        c.create_index("year").expect("create year");
        c.create_index("category").expect("create category");
        c.flush_full().expect("flush");
    }
    let reopened = reopen_docs(&dir);
    let fields = reopened.config().indexed_fields;
    assert!(fields.contains("year"), "year must persist as authority");
    assert!(
        fields.contains("category"),
        "category must persist as authority"
    );
    assert_eq!(fields.len(), 2, "exactly the two created fields");
}

#[test]
fn no_index_collection_does_not_serialize_indexed_fields() {
    let dir = TempDir::new().expect("temp dir");
    let c = create_docs(&dir);
    c.upsert(year_points(ROWS)).expect("upsert");
    c.flush_full().expect("flush");
    let config_json =
        std::fs::read_to_string(dir.path().join("docs").join("config.json")).expect("read config");
    assert!(
        !config_json.contains("indexed_fields"),
        "skip_serializing_if must omit the empty set (no config drift): {config_json}"
    );
}

#[test]
fn config_without_indexed_fields_key_opens_to_empty_set() {
    // Backward-compat: a config.json written before phase 3d has no
    // `indexed_fields` key and must deserialize to an empty set, opening
    // cleanly with no secondary indexes (results still correct via fallback).
    let dir = TempDir::new().expect("temp dir");
    {
        let c = create_docs(&dir);
        c.upsert(year_points(ROWS)).expect("upsert");
        c.flush_full().expect("flush");
    }
    let config_path = dir.path().join("docs").join("config.json");
    let raw = std::fs::read_to_string(&config_path).expect("read config");
    assert!(!raw.contains("indexed_fields"), "precondition: key absent");
    let reopened = reopen_docs(&dir);
    assert!(reopened.config().indexed_fields.is_empty());
    assert!(!reopened.has_secondary_index("year"));
}

// ---------------------------------------------------------------------------
// #4 / #5 — the index survives restart and the fast path serves it
// ---------------------------------------------------------------------------

#[test]
fn create_index_survives_restart_and_query_matches_baseline() {
    let sql = "SELECT * FROM docs ORDER BY year DESC LIMIT 4";
    let dir = TempDir::new().expect("temp dir");
    let pre_restart = {
        let c = create_docs(&dir);
        c.upsert(year_points(ROWS)).expect("upsert");
        c.create_index("year").expect("create index");
        let pre = query_ids(&c, sql);
        c.flush_full().expect("flush");
        pre
    };
    let reopened = reopen_docs(&dir);
    assert!(
        reopened.has_secondary_index("year"),
        "index must be restored from the config authority — not lost on restart"
    );
    let post_restart = query_ids(&reopened, sql);
    assert_eq!(
        post_restart, pre_restart,
        "the restored index must return the identical page it did before restart"
    );
    assert_eq!(
        post_restart,
        exhaustive_baseline(&year_points(ROWS), sql),
        "the restored fast path must equal the exhaustive no-index baseline"
    );
}

#[test]
fn restored_index_matches_baseline_across_asc_offset_and_ties() {
    // Ties (2019→{5,8}, 2020→{1,7}, 2022→{2,6}) + OFFSET exercise the
    // within-bucket ordering the restored index must reproduce.
    let dir = build_indexed_then_drop(year_points(ROWS));
    let reopened = reopen_docs(&dir);
    assert!(
        reopened.has_secondary_index("year"),
        "the index must be restored (else this test would pass on the fallback alone)"
    );
    for sql in [
        "SELECT * FROM docs ORDER BY year ASC LIMIT 5",
        "SELECT * FROM docs ORDER BY year ASC LIMIT 3 OFFSET 2",
        "SELECT * FROM docs ORDER BY year DESC LIMIT 100",
    ] {
        assert_eq!(
            query_ids(&reopened, sql),
            exhaustive_baseline(&year_points(ROWS), sql),
            "restored index diverged from baseline for: {sql}"
        );
    }
}

// ---------------------------------------------------------------------------
// #4 — DROP stays dropped (no resurrection)
// ---------------------------------------------------------------------------

#[test]
fn drop_index_stays_dropped_after_restart() {
    let dir = TempDir::new().expect("temp dir");
    {
        let c = create_docs(&dir);
        c.upsert(year_points(ROWS)).expect("upsert");
        c.create_index("year").expect("create index");
        c.flush_full().expect("flush");
        assert!(c.drop_secondary_index("year"), "drop returns true");
        c.flush_full().expect("flush after drop");
    }
    let reopened = reopen_docs(&dir);
    assert!(
        !reopened.has_secondary_index("year"),
        "a dropped index must NOT resurrect on restart"
    );
    assert!(
        reopened.config().indexed_fields.is_empty(),
        "the config authority must no longer list the dropped field"
    );
}

#[test]
fn drop_without_prior_index_is_false_and_persists_nothing() {
    let dir = TempDir::new().expect("temp dir");
    let c = create_docs(&dir);
    c.upsert(year_points(ROWS)).expect("upsert");
    assert!(
        !c.drop_secondary_index("never_indexed"),
        "dropping a non-existent index returns false"
    );
    assert!(c.config().indexed_fields.is_empty());
}

// ---------------------------------------------------------------------------
// #5 — restart equivalence crossing a TTL sweep and an explicit delete
// ---------------------------------------------------------------------------

#[test]
fn restart_equivalence_across_ttl_expired_and_deleted_rows() {
    // id1 carries a past `_veles_expires_at` (lazy-expired but unswept); id4 is
    // explicitly deleted. The restored index must produce the SAME page as a
    // fresh no-index collection holding the identical surviving data.
    let rows = &[(1i64, 2020), (2, 2022), (3, 2021), (4, 2023), (5, 2019)];
    let with_ttl = |&(id, year): &(i64, i64)| {
        let payload = if id == 1 {
            json!({ "year": year, "_veles_expires_at": 1 })
        } else {
            json!({ "year": year })
        };
        #[allow(clippy::cast_sign_loss)]
        Point::new(id as u64, vec![1.0, 0.0], Some(payload))
    };
    let points: Vec<Point> = rows.iter().map(with_ttl).collect();

    let dir = TempDir::new().expect("temp dir");
    {
        let c = create_docs(&dir);
        c.upsert(points.clone()).expect("upsert");
        c.delete(&[4]).expect("delete id4");
        c.create_index("year").expect("create index");
        c.flush_full().expect("flush");
    }
    let reopened = reopen_docs(&dir);

    // Baseline = a fresh no-index collection with the same upsert + delete.
    let baseline_dir = TempDir::new().expect("temp dir");
    let baseline = create_docs(&baseline_dir);
    baseline.upsert(points).expect("upsert baseline");
    baseline.delete(&[4]).expect("delete baseline");

    for sql in [
        "SELECT * FROM docs ORDER BY year ASC LIMIT 3",
        "SELECT * FROM docs ORDER BY year ASC LIMIT 2 OFFSET 1",
        "SELECT * FROM docs ORDER BY year DESC LIMIT 10",
    ] {
        assert_eq!(
            query_ids(&reopened, sql),
            query_ids(&baseline, sql),
            "restored index (post TTL+delete) diverged from the live baseline for: {sql}"
        );
    }
}

// ---------------------------------------------------------------------------
// #2 — vector-collection coverage denominator (rows missing the field)
// ---------------------------------------------------------------------------

#[test]
fn restart_equivalence_when_some_rows_lack_the_indexed_field() {
    // Two of five points have no `year`. A full ORDER BY sorts field-missing
    // rows first (ASC); the index is NOT fully covering, so the fast path must
    // decline — before AND after restart the result must equal the exhaustive
    // baseline (correctness regardless of which path runs).
    let points = vec![
        Point::new(1, vec![1.0, 0.0], Some(json!({ "year": 2020 }))),
        Point::new(2, vec![1.0, 0.0], Some(json!({ "other": "x" }))),
        Point::new(3, vec![1.0, 0.0], Some(json!({ "year": 2019 }))),
        Point::new(4, vec![1.0, 0.0], Some(json!({ "other": "y" }))),
        Point::new(5, vec![1.0, 0.0], Some(json!({ "year": 2021 }))),
    ];
    let dir = build_indexed_then_drop(points.clone());
    let reopened = reopen_docs(&dir);
    assert!(
        reopened.has_secondary_index("year"),
        "the index entry must be restored even when it does not fully cover the collection"
    );
    for sql in [
        "SELECT * FROM docs ORDER BY year ASC LIMIT 5",
        "SELECT * FROM docs ORDER BY year DESC LIMIT 3",
    ] {
        assert_eq!(
            query_ids(&reopened, sql),
            exhaustive_baseline(&points, sql),
            "partial-coverage restored index diverged from baseline for: {sql}"
        );
    }
}

// ---------------------------------------------------------------------------
// #2 / #5 — WHERE-filtered ORDER BY top-k (the route with no decline guard)
// ---------------------------------------------------------------------------

#[test]
fn restart_filtered_orderby_topk_matches_baseline() {
    let sql = "SELECT * FROM docs WHERE year >= 2020 ORDER BY year ASC LIMIT 3";
    let dir = build_indexed_then_drop(year_points(ROWS));
    let reopened = reopen_docs(&dir);
    assert!(
        reopened.has_secondary_index("year"),
        "the index must be restored to exercise the WHERE-filtered ordered route"
    );
    assert_eq!(
        query_ids(&reopened, sql),
        exhaustive_baseline(&year_points(ROWS), sql),
        "restored WHERE-filtered top-k diverged from the exhaustive baseline"
    );
}

// ---------------------------------------------------------------------------
// #4 — end-to-end DDL through the Database / VelesQL pipeline
// ---------------------------------------------------------------------------

fn execute_sql(db: &Database, sql: &str) -> Vec<velesdb_core::point::SearchResult> {
    let query = Parser::parse(sql).expect("parse");
    db.execute_query(&query, &HashMap::new()).expect("execute")
}

/// Opens a `Database` at `dir`, creates the "docs" vector collection, upserts
/// `ROWS`, and issues `CREATE INDEX ON docs (year)` via the `VelesQL` DDL
/// pipeline. Shared by the two DDL restart tests.
fn db_with_indexed_docs(dir: &TempDir) -> Database {
    let db = Database::open(dir.path()).expect("open db");
    db.create_collection("docs", 2, DistanceMetric::Cosine)
        .expect("create collection");
    db.get_vector_collection("docs")
        .expect("collection")
        .upsert(year_points(ROWS))
        .expect("upsert");
    execute_sql(&db, "CREATE INDEX ON docs (year)");
    db
}

#[test]
fn ddl_create_index_survives_database_restart() {
    let dir = TempDir::new().expect("temp dir");
    drop(db_with_indexed_docs(&dir)); // create + index, then close before reopen
    let db = Database::open(dir.path()).expect("reopen db");
    let coll = db.get_vector_collection("docs").expect("collection");
    assert!(
        coll.has_secondary_index("year"),
        "CREATE INDEX must survive a Database restart without being re-issued"
    );
    assert!(coll.config().indexed_fields.contains("year"));
}

#[test]
fn ddl_drop_index_stays_dropped_after_database_restart() {
    let dir = TempDir::new().expect("temp dir");
    {
        let db = db_with_indexed_docs(&dir);
        execute_sql(&db, "DROP INDEX ON docs (year)");
    }
    let db = Database::open(dir.path()).expect("reopen db");
    let coll = db.get_vector_collection("docs").expect("collection");
    assert!(
        !coll.has_secondary_index("year"),
        "DROP INDEX must survive a restart (no resurrection)"
    );
    assert!(coll.config().indexed_fields.is_empty());
}

// ---------------------------------------------------------------------------
// #2 — metadata-only collection restore (payload-ids coverage denominator)
// ---------------------------------------------------------------------------

#[test]
fn restore_equivalence_for_metadata_only_collection() {
    // Metadata-only collections derive point_count from payload ids (not
    // vector_storage.len), so they exercise the other coverage-denominator
    // branch. Build a status-indexed metadata collection, restart, and compare
    // an ORDER BY status query to a fresh no-index metadata baseline.
    let status_points = || -> Vec<Point> {
        [(1u64, "b"), (2, "a"), (3, "c"), (4, "a")]
            .iter()
            .map(|&(id, s)| Point::new(id, vec![], Some(json!({ "status": s }))))
            .collect()
    };
    let dir = TempDir::new().expect("temp dir");
    {
        let db = Database::open(dir.path()).expect("open db");
        db.create_metadata_collection("meta").expect("create meta");
        db.get_metadata_collection("meta")
            .expect("meta")
            .upsert(status_points())
            .expect("upsert");
        execute_sql(&db, "CREATE INDEX ON meta (status)");
        db.get_metadata_collection("meta")
            .expect("meta")
            .flush_full()
            .expect("flush");
    }
    let db = Database::open(dir.path()).expect("reopen db");
    let coll = db.get_metadata_collection("meta").expect("meta");

    // Baseline: fresh no-index metadata collection with the same rows.
    let bdir = TempDir::new().expect("temp dir");
    let bdb = Database::open(bdir.path()).expect("open baseline db");
    bdb.create_metadata_collection("meta").expect("create");
    bdb.get_metadata_collection("meta")
        .expect("meta")
        .upsert(status_points())
        .expect("upsert");
    let baseline = bdb.get_metadata_collection("meta").expect("meta");

    for sql in [
        "SELECT * FROM meta ORDER BY status ASC LIMIT 4",
        "SELECT * FROM meta ORDER BY status DESC LIMIT 2",
    ] {
        let restored = ids(&coll
            .execute_query_str(sql, &HashMap::new())
            .expect("restored"));
        let base = ids(&baseline
            .execute_query_str(sql, &HashMap::new())
            .expect("baseline"));
        assert_eq!(
            restored, base,
            "metadata-only restored index diverged from baseline for: {sql}"
        );
    }
}

// ---------------------------------------------------------------------------
// #4 — a CREATE INDEX whose authority cannot be persisted surfaces the error
// ---------------------------------------------------------------------------

#[cfg(feature = "test-fault-injection")]
#[test]
fn create_index_surfaces_save_config_failure() {
    use velesdb_core::fault_injection::SaveConfigFaultGuard;
    let dir = TempDir::new().expect("temp dir");
    let c = create_docs(&dir);
    c.upsert(year_points(ROWS)).expect("upsert");
    // The create-time save_config already ran; reset and fail the NEXT call,
    // which is the one create_index issues after recording the authority.
    let _guard = SaveConfigFaultGuard::activate_on_first_call();
    assert!(
        c.create_index("year").is_err(),
        "a CREATE INDEX whose config.json save fails must surface the error, \
         not silently lose the index on restart"
    );
}
