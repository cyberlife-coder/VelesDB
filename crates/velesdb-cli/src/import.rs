//! Bulk import module for VelesDB CLI
//!
//! Supports importing vectors from CSV and JSON Lines files.

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
pub fn import_jsonl(db: &Database, path: &Path, config: &ImportConfig) -> Result<ImportStats> {
    let file = File::open(path).context("Failed to open JSONL file")?;
    let reader = BufReader::new(file);
    let lines: Vec<_> = reader.lines().collect::<std::io::Result<_>>()?;
    let total = lines.len();

    if total == 0 {
        anyhow::bail!("Empty file");
    }

    // Detect dimension from first record
    let first_record: JsonRecord =
        serde_json::from_str(&lines[0]).context("Failed to parse first line")?;
    let dimension = config.dimension.unwrap_or(first_record.vector.len());

    // Create or get collection
    let collection = get_or_create_collection(db, &config.collection, dimension, config.metric)?;

    let progress = create_progress_bar(total, config.show_progress);
    let mut stats = ImportStats::default();
    let start = std::time::Instant::now();

    let mut batch: Vec<Point> = Vec::with_capacity(config.batch_size);

    for line in lines {
        match serde_json::from_str::<JsonRecord>(&line) {
            Ok(record) => {
                if record.vector.len() != dimension {
                    stats.errors += 1;
                    progress.inc(1);
                    continue;
                }
                batch.push(Point::new(record.id, record.vector, record.payload));
                stats.imported += 1;

                if batch.len() >= config.batch_size {
                    collection.upsert(std::mem::take(&mut batch))?;
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
        collection.upsert(batch)?;
    }

    progress.finish_with_message("Import complete");
    stats.duration_ms = start.elapsed().as_millis() as u64;
    stats.total = total;

    Ok(stats)
}

/// Import from CSV file
pub fn import_csv(db: &Database, path: &Path, config: &ImportConfig) -> Result<ImportStats> {
    let file = File::open(path).context("Failed to open CSV file")?;
    let mut reader = csv::Reader::from_reader(file);

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

    // Count records for progress bar
    let total = reader.records().count();
    if total == 0 {
        anyhow::bail!("Empty file");
    }

    let file = File::open(path)?;
    let mut reader = csv::Reader::from_reader(file);

    // Detect dimension from first record
    let first_record = reader.records().next().context("No records in CSV")??;
    let vector_str = &first_record[vector_idx];
    let first_vector = parse_vector(vector_str)?;
    let dimension = config.dimension.unwrap_or(first_vector.len());

    // Create or get collection
    let collection = get_or_create_collection(db, &config.collection, dimension, config.metric)?;

    // Reset reader
    let file = File::open(path)?;
    let mut reader = csv::Reader::from_reader(file);

    let progress = create_progress_bar(total, config.show_progress);
    let mut stats = ImportStats::default();
    let start = std::time::Instant::now();

    let mut batch: Vec<Point> = Vec::with_capacity(config.batch_size);

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

                        if batch.len() >= config.batch_size {
                            collection.upsert(std::mem::take(&mut batch))?;
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
        collection.upsert(batch)?;
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
    pub fn records_per_sec(&self) -> f64 {
        if self.duration_ms == 0 {
            0.0
        } else {
            (self.imported as f64) / (self.duration_ms as f64 / 1000.0)
        }
    }
}
