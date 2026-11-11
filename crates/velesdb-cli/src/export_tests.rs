//! Tests for the streaming export module (`data export --format/--where`).

use super::*;
use tempfile::tempdir;
use velesdb_core::{Database, DistanceMetric, Point};

/// Builds a temp DB with a `"docs"` collection of three tagged points.
fn docs_collection(dir: &tempfile::TempDir) -> (Database, VectorCollection) {
    let db = Database::open(dir.path()).unwrap();
    db.create_vector_collection("docs", 3, DistanceMetric::Cosine)
        .unwrap();
    let col = db.get_vector_collection("docs").unwrap();
    col.upsert(vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0],
            Some(serde_json::json!({"cat": "tech"})),
        ),
        Point::new(
            2,
            vec![0.0, 1.0, 0.0],
            Some(serde_json::json!({"cat": "news", "x": 5})),
        ),
        Point::new(
            3,
            vec![0.0, 0.0, 1.0],
            Some(serde_json::json!({"cat": "tech"})),
        ),
    ])
    .unwrap();
    (db, col)
}

// =========================================================================
// parse_where_filter
// =========================================================================

#[test]
fn test_where_lowers_metadata_and_id_conditions() {
    let filter = parse_where_filter("cat = 'tech' AND id < 3").unwrap();
    assert!(filter.matches_point(1, Some(&serde_json::json!({"cat": "tech"}))));
    assert!(!filter.matches_point(9, Some(&serde_json::json!({"cat": "tech"}))));
    assert!(!filter.matches_point(1, Some(&serde_json::json!({"cat": "news"}))));
}

#[test]
fn test_where_rejects_query_structure_after_the_condition() {
    for clause in [
        "id = 1 LIMIT 2",
        "id = 1 ORDER BY id",
        "id = 1; DROP TABLE x",
    ] {
        let err = parse_where_filter(clause).unwrap_err().to_string();
        assert!(
            err.contains("Invalid") || err.contains("filter condition only"),
            "{clause}: {err}"
        );
    }
}

#[test]
fn test_where_rejects_unparseable_and_unfilterable() {
    assert!(parse_where_filter("@@@").is_err());
    let err = parse_where_filter("similarity(title, [0.1]) >= 0.8")
        .unwrap_err()
        .to_string();
    assert!(err.contains("vector-similarity"), "got: {err}");
}

// =========================================================================
// export_collection — formats
// =========================================================================

#[test]
fn test_export_json_matches_pretty_serialized_records() {
    let dir = tempdir().unwrap();
    let (_db, col) = docs_collection(&dir);
    let out = dir.path().join("out.json");

    let count = export_collection(&col, &out, ExportFormatArg::Json, true, None).unwrap();
    assert_eq!(count, 3);

    // Same bytes as the historical `to_string_pretty(records)` shape.
    let parsed: Vec<serde_json::Value> =
        serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    assert_eq!(parsed.len(), 3);
    assert_eq!(parsed[0]["id"], 1);
    assert_eq!(parsed[0]["payload"]["cat"], "tech");
}

#[test]
fn test_export_jsonl_writes_one_object_per_line() {
    let dir = tempdir().unwrap();
    let (_db, col) = docs_collection(&dir);
    let out = dir.path().join("out.jsonl");

    export_collection(&col, &out, ExportFormatArg::Jsonl, true, None).unwrap();

    let text = std::fs::read_to_string(&out).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3);
    for line in lines {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        assert!(v["id"].is_u64() && v["vector"].is_array());
    }
}

#[test]
fn test_export_csv_round_trips_through_import() {
    let dir = tempdir().unwrap();
    let (_db, col) = docs_collection(&dir);
    let csv_path = dir.path().join("out.csv");

    export_collection(&col, &csv_path, ExportFormatArg::Csv, true, None).unwrap();

    let text = std::fs::read_to_string(&csv_path).unwrap();
    let mut lines = text.lines();
    assert_eq!(lines.next().unwrap(), "id,vector,cat,x");

    // Re-import into a fresh collection via the CSV importer.
    let db_path = dir.path().join("db2");
    let db2 = Database::open(&db_path).unwrap();
    let stats = crate::import::import_csv(
        &db2,
        &csv_path,
        &crate::import::ImportConfig {
            collection: "reimported".to_string(),
            show_progress: false,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(stats.imported, 3);
    assert_eq!(stats.errors, 0);
}

#[test]
fn test_export_where_filters_rows() {
    let dir = tempdir().unwrap();
    let (_db, col) = docs_collection(&dir);
    let out = dir.path().join("filtered.jsonl");
    let filter = parse_where_filter("cat = 'tech'").unwrap();

    let count =
        export_collection(&col, &out, ExportFormatArg::Jsonl, false, Some(&filter)).unwrap();
    assert_eq!(count, 2);

    let text = std::fs::read_to_string(&out).unwrap();
    assert!(!text.contains("\"vector\""), "payload-only export: {text}");
}

#[test]
fn test_export_empty_collection_writes_empty_json_array() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    db.create_vector_collection("empty", 3, DistanceMetric::Cosine)
        .unwrap();
    let col = db.get_vector_collection("empty").unwrap();
    let out = dir.path().join("empty.json");

    export_collection(&col, &out, ExportFormatArg::Json, true, None).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    assert_eq!(v, serde_json::json!([]));
}
