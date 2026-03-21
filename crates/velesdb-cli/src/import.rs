//! Bulk import module for VelesDB CLI
//!
//! Supports importing vectors from CSV and JSON Lines files.
//!
//! The batch-accumulate-flush loop is shared between `import_jsonl` and
//! `import_csv` via [`crate::helpers::BatchImporter`].

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::if_not_else,
    clippy::single_match_else,
    clippy::needless_raw_string_hashes
)]

use anyhow::{Context, Result};
use serde::Deserialize;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;
use velesdb_core::{Database, DistanceMetric, Point, StorageMode};

use crate::helpers::{self, BatchImporter};

/// Import configuration
pub struct ImportConfig {
    pub collection: String,
    pub dimension: Option<usize>,
    pub metric: DistanceMetric,
    pub storage_mode: StorageMode,
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
            storage_mode: StorageMode::Full,
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
    let collection = get_or_create_collection(
        db,
        &config.collection,
        dimension,
        config.metric,
        config.storage_mode,
    )?;

    let progress = helpers::create_progress_bar(total, config.show_progress);
    helpers::set_import_message(&progress, total, file_size, config.show_progress);

    let start = std::time::Instant::now();
    let mut importer = BatchImporter::new(&collection, config.batch_size);

    // Process first record (already parsed)
    if first_record.vector.len() == dimension {
        importer.push(Point::new(
            first_record.id,
            first_record.vector,
            first_record.payload,
        ))?;
    } else {
        importer.record_error();
    }
    progress.inc(1);

    // Perf: Streaming parse - process line by line
    let mut line = String::with_capacity(dimension * 10); // Pre-allocate line buffer
    while reader.read_line(&mut line)? > 0 {
        match serde_json::from_str::<JsonRecord>(&line) {
            Ok(record) => {
                if record.vector.len() != dimension {
                    importer.record_error();
                } else {
                    importer.push(Point::new(record.id, record.vector, record.payload))?;
                }
            }
            Err(_) => {
                importer.record_error();
            }
        }
        line.clear(); // Reuse buffer
        progress.inc(1);
    }

    let acc = importer.flush()?;
    progress.finish_with_message("Import complete");

    Ok(ImportStats {
        total,
        imported: acc.imported,
        errors: acc.errors,
        duration_ms: start.elapsed().as_millis() as u64,
    })
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
#[allow(clippy::too_many_lines)]
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
    let collection = get_or_create_collection(
        db,
        &config.collection,
        dimension,
        config.metric,
        config.storage_mode,
    )?;

    // Reopen for final processing
    let file = File::open(path)?;
    let buffered = BufReader::with_capacity(128 * 1024, file);
    let mut reader = csv::Reader::from_reader(buffered);

    let progress = helpers::create_progress_bar(total, config.show_progress);
    helpers::set_import_message(&progress, total, file_size, config.show_progress);

    let start = std::time::Instant::now();
    let mut importer = BatchImporter::new(&collection, config.batch_size);

    // Perf: Streaming parse - process record by record
    for result in reader.records() {
        match result {
            Ok(record) => {
                process_csv_record(
                    &record,
                    &headers,
                    id_idx,
                    vector_idx,
                    dimension,
                    &mut importer,
                )?;
            }
            Err(_) => {
                importer.record_error();
            }
        }
        progress.inc(1);
    }

    let acc = importer.flush()?;
    progress.finish_with_message("Import complete");

    Ok(ImportStats {
        total,
        imported: acc.imported,
        errors: acc.errors,
        duration_ms: start.elapsed().as_millis() as u64,
    })
}

/// Process a single CSV record and push it into the batch importer.
fn process_csv_record(
    record: &csv::StringRecord,
    headers: &csv::StringRecord,
    id_idx: usize,
    vector_idx: usize,
    dimension: usize,
    importer: &mut BatchImporter<'_>,
) -> Result<()> {
    let id: u64 = match record[id_idx].parse() {
        Ok(id) => id,
        Err(_) => {
            importer.record_error();
            return Ok(());
        }
    };

    match parse_vector(&record[vector_idx]) {
        Ok(vector) => {
            if vector.len() != dimension {
                importer.record_error();
                return Ok(());
            }
            let payload_val = build_csv_payload(headers, record, id_idx, vector_idx);
            importer.push(Point::new(id, vector, payload_val))?;
        }
        Err(_) => {
            importer.record_error();
        }
    }
    Ok(())
}

/// Builds a JSON payload from non-id, non-vector CSV columns.
fn build_csv_payload(
    headers: &csv::StringRecord,
    record: &csv::StringRecord,
    id_idx: usize,
    vector_idx: usize,
) -> Option<serde_json::Value> {
    let mut payload = serde_json::Map::new();
    for (i, header) in headers.iter().enumerate() {
        if i != id_idx && i != vector_idx {
            payload.insert(
                header.to_string(),
                serde_json::Value::String(record[i].to_string()),
            );
        }
    }
    if payload.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(payload))
    }
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
    storage_mode: StorageMode,
) -> Result<velesdb_core::VectorCollection> {
    if let Some(col) = db.get_vector_collection(name) {
        Ok(col)
    } else {
        db.create_vector_collection_with_options(name, dimension, metric, storage_mode)?;
        db.get_vector_collection(name)
            .context("Failed to get created collection")
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

// NOTE: Tests moved to import_tests.rs (EPIC-061/US-007 refactoring)
#[cfg(test)]
#[allow(
    clippy::cast_precision_loss,
    clippy::float_cmp,
    clippy::redundant_closure_for_method_calls
)]
#[path = "import_tests.rs"]
mod import_tests;
