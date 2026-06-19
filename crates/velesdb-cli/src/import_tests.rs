//! Tests for CLI import module (EPIC-061/US-007 refactoring).
//!
//! Extracted from import.rs to improve modularity.

use super::*;
use std::io::Write;
use std::path::Path;
use tempfile::tempdir;

/// Write each line (with a trailing newline) to a fresh file at `path`.
///
/// Shared fixture for the JSONL/CSV import tests, which all materialize a
/// small data file as a list of text lines before importing it.
fn write_lines(path: &Path, lines: &[&str]) {
    let mut file = File::create(path).unwrap();
    for line in lines {
        writeln!(file, "{line}").unwrap();
    }
}

/// Open a DB at `db_path` and build the default import config targeting the
/// `"test"` collection with progress disabled — the setup every basic
/// import test shares verbatim.
fn open_db_and_test_config(db_path: &Path) -> (Database, ImportConfig) {
    let db = Database::open(db_path).unwrap();
    let config = ImportConfig {
        collection: "test".to_string(),
        show_progress: false,
        ..Default::default()
    };
    (db, config)
}

/// Opens a fresh DB with the standard test config and imports `jsonl_path`,
/// returning the DB handle and the resulting stats. Shared by the basic JSONL
/// import tests (default `collection="test"`, progress off).
fn run_jsonl_import(db_path: &Path, jsonl_path: &Path) -> (Database, ImportStats) {
    let (db, config) = open_db_and_test_config(db_path);
    let stats = import_jsonl(&db, jsonl_path, &config).unwrap();
    (db, stats)
}

// =========================================================================
// Unit tests for parse_vector
// =========================================================================

#[test]
fn test_parse_vector_json_array() {
    let input = "[1.0, 2.0, 3.0]";
    let result = parse_vector(input).unwrap();
    assert_eq!(result, vec![1.0, 2.0, 3.0]);
}

#[test]
fn test_parse_vector_json_array_with_whitespace() {
    let input = "  [ 1.0 , 2.0 , 3.0 ]  ";
    let result = parse_vector(input).unwrap();
    assert_eq!(result, vec![1.0, 2.0, 3.0]);
}

#[test]
fn test_parse_vector_comma_separated() {
    let input = "1.0, 2.0, 3.0";
    let result = parse_vector(input).unwrap();
    assert_eq!(result, vec![1.0, 2.0, 3.0]);
}

#[test]
fn test_parse_vector_comma_separated_no_spaces() {
    let input = "1.0,2.0,3.0";
    let result = parse_vector(input).unwrap();
    assert_eq!(result, vec![1.0, 2.0, 3.0]);
}

#[test]
fn test_parse_vector_invalid_json() {
    let input = "[1.0, 2.0, invalid]";
    let result = parse_vector(input);
    assert!(result.is_err());
}

#[test]
fn test_parse_vector_invalid_csv() {
    let input = "1.0, not_a_number, 3.0";
    let result = parse_vector(input);
    assert!(result.is_err());
}

// =========================================================================
// Unit tests for ImportStats
// =========================================================================

#[test]
fn test_import_stats_records_per_sec() {
    let stats = ImportStats {
        total: 100,
        imported: 1000,
        errors: 0,
        duration_ms: 500,
    };
    assert!((stats.records_per_sec() - 2000.0).abs() < 0.001);
}

#[test]
fn test_import_stats_records_per_sec_zero_duration() {
    let stats = ImportStats {
        total: 100,
        imported: 1000,
        errors: 0,
        duration_ms: 0,
    };
    assert_eq!(stats.records_per_sec(), 0.0);
}

// =========================================================================
// Unit tests for ImportConfig
// =========================================================================

#[test]
fn test_import_config_default() {
    let config = ImportConfig::default();
    assert!(config.collection.is_empty());
    assert!(config.dimension.is_none());
    assert_eq!(config.batch_size, 1000);
    assert_eq!(config.id_column, "id");
    assert_eq!(config.vector_column, "vector");
    assert!(config.show_progress);
}

// =========================================================================
// Integration tests for JSONL import
// =========================================================================

#[test]
fn test_import_jsonl_basic() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("db");
    let jsonl_path = dir.path().join("data.jsonl");

    // Create test JSONL file
    write_lines(
        &jsonl_path,
        &[
            r#"{"id": 1, "vector": [1.0, 0.0, 0.0]}"#,
            r#"{"id": 2, "vector": [0.0, 1.0, 0.0]}"#,
            r#"{"id": 3, "vector": [0.0, 0.0, 1.0]}"#,
        ],
    );

    let (db, stats) = run_jsonl_import(&db_path, &jsonl_path);

    assert_eq!(stats.total, 3);
    assert_eq!(stats.imported, 3);
    assert_eq!(stats.errors, 0);

    let col = db.get_vector_collection("test").unwrap();
    assert_eq!(col.len(), 3);
}

#[test]
fn test_import_jsonl_with_payload() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("db");
    let jsonl_path = dir.path().join("data.jsonl");

    write_lines(
        &jsonl_path,
        &[
            r#"{"id": 1, "vector": [1.0, 0.0, 0.0], "payload": {"title": "Doc 1"}}"#,
            r#"{"id": 2, "vector": [0.0, 1.0, 0.0], "payload": {"title": "Doc 2"}}"#,
        ],
    );

    let (db, stats) = run_jsonl_import(&db_path, &jsonl_path);

    assert_eq!(stats.imported, 2);

    let col = db.get_vector_collection("test").unwrap();
    let points = col.get(&[1, 2]);
    assert!(points[0].as_ref().unwrap().payload.is_some());
}

#[test]
fn test_import_jsonl_with_errors() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("db");
    let jsonl_path = dir.path().join("data.jsonl");

    write_lines(
        &jsonl_path,
        &[
            r#"{"id": 1, "vector": [1.0, 0.0, 0.0]}"#,
            "invalid json line",
            r#"{"id": 3, "vector": [0.0, 0.0, 1.0]}"#,
        ],
    );

    let (_db, stats) = run_jsonl_import(&db_path, &jsonl_path);

    assert_eq!(stats.total, 3);
    assert_eq!(stats.imported, 2);
    assert_eq!(stats.errors, 1);
}

#[test]
fn test_import_jsonl_dimension_mismatch() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("db");
    let jsonl_path = dir.path().join("data.jsonl");

    write_lines(
        &jsonl_path,
        &[
            r#"{"id": 1, "vector": [1.0, 0.0, 0.0]}"#,
            r#"{"id": 2, "vector": [0.0, 1.0]}"#, // Wrong dimension
            r#"{"id": 3, "vector": [0.0, 0.0, 1.0]}"#,
        ],
    );

    let (_db, stats) = run_jsonl_import(&db_path, &jsonl_path);

    assert_eq!(stats.imported, 2);
    assert_eq!(stats.errors, 1);
}

// =========================================================================
// Integration tests for CSV import
// =========================================================================

#[test]
fn test_import_csv_basic() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("db");
    let csv_path = dir.path().join("data.csv");

    write_lines(
        &csv_path,
        &[
            "id,vector",
            "1,\"[1.0, 0.0, 0.0]\"",
            "2,\"[0.0, 1.0, 0.0]\"",
            "3,\"[0.0, 0.0, 1.0]\"",
        ],
    );

    let (db, config) = open_db_and_test_config(&db_path);

    let stats = import_csv(&db, &csv_path, &config).unwrap();

    assert_eq!(stats.total, 3);
    assert_eq!(stats.imported, 3);
    assert_eq!(stats.errors, 0);
}

#[test]
fn test_import_csv_comma_separated_vector() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("db");
    let csv_path = dir.path().join("data.csv");

    write_lines(
        &csv_path,
        &["id,vector", "1,\"1.0,0.0,0.0\"", "2,\"0.0,1.0,0.0\""],
    );

    let (db, config) = open_db_and_test_config(&db_path);

    let stats = import_csv(&db, &csv_path, &config).unwrap();

    assert_eq!(stats.imported, 2);
}

#[test]
fn test_import_csv_with_extra_columns() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("db");
    let csv_path = dir.path().join("data.csv");

    write_lines(
        &csv_path,
        &[
            "id,vector,title,category",
            "1,\"[1.0, 0.0, 0.0]\",Document 1,tech",
            "2,\"[0.0, 1.0, 0.0]\",Document 2,science",
        ],
    );

    let (db, config) = open_db_and_test_config(&db_path);

    let stats = import_csv(&db, &csv_path, &config).unwrap();

    assert_eq!(stats.imported, 2);

    // Extra columns should be stored as payload
    let col = db.get_vector_collection("test").unwrap();
    let points = col.get(&[1]);
    let payload = points[0].as_ref().unwrap().payload.as_ref().unwrap();
    assert_eq!(payload["title"], "Document 1");
    assert_eq!(payload["category"], "tech");
}

#[test]
fn test_import_csv_custom_columns() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("db");
    let csv_path = dir.path().join("data.csv");

    write_lines(
        &csv_path,
        &[
            "doc_id,embedding",
            "1,\"[1.0, 0.0, 0.0]\"",
            "2,\"[0.0, 1.0, 0.0]\"",
        ],
    );

    let db = Database::open(&db_path).unwrap();
    let config = ImportConfig {
        collection: "test".to_string(),
        id_column: "doc_id".to_string(),
        vector_column: "embedding".to_string(),
        show_progress: false,
        ..Default::default()
    };

    let stats = import_csv(&db, &csv_path, &config).unwrap();

    assert_eq!(stats.imported, 2);
}

// =========================================================================
// Integration tests for VRB1 binary import
// =========================================================================

#[test]
fn test_import_raw_bulk_basic() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("db");
    let bin_path = dir.path().join("data.bin");

    // Encode a small VRB1 buffer via the shared core codec.
    let ids = [1u64, 2, 3];
    let vectors = [
        1.0f32, 0.0, 0.0, // id 1
        0.0, 1.0, 0.0, // id 2
        0.0, 0.0, 1.0, // id 3
    ];
    let bytes = velesdb_core::wire::vrb1::encode(&ids, &vectors, 3);
    std::fs::write(&bin_path, &bytes).unwrap();

    let db = Database::open(&db_path).unwrap();
    let config = ImportConfig {
        collection: "test".to_string(),
        show_progress: false,
        ..Default::default()
    };

    let stats = import_raw_bulk(&db, &bin_path, &config).unwrap();

    assert_eq!(stats.total, 3);
    assert_eq!(stats.imported, 3);
    assert_eq!(stats.errors, 0);

    // The imported vectors must be searchable.
    let col = db.get_vector_collection("test").unwrap();
    assert_eq!(col.len(), 3);
    let results = col.search(&[1.0, 0.0, 0.0], 1).unwrap();
    assert_eq!(results[0].point.id, 1);
}

#[test]
fn test_import_raw_bulk_invalid_magic() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("db");
    let bin_path = dir.path().join("bad.bin");

    let mut bytes = velesdb_core::wire::vrb1::encode(&[1u64], &[0.0f32, 0.0, 0.0], 3);
    bytes[0] = b'X'; // corrupt the magic
    std::fs::write(&bin_path, &bytes).unwrap();

    let db = Database::open(&db_path).unwrap();
    let config = ImportConfig {
        collection: "test".to_string(),
        show_progress: false,
        ..Default::default()
    };

    assert!(import_raw_bulk(&db, &bin_path, &config).is_err());
}

// =========================================================================
// Integration tests for typical usage scenarios
// =========================================================================

#[test]
fn test_scenario_rag_document_import() {
    // Simulates importing embeddings for a RAG application
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("rag_db");
    let jsonl_path = dir.path().join("embeddings.jsonl");

    // Create embeddings file (768D like BERT)
    let mut file = File::create(&jsonl_path).unwrap();
    for i in 0..100 {
        let vector: Vec<f32> = (0..768).map(|j| ((i + j) % 100) as f32 / 100.0).collect();
        let payload = serde_json::json!({
            "content": format!("Document {} content about topic {}", i, i % 10),
            "source": format!("file_{}.txt", i),
            "chunk_id": i
        });
        writeln!(
            file,
            r#"{{"id": {}, "vector": {:?}, "payload": {}}}"#,
            i, vector, payload
        )
        .unwrap();
    }

    let db = Database::open(&db_path).unwrap();
    let config = ImportConfig {
        collection: "documents".to_string(),
        batch_size: 50,
        show_progress: false,
        ..Default::default()
    };

    let stats = import_jsonl(&db, &jsonl_path, &config).unwrap();

    assert_eq!(stats.imported, 100);
    assert!(stats.duration_ms > 0);
    assert!(stats.records_per_sec() > 0.0);

    // Verify search works
    let col = db.get_vector_collection("documents").unwrap();
    let query: Vec<f32> = (0..768).map(|i| i as f32 / 100.0).collect();
    let results = col.search(&query, 10).unwrap();
    assert_eq!(results.len(), 10);
}

#[test]
fn test_scenario_product_catalog_import() {
    // Simulates importing product embeddings from CSV
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("catalog_db");
    let csv_path = dir.path().join("products.csv");

    let mut file = File::create(&csv_path).unwrap();
    writeln!(file, "id,vector,name,price,category").unwrap();
    for i in 0..50 {
        let vector: Vec<f32> = (0..128).map(|j| ((i + j) % 50) as f32 / 50.0).collect();
        writeln!(
            file,
            "{},\"{:?}\",Product {},{:.2},Category {}",
            i,
            vector,
            i,
            (i as f32) * 9.99,
            i % 5
        )
        .unwrap();
    }

    let db = Database::open(&db_path).unwrap();
    let config = ImportConfig {
        collection: "products".to_string(),
        batch_size: 20,
        show_progress: false,
        ..Default::default()
    };

    let stats = import_csv(&db, &csv_path, &config).unwrap();

    assert_eq!(stats.imported, 50);

    // Verify metadata is preserved
    let col = db.get_vector_collection("products").unwrap();
    let points = col.get(&[0]);
    let payload = points[0].as_ref().unwrap().payload.as_ref().unwrap();
    assert_eq!(payload["name"], "Product 0");
    assert_eq!(payload["category"], "Category 0");
}

#[test]
fn test_scenario_incremental_import() {
    // Simulates importing data in multiple batches within same session
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("incremental_db");
    let jsonl_path1 = dir.path().join("batch1.jsonl");
    let jsonl_path2 = dir.path().join("batch2.jsonl");

    // Create both files
    let mut file1 = File::create(&jsonl_path1).unwrap();
    for i in 0..50 {
        let vector: Vec<f32> = (0..64).map(|j| ((i + j) % 50) as f32 / 50.0).collect();
        writeln!(file1, r#"{{"id": {}, "vector": {:?}}}"#, i, vector).unwrap();
    }
    drop(file1);

    let mut file2 = File::create(&jsonl_path2).unwrap();
    for i in 50..100 {
        let vector: Vec<f32> = (0..64).map(|j| ((i + j) % 50) as f32 / 50.0).collect();
        writeln!(file2, r#"{{"id": {}, "vector": {:?}}}"#, i, vector).unwrap();
    }
    drop(file2);

    // Import both batches in same session
    let db = Database::open(&db_path).unwrap();
    let config = ImportConfig {
        collection: "data".to_string(),
        show_progress: false,
        ..Default::default()
    };

    // First batch
    let stats1 = import_jsonl(&db, &jsonl_path1, &config).unwrap();
    assert_eq!(stats1.imported, 50);

    // Second batch (same collection)
    let stats2 = import_jsonl(&db, &jsonl_path2, &config).unwrap();
    assert_eq!(stats2.imported, 50);

    // Verify final state
    let col = db.get_vector_collection("data").unwrap();
    assert_eq!(col.len(), 100);

    // Verify random access works across both batches
    let points = col.get(&[0, 50, 99]);
    assert!(points.iter().all(|p| p.is_some()));
}

#[test]
fn test_scenario_large_dimension_vectors() {
    // Simulates importing high-dimensional vectors (1536D like GPT-4)
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("gpt_db");
    let jsonl_path = dir.path().join("gpt_embeddings.jsonl");

    let mut file = File::create(&jsonl_path).unwrap();
    for i in 0..20 {
        let vector: Vec<f32> = (0..1536).map(|j| ((i + j) % 100) as f32 / 100.0).collect();
        writeln!(file, r#"{{"id": {}, "vector": {:?}}}"#, i, vector).unwrap();
    }

    let db = Database::open(&db_path).unwrap();
    let config = ImportConfig {
        collection: "gpt_embeddings".to_string(),
        dimension: Some(1536),
        show_progress: false,
        ..Default::default()
    };

    let stats = import_jsonl(&db, &jsonl_path, &config).unwrap();

    assert_eq!(stats.imported, 20);

    let col = db.get_vector_collection("gpt_embeddings").unwrap();
    assert_eq!(col.config().dimension, 1536);
}
