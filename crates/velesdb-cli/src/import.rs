//! Bulk import module for VelesDB CLI
//!
//! Supports importing vectors from CSV and JSON Lines files.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::if_not_else,
    clippy::single_match_else,
    clippy::needless_raw_string_hashes
)]

use anyhow::{Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use serde::Deserialize;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;
use velesdb_core::{Database, DistanceMetric, Point};

/// Import configuration
pub struct ImportConfig {
    pub collection: String,
    pub dimension: Option<usize>,
    pub metric: DistanceMetric,
    pub batch_size: usize,
    pub id_column: String,
    pub vector_column: String,
    pub show_progress: bool,
}

impl Default for ImportConfig {
    fn default() -> Self {
        Self {
            collection: String::new(),
            dimension: None,
            metric: DistanceMetric::Cosine,
            batch_size: 1000,
            id_column: "id".to_string(),
            vector_column: "vector".to_string(),
            show_progress: true,
        }
    }
}

/// JSON Lines record structure
#[derive(Debug, Deserialize)]
struct JsonRecord {
    id: u64,
    vector: Vec<f32>,
    #[serde(default)]
    payload: Option<serde_json::Value>,
}

/// Import from JSON Lines file
///
/// # Performance
///
/// Optimized for high-throughput import:
/// - **Streaming parse**: Processes file line-by-line (no full file in memory)
/// - **Parallel HNSW insert**: Uses rayon for CPU-bound indexing
/// - **Batch flush**: Single I/O flush per batch
/// - Target: ~3-5K vectors/sec at 768D with batch_size=1000
pub fn import_jsonl(db: &Database, path: &Path, config: &ImportConfig) -> Result<ImportStats> {
    let file = File::open(path).context("Failed to open JSONL file")?;
    let file_size = file.metadata()?.len();

    // Perf: Streaming - count lines without loading all in memory
    let total = BufReader::with_capacity(64 * 1024, &file).lines().count();

    if total == 0 {
        anyhow::bail!("Empty file");
    }

    // Reopen file for actual processing
    let file = File::open(path)?;
    let mut reader = BufReader::with_capacity(128 * 1024, file);

    // Perf: Read first line to detect dimension
    let mut first_line = String::new();
    reader.read_line(&mut first_line)?;
    let first_record: JsonRecord =
        serde_json::from_str(&first_line).context("Failed to parse first line")?;
    let dimension = config.dimension.unwrap_or(first_record.vector.len());

    // Create or get collection
    let collection = get_or_create_collection(db, &config.collection, dimension, config.metric)?;

    let progress = create_progress_bar(total, config.show_progress);
    if config.show_progress {
        progress.set_message(format!(
            "Importing {} vectors ({:.1} MB)",
            total,
            file_size as f64 / (1024.0 * 1024.0)
        ));
    }

    let mut stats = ImportStats::default();
    let start = std::time::Instant::now();

    // Perf: Pre-allocate batch buffer
    let mut batch: Vec<Point> = Vec::with_capacity(config.batch_size);

    // Process first record (already parsed)
    if first_record.vector.len() == dimension {
        batch.push(Point::new(
            first_record.id,
            first_record.vector,
            first_record.payload,
        ));
        stats.imported += 1;
    } else {
        stats.errors += 1;
    }
    progress.inc(1);

    // Perf: Streaming parse - process line by line
    let mut line = String::with_capacity(dimension * 10); // Pre-allocate line buffer
    while reader.read_line(&mut line)? > 0 {
        match serde_json::from_str::<JsonRecord>(&line) {
            Ok(record) => {
                if record.vector.len() != dimension {
                    stats.errors += 1;
                } else {
                    batch.push(Point::new(record.id, record.vector, record.payload));
                    stats.imported += 1;

                    // Perf: Use upsert_bulk with parallel HNSW insert
                    if batch.len() >= config.batch_size {
                        collection.upsert_bulk(&batch)?;
                        batch.clear();
                    }
                }
            }
            Err(_) => {
                stats.errors += 1;
            }
        }
        line.clear(); // Reuse buffer
        progress.inc(1);
    }

    // Flush remaining batch
    if !batch.is_empty() {
        collection.upsert_bulk(&batch)?;
    }

    progress.finish_with_message("Import complete");
    stats.duration_ms = start.elapsed().as_millis() as u64;
    stats.total = total;

    Ok(stats)
}

/// Import from CSV file
///
/// # Performance
///
/// Optimized for high-throughput import:
/// - **Streaming parse**: Processes records one at a time
/// - **Parallel HNSW insert**: Uses rayon for CPU-bound indexing
/// - **Batch flush**: Single I/O flush per batch
/// - Target: ~3-5K vectors/sec at 768D with batch_size=1000
pub fn import_csv(db: &Database, path: &Path, config: &ImportConfig) -> Result<ImportStats> {
    let file = File::open(path).context("Failed to open CSV file")?;
    let file_size = file.metadata()?.len();

    // Perf: Use large buffer for reduced syscalls
    let buffered = BufReader::with_capacity(128 * 1024, file);
    let mut reader = csv::Reader::from_reader(buffered);

    // Get headers
    let headers = reader.headers()?.clone();
    let id_idx = headers
        .iter()
        .position(|h| h == config.id_column)
        .context(format!("ID column '{}' not found", config.id_column))?;
    let vector_idx = headers
        .iter()
        .position(|h| h == config.vector_column)
        .context(format!(
            "Vector column '{}' not found",
            config.vector_column
        ))?;

    // Count records for progress bar (streaming count)
    let total = reader.records().count();
    if total == 0 {
        anyhow::bail!("Empty file");
    }

    // Reopen for processing
    let file = File::open(path)?;
    let buffered = BufReader::with_capacity(128 * 1024, file);
    let mut reader = csv::Reader::from_reader(buffered);

    // Detect dimension from first record
    let first_record = reader.records().next().context("No records in CSV")??;
    let vector_str = &first_record[vector_idx];
    let first_vector = parse_vector(vector_str)?;
    let dimension = config.dimension.unwrap_or(first_vector.len());

    // Create or get collection
    let collection = get_or_create_collection(db, &config.collection, dimension, config.metric)?;

    // Reopen for final processing
    let file = File::open(path)?;
    let buffered = BufReader::with_capacity(128 * 1024, file);
    let mut reader = csv::Reader::from_reader(buffered);

    let progress = create_progress_bar(total, config.show_progress);
    if config.show_progress {
        progress.set_message(format!(
            "Importing {} vectors ({:.1} MB)",
            total,
            file_size as f64 / (1024.0 * 1024.0)
        ));
    }

    let mut stats = ImportStats::default();
    let start = std::time::Instant::now();

    // Perf: Pre-allocate batch buffer
    let mut batch: Vec<Point> = Vec::with_capacity(config.batch_size);

    // Perf: Streaming parse - process record by record
    for result in reader.records() {
        match result {
            Ok(record) => {
                let id: u64 = match record[id_idx].parse() {
                    Ok(id) => id,
                    Err(_) => {
                        stats.errors += 1;
                        progress.inc(1);
                        continue;
                    }
                };

                match parse_vector(&record[vector_idx]) {
                    Ok(vector) => {
                        if vector.len() != dimension {
                            stats.errors += 1;
                            progress.inc(1);
                            continue;
                        }
                        // Build payload from other columns
                        let mut payload = serde_json::Map::new();
                        for (i, header) in headers.iter().enumerate() {
                            if i != id_idx && i != vector_idx {
                                payload.insert(
                                    header.to_string(),
                                    serde_json::Value::String(record[i].to_string()),
                                );
                            }
                        }
                        let payload_val = if payload.is_empty() {
                            None
                        } else {
                            Some(serde_json::Value::Object(payload))
                        };

                        batch.push(Point::new(id, vector, payload_val));
                        stats.imported += 1;

                        // Perf: Use upsert_bulk with parallel HNSW insert
                        if batch.len() >= config.batch_size {
                            collection.upsert_bulk(&batch)?;
                            batch.clear();
                        }
                    }
                    Err(_) => {
                        stats.errors += 1;
                    }
                }
            }
            Err(_) => {
                stats.errors += 1;
            }
        }
        progress.inc(1);
    }

    // Flush remaining batch
    if !batch.is_empty() {
        collection.upsert_bulk(&batch)?;
    }

    progress.finish_with_message("Import complete");
    stats.duration_ms = start.elapsed().as_millis() as u64;
    stats.total = total;

    Ok(stats)
}

/// Parse vector from string (comma-separated or JSON array)
fn parse_vector(s: &str) -> Result<Vec<f32>> {
    let s = s.trim();
    if s.starts_with('[') {
        // JSON array format
        serde_json::from_str(s).context("Invalid JSON vector")
    } else {
        // Comma-separated format
        s.split(',')
            .map(|v| v.trim().parse::<f32>().context("Invalid float value"))
            .collect()
    }
}

/// Get or create collection
fn get_or_create_collection(
    db: &Database,
    name: &str,
    dimension: usize,
    metric: DistanceMetric,
) -> Result<velesdb_core::Collection> {
    if let Some(col) = db.get_collection(name) {
        Ok(col)
    } else {
        db.create_collection(name, dimension, metric)?;
        db.get_collection(name)
            .context("Failed to get created collection")
    }
}

/// Create progress bar
fn create_progress_bar(total: usize, show: bool) -> ProgressBar {
    if show {
        let pb = ProgressBar::new(total as u64);
        pb.set_style(
            ProgressStyle::default_bar()
                .template(
                    "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})",
                )
                .unwrap()
                .progress_chars("#>-"),
        );
        pb
    } else {
        ProgressBar::hidden()
    }
}

/// Import statistics
#[derive(Debug, Default)]
pub struct ImportStats {
    pub total: usize,
    pub imported: usize,
    pub errors: usize,
    pub duration_ms: u64,
}

impl ImportStats {
    /// Records per second
    #[allow(clippy::cast_precision_loss)]
    #[must_use]
    pub fn records_per_sec(&self) -> f64 {
        if self.duration_ms == 0 {
            0.0
        } else {
            (self.imported as f64) / (self.duration_ms as f64 / 1000.0)
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::cast_precision_loss,
    clippy::float_cmp,
    clippy::redundant_closure_for_method_calls
)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

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
    fn test_import_stats_default() {
        let stats = ImportStats::default();
        assert_eq!(stats.total, 0);
        assert_eq!(stats.imported, 0);
        assert_eq!(stats.errors, 0);
        assert_eq!(stats.duration_ms, 0);
    }

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
        let mut file = File::create(&jsonl_path).unwrap();
        writeln!(file, r#"{{"id": 1, "vector": [1.0, 0.0, 0.0]}}"#).unwrap();
        writeln!(file, r#"{{"id": 2, "vector": [0.0, 1.0, 0.0]}}"#).unwrap();
        writeln!(file, r#"{{"id": 3, "vector": [0.0, 0.0, 1.0]}}"#).unwrap();

        let db = Database::open(&db_path).unwrap();
        let config = ImportConfig {
            collection: "test".to_string(),
            show_progress: false,
            ..Default::default()
        };

        let stats = import_jsonl(&db, &jsonl_path, &config).unwrap();

        assert_eq!(stats.total, 3);
        assert_eq!(stats.imported, 3);
        assert_eq!(stats.errors, 0);

        let col = db.get_collection("test").unwrap();
        assert_eq!(col.len(), 3);
    }

    #[test]
    fn test_import_jsonl_with_payload() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("db");
        let jsonl_path = dir.path().join("data.jsonl");

        let mut file = File::create(&jsonl_path).unwrap();
        writeln!(
            file,
            r#"{{"id": 1, "vector": [1.0, 0.0, 0.0], "payload": {{"title": "Doc 1"}}}}"#
        )
        .unwrap();
        writeln!(
            file,
            r#"{{"id": 2, "vector": [0.0, 1.0, 0.0], "payload": {{"title": "Doc 2"}}}}"#
        )
        .unwrap();

        let db = Database::open(&db_path).unwrap();
        let config = ImportConfig {
            collection: "test".to_string(),
            show_progress: false,
            ..Default::default()
        };

        let stats = import_jsonl(&db, &jsonl_path, &config).unwrap();

        assert_eq!(stats.imported, 2);

        let col = db.get_collection("test").unwrap();
        let points = col.get(&[1, 2]);
        assert!(points[0].as_ref().unwrap().payload.is_some());
    }

    #[test]
    fn test_import_jsonl_with_errors() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("db");
        let jsonl_path = dir.path().join("data.jsonl");

        let mut file = File::create(&jsonl_path).unwrap();
        writeln!(file, r#"{{"id": 1, "vector": [1.0, 0.0, 0.0]}}"#).unwrap();
        writeln!(file, r#"invalid json line"#).unwrap();
        writeln!(file, r#"{{"id": 3, "vector": [0.0, 0.0, 1.0]}}"#).unwrap();

        let db = Database::open(&db_path).unwrap();
        let config = ImportConfig {
            collection: "test".to_string(),
            show_progress: false,
            ..Default::default()
        };

        let stats = import_jsonl(&db, &jsonl_path, &config).unwrap();

        assert_eq!(stats.total, 3);
        assert_eq!(stats.imported, 2);
        assert_eq!(stats.errors, 1);
    }

    #[test]
    fn test_import_jsonl_dimension_mismatch() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("db");
        let jsonl_path = dir.path().join("data.jsonl");

        let mut file = File::create(&jsonl_path).unwrap();
        writeln!(file, r#"{{"id": 1, "vector": [1.0, 0.0, 0.0]}}"#).unwrap();
        writeln!(file, r#"{{"id": 2, "vector": [0.0, 1.0]}}"#).unwrap(); // Wrong dimension
        writeln!(file, r#"{{"id": 3, "vector": [0.0, 0.0, 1.0]}}"#).unwrap();

        let db = Database::open(&db_path).unwrap();
        let config = ImportConfig {
            collection: "test".to_string(),
            show_progress: false,
            ..Default::default()
        };

        let stats = import_jsonl(&db, &jsonl_path, &config).unwrap();

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

        let mut file = File::create(&csv_path).unwrap();
        writeln!(file, "id,vector").unwrap();
        writeln!(file, "1,\"[1.0, 0.0, 0.0]\"").unwrap();
        writeln!(file, "2,\"[0.0, 1.0, 0.0]\"").unwrap();
        writeln!(file, "3,\"[0.0, 0.0, 1.0]\"").unwrap();

        let db = Database::open(&db_path).unwrap();
        let config = ImportConfig {
            collection: "test".to_string(),
            show_progress: false,
            ..Default::default()
        };

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

        let mut file = File::create(&csv_path).unwrap();
        writeln!(file, "id,vector").unwrap();
        writeln!(file, "1,\"1.0,0.0,0.0\"").unwrap();
        writeln!(file, "2,\"0.0,1.0,0.0\"").unwrap();

        let db = Database::open(&db_path).unwrap();
        let config = ImportConfig {
            collection: "test".to_string(),
            show_progress: false,
            ..Default::default()
        };

        let stats = import_csv(&db, &csv_path, &config).unwrap();

        assert_eq!(stats.imported, 2);
    }

    #[test]
    fn test_import_csv_with_extra_columns() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("db");
        let csv_path = dir.path().join("data.csv");

        let mut file = File::create(&csv_path).unwrap();
        writeln!(file, "id,vector,title,category").unwrap();
        writeln!(file, "1,\"[1.0, 0.0, 0.0]\",Document 1,tech").unwrap();
        writeln!(file, "2,\"[0.0, 1.0, 0.0]\",Document 2,science").unwrap();

        let db = Database::open(&db_path).unwrap();
        let config = ImportConfig {
            collection: "test".to_string(),
            show_progress: false,
            ..Default::default()
        };

        let stats = import_csv(&db, &csv_path, &config).unwrap();

        assert_eq!(stats.imported, 2);

        // Extra columns should be stored as payload
        let col = db.get_collection("test").unwrap();
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

        let mut file = File::create(&csv_path).unwrap();
        writeln!(file, "doc_id,embedding").unwrap();
        writeln!(file, "1,\"[1.0, 0.0, 0.0]\"").unwrap();
        writeln!(file, "2,\"[0.0, 1.0, 0.0]\"").unwrap();

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
        let col = db.get_collection("documents").unwrap();
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
        let col = db.get_collection("products").unwrap();
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
        let col = db.get_collection("data").unwrap();
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

        let col = db.get_collection("gpt_embeddings").unwrap();
        assert_eq!(col.config().dimension, 1536);
    }
}
