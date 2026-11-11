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

use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use velesdb_core::{Database, DistanceMetric, Point, StorageMode};

use crate::helpers::{self, BatchImporter};

#[path = "import_validate.rs"]
mod validate;
use validate::*;

/// How many rejected records [`ImportStats::error_details`] keeps in memory.
/// Past the cap, records are still counted in `errors` and still land in the
/// `--error-report` file — only the in-memory list is bounded.
const MAX_REPORTED_ERRORS: usize = 4096;

/// What `data import` does when an imported id already exists in the
/// collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OnConflict {
    /// Update the existing point (historical behaviour).
    #[default]
    Upsert,
    /// Leave the existing point untouched; the record is counted as skipped.
    Skip,
    /// Refuse the import when an id already exists.
    Fail,
}

/// One rejected record: its 1-based ordinal in the file (line for JSONL,
/// record for CSV) and the reason it was refused.
#[derive(Debug)]
pub struct ImportError {
    /// Line (JSONL) or record ordinal (CSV) of the bad record, 1-based.
    pub line: usize,
    /// Why the record was rejected.
    pub reason: String,
}

/// Import configuration
pub struct ImportConfig {
    pub collection: String,
    pub dimension: Option<usize>,
    /// `None` when the caller did not name a metric: an existing
    /// collection's own metric is then kept, and a new one gets cosine.
    pub metric: Option<DistanceMetric>,
    pub storage_mode: StorageMode,
    pub batch_size: usize,
    pub id_column: String,
    pub vector_column: String,
    pub show_progress: bool,
    /// Validate and count without writing (`--dry-run`).
    pub dry_run: bool,
    /// Policy for ids already present in the collection.
    pub on_conflict: OnConflict,
    /// Keep importing after a bad record (`--continue-on-error`); when false,
    /// the first invalid record aborts before anything is written.
    pub continue_on_error: bool,
    /// Optional JSONL file receiving every rejected record.
    pub error_report: Option<PathBuf>,
}

impl Default for ImportConfig {
    fn default() -> Self {
        Self {
            collection: String::new(),
            dimension: None,
            metric: None,
            storage_mode: StorageMode::Full,
            batch_size: 1000,
            id_column: "id".to_string(),
            vector_column: "vector".to_string(),
            show_progress: true,
            dry_run: false,
            on_conflict: OnConflict::Upsert,
            continue_on_error: false,
            error_report: None,
        }
    }
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
    let start = std::time::Instant::now();
    let file_size = File::open(path)
        .context("Failed to open JSONL file")?
        .metadata()?
        .len();

    let all_errors = config.continue_on_error || config.dry_run;
    let validation = validate_jsonl(path, config, all_errors)?;
    if validation.records == 0 {
        anyhow::bail!("Empty file");
    }
    fail_fast_on_first_error(&validation, config)?;

    if config.dry_run {
        return dry_run_stats(db, config, validation, start);
    }

    let dimension = resolve_dimension(&validation, config)?;
    let collection = get_or_create_collection(
        db,
        &config.collection,
        dimension,
        config.metric,
        config.storage_mode,
    )?;
    let conflicts = resolve_conflicts(&collection, config, &validation.ids)?;

    let total = validation.records;
    let progress = helpers::create_progress_bar(total, config.show_progress);
    helpers::set_import_message(&progress, total, file_size, config.show_progress);

    let mut importer = BatchImporter::new(&collection, config.batch_size);
    let mut report = open_error_report(config)?;
    let mut details: Vec<ImportError> = Vec::new();
    let mut seen = HashSet::with_capacity(validation.ids.len());
    let mut skipped = 0usize;

    let mut reader = BufReader::with_capacity(128 * 1024, File::open(path)?);
    let mut line = String::with_capacity(dimension * 10);
    let mut ordinal = 0usize;
    while reader.read_line(&mut line)? > 0 {
        ordinal += 1;
        match serde_json::from_str::<JsonRecord>(&line) {
            Ok(rec) if check_dimension(rec.vector.len(), Some(dimension), None).is_none() => {
                if take_conflict(
                    rec.id,
                    config.on_conflict,
                    &conflicts,
                    &mut seen,
                    &mut skipped,
                ) {
                    importer.push(Point::new(rec.id, rec.vector, rec.payload))?;
                }
            }
            Ok(_) => record_error(
                &mut details,
                &mut report,
                &mut importer,
                ImportError {
                    line: ordinal,
                    reason: format!("dimension does not match {dimension}"),
                },
            )?,
            Err(e) => record_error(
                &mut details,
                &mut report,
                &mut importer,
                ImportError {
                    line: ordinal,
                    reason: format!("invalid JSON: {e}"),
                },
            )?,
        }
        line.clear();
        progress.inc(1);
    }

    finalize_import(
        importer,
        &progress,
        total,
        start,
        skipped,
        conflicts.len(),
        details,
    )
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
    let start = std::time::Instant::now();
    let file_size = path.metadata()?.len();

    let all_errors = config.continue_on_error || config.dry_run;
    let validation = validate_csv(path, config, all_errors)?;
    if validation.records == 0 {
        anyhow::bail!("Empty file");
    }
    fail_fast_on_first_error(&validation, config)?;

    if config.dry_run {
        return dry_run_stats(db, config, validation, start);
    }

    let dimension = resolve_dimension(&validation, config)?;
    let collection = get_or_create_collection(
        db,
        &config.collection,
        dimension,
        config.metric,
        config.storage_mode,
    )?;
    let conflicts = resolve_conflicts(&collection, config, &validation.ids)?;

    let (mut reader, id_idx, vector_idx) = open_csv(path, config)?;
    let headers = reader.headers()?.clone();
    let total = validation.records;
    let progress = helpers::create_progress_bar(total, config.show_progress);
    helpers::set_import_message(&progress, total, file_size, config.show_progress);

    let mut importer = BatchImporter::new(&collection, config.batch_size);
    let mut report = open_error_report(config)?;
    let mut details: Vec<ImportError> = Vec::new();
    let mut seen = HashSet::with_capacity(validation.ids.len());
    let mut skipped = 0usize;

    for (idx, result) in reader.records().enumerate() {
        let ordinal = idx + 1;
        match result {
            Ok(record) => {
                match csv_record_check(&record, id_idx, vector_idx, Some(dimension), None) {
                    Ok((id, vector)) => {
                        if take_conflict(
                            id,
                            config.on_conflict,
                            &conflicts,
                            &mut seen,
                            &mut skipped,
                        ) {
                            let payload_val =
                                build_csv_payload(&headers, &record, id_idx, vector_idx);
                            importer.push(Point::new(id, vector, payload_val))?;
                        }
                    }
                    Err(reason) => record_error(
                        &mut details,
                        &mut report,
                        &mut importer,
                        ImportError {
                            line: ordinal,
                            reason,
                        },
                    )?,
                }
            }
            Err(e) => record_error(
                &mut details,
                &mut report,
                &mut importer,
                ImportError {
                    line: ordinal,
                    reason: format!("invalid CSV record: {e}"),
                },
            )?,
        }
        progress.inc(1);
    }

    finalize_import(
        importer,
        &progress,
        total,
        start,
        skipped,
        conflicts.len(),
        details,
    )
}

/// Flush the batch importer, finish the progress bar, and build [`ImportStats`].
///
/// Shared finalization tail for `import_jsonl` and `import_csv`.
fn finalize_import(
    importer: BatchImporter<'_>,
    progress: &indicatif::ProgressBar,
    total: usize,
    start: std::time::Instant,
    skipped: usize,
    conflicts: usize,
    error_details: Vec<ImportError>,
) -> Result<ImportStats> {
    let acc = importer.flush()?;
    progress.finish_with_message("Import complete");

    Ok(ImportStats {
        total,
        imported: acc.imported,
        errors: acc.errors,
        skipped,
        conflicts,
        error_details,
        duration_ms: start.elapsed().as_millis() as u64,
    })
}

/// Import from a VRB1 binary file (`.bin` / `.vrb1`).
///
/// Reads the whole file, decodes the shared VRB1 wire format, and forwards the
/// flat ids/vectors to the zero-copy `upsert_bulk_from_raw` path. The declared
/// dimension in the file is authoritative (it sizes a freshly created
/// collection). Payloads are not carried by this format — use `.jsonl` / `.csv`
/// when payloads are needed.
pub fn import_raw_bulk(db: &Database, path: &Path, config: &ImportConfig) -> Result<ImportStats> {
    let start = std::time::Instant::now();
    let bytes = std::fs::read(path).context("Failed to read binary file")?;
    let raw = velesdb_core::wire::vrb1::decode(&bytes)
        .map_err(|e| anyhow::anyhow!("Invalid VRB1 binary file: {e}"))?;

    let total = raw.ids.len();
    if config.dry_run {
        let conflicts = match (
            config.on_conflict,
            db.get_vector_collection(&config.collection),
        ) {
            (OnConflict::Upsert, _) | (_, None) => 0,
            (_, Some(col)) => existing_ids(&col, &raw.ids)?.len(),
        };
        let imported = if config.on_conflict == OnConflict::Fail && conflicts > 0 {
            0
        } else {
            total - conflicts.min(total)
        };
        return Ok(ImportStats {
            total,
            imported,
            errors: 0,
            skipped: 0,
            conflicts,
            error_details: Vec::new(),
            duration_ms: start.elapsed().as_millis() as u64,
        });
    }

    let collection = get_or_create_collection(
        db,
        &config.collection,
        raw.dimension,
        config.metric,
        config.storage_mode,
    )?;
    let conflicts = resolve_conflicts(&collection, config, &raw.ids)?;

    let n_conflicts = conflicts.len();
    let (vectors, ids, skipped) = if conflicts.is_empty() {
        (raw.vectors, raw.ids, 0)
    } else {
        filter_conflicting(&raw.vectors, &raw.ids, raw.dimension, &conflicts)
    };
    let imported = collection.upsert_bulk_from_raw(&vectors, &ids, raw.dimension, None)?;

    Ok(ImportStats {
        total,
        imported,
        errors: 0,
        skipped,
        conflicts: n_conflicts,
        error_details: Vec::new(),
        duration_ms: start.elapsed().as_millis() as u64,
    })
}

/// Get or create collection
fn get_or_create_collection(
    db: &Database,
    name: &str,
    dimension: usize,
    metric: Option<DistanceMetric>,
    storage_mode: StorageMode,
) -> Result<velesdb_core::VectorCollection> {
    if let Some(col) = db.get_vector_collection(name) {
        // Reusing an existing collection: an explicit --metric or
        // --dimension describes the collection the caller expects, so a
        // mismatch must be loud — silently importing into a
        // differently-metric'd collection mis-maps the corpus (#2095). An
        // absent --metric keeps whatever the collection already has: the
        // default cosine must not be read as intent and refuse the import.
        if let Some(metric) = metric {
            anyhow::ensure!(
                col.metric() == metric,
                "Collection '{name}' already exists with metric {:?};                  refusing to import with --metric {metric:?}",
                col.metric()
            );
        }
        anyhow::ensure!(
            col.dimension() == dimension,
            "Collection '{name}' already exists with dimension {};              refusing to import with --dimension {dimension}",
            col.dimension()
        );
        Ok(col)
    } else {
        db.create_vector_collection_with_options(
            name,
            dimension,
            metric.unwrap_or(DistanceMetric::Cosine),
            storage_mode,
        )?;
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
    /// Records skipped by `--on-conflict skip` (existing or duplicate ids).
    pub skipped: usize,
    /// Ids already in the collection, reported by `--dry-run`.
    pub conflicts: usize,
    /// First [`MAX_REPORTED_ERRORS`] rejected records (the `--error-report`
    /// file carries the full list).
    pub error_details: Vec<ImportError>,
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
