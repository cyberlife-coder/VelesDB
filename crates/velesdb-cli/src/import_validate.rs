//! Pre-write validation for `data import`.
//!
//! Everything here runs before a single point is written: the file is read
//! once to prove every record (or refuse fast), conflicts are resolved, and
//! `--dry-run` gets its numbers without touching the collection.

use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;
use velesdb_core::Database;

use super::{ImportConfig, ImportError, ImportStats, OnConflict, MAX_REPORTED_ERRORS};
use crate::helpers::BatchImporter;

/// JSON Lines record structure
#[derive(Debug, Deserialize)]
pub(super) struct JsonRecord {
    pub(super) id: u64,
    pub(super) vector: Vec<f32>,
    #[serde(default)]
    pub(super) payload: Option<serde_json::Value>,
}

/// What a full read of the input file proved before any write.
#[derive(Default)]
pub(super) struct Validation {
    /// Records read (1 per line for JSONL, 1 per data row for CSV).
    pub(super) records: usize,
    /// Ids of the well-formed records, in file order.
    pub(super) ids: Vec<u64>,
    /// Rejected records (all of them under `--dry-run` /
    /// `--continue-on-error`, otherwise the run stops at the first).
    /// Capped at [`MAX_REPORTED_ERRORS`]; `error_count` stays exact.
    errors: Vec<ImportError>,
    /// Every rejected record, capped or not.
    pub(super) error_count: usize,
    /// Vector length of the first well-formed record.
    pub(super) dimension: Option<usize>,
}

/// Validates a record's vector length against the expected dimension and
/// returns a reason string when it mismatches.
pub(super) fn check_dimension(
    len: usize,
    expected: Option<usize>,
    configured: Option<usize>,
) -> Option<String> {
    let expected = configured.or(expected);
    match expected {
        Some(d) if d != len => Some(format!("dimension {len} does not match {d}")),
        _ => None,
    }
}

/// Streaming validation pass over a JSONL file: no vectors kept, ids and
/// error ordinals only. `all_errors` collects every bad record; otherwise the
/// pass stops at the first one — enough to refuse the run without paying for
/// the rest of a file that will not be imported anyway.
pub(super) fn validate_jsonl(
    path: &Path,
    config: &ImportConfig,
    all_errors: bool,
) -> Result<Validation> {
    let file = File::open(path).context("Failed to open JSONL file")?;
    let mut reader = BufReader::with_capacity(128 * 1024, file);
    let mut out = Validation::default();
    let mut line = String::new();
    while reader.read_line(&mut line)? > 0 {
        out.records += 1;
        let ordinal = out.records;
        match serde_json::from_str::<JsonRecord>(&line) {
            Ok(rec) => match check_dimension(rec.vector.len(), out.dimension, config.dimension) {
                Some(reason) => {
                    out.error_count += 1;
                    if out.errors.len() < MAX_REPORTED_ERRORS {
                        out.errors.push(ImportError {
                            line: ordinal,
                            reason,
                        });
                    }
                    if !all_errors {
                        break;
                    }
                }
                None => {
                    out.dimension.get_or_insert(rec.vector.len());
                    out.ids.push(rec.id);
                }
            },
            Err(e) => {
                out.error_count += 1;
                if out.errors.len() < MAX_REPORTED_ERRORS {
                    out.errors.push(ImportError {
                        line: ordinal,
                        reason: format!("invalid JSON: {e}"),
                    });
                }
                if !all_errors {
                    break;
                }
            }
        }
        line.clear();
    }
    Ok(out)
}

/// Streaming validation pass over a CSV file; `line` counts data records
/// (the header is not a record).
pub(super) fn validate_csv(
    path: &Path,
    config: &ImportConfig,
    all_errors: bool,
) -> Result<Validation> {
    let (mut reader, id_idx, vector_idx) = open_csv(path, config)?;
    let mut out = Validation::default();
    for result in reader.records() {
        out.records += 1;
        let ordinal = out.records;
        match result {
            Ok(record) => {
                match csv_record_check(&record, id_idx, vector_idx, out.dimension, config.dimension)
                {
                    Ok((id, vector)) => {
                        out.dimension.get_or_insert(vector.len());
                        out.ids.push(id);
                    }
                    Err(reason) => {
                        out.errors.push(ImportError {
                            line: ordinal,
                            reason,
                        });
                        if !all_errors {
                            break;
                        }
                    }
                }
            }
            Err(e) => {
                out.error_count += 1;
                if out.errors.len() < MAX_REPORTED_ERRORS {
                    out.errors.push(ImportError {
                        line: ordinal,
                        reason: format!("invalid CSV record: {e}"),
                    });
                }
                if !all_errors {
                    break;
                }
            }
        }
    }
    Ok(out)
}

/// Opens a CSV reader and resolves the id/vector column indexes.
pub(super) fn open_csv(
    path: &Path,
    config: &ImportConfig,
) -> Result<(csv::Reader<BufReader<File>>, usize, usize)> {
    let file = File::open(path).context("Failed to open CSV file")?;
    let buffered = BufReader::with_capacity(128 * 1024, file);
    let mut reader = csv::Reader::from_reader(buffered);
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
    Ok((reader, id_idx, vector_idx))
}

/// Validates one CSV record and returns its `(id, vector)` when usable, else
/// the rejection reason. The returned vector is what the insert pass writes —
/// it is parsed exactly once per record.
pub(super) fn csv_record_check(
    record: &csv::StringRecord,
    id_idx: usize,
    vector_idx: usize,
    seen_dim: Option<usize>,
    configured_dim: Option<usize>,
) -> std::result::Result<(u64, Vec<f32>), String> {
    let id: u64 = record[id_idx]
        .parse()
        .map_err(|_| format!("invalid id '{}'", &record[id_idx]))?;
    let vector = parse_vector(&record[vector_idx]).map_err(|e| format!("invalid vector: {e}"))?;
    match check_dimension(vector.len(), seen_dim, configured_dim) {
        Some(reason) => Err(reason),
        None => Ok((id, vector)),
    }
}

/// Fetches the subset of `ids` already stored in `collection`, in 1000-id
/// chunks — the conflict set for `--on-conflict` and `--dry-run` reports.
pub(super) fn existing_ids(
    collection: &velesdb_core::VectorCollection,
    ids: &[u64],
) -> Result<HashSet<u64>> {
    let mut existing = HashSet::new();
    for chunk in ids.chunks(1000) {
        for (id, point) in chunk.iter().zip(collection.get(chunk)) {
            if point.is_some() {
                existing.insert(*id);
            }
        }
    }
    Ok(existing)
}

/// Opens the `--error-report` JSONL writer when configured.
pub(super) fn open_error_report(config: &ImportConfig) -> Result<Option<BufWriter<File>>> {
    config
        .error_report
        .as_ref()
        .map(|p| {
            File::create(p)
                .map(BufWriter::new)
                .with_context(|| format!("Failed to create error report {}", p.display()))
        })
        .transpose()
}

/// Records one rejected record: counted, kept in memory up to
/// [`MAX_REPORTED_ERRORS`], and streamed to the report file when open.
pub(super) fn record_error(
    details: &mut Vec<ImportError>,
    report: &mut Option<BufWriter<File>>,
    importer: &mut BatchImporter<'_>,
    error: ImportError,
) -> Result<()> {
    if details.len() < MAX_REPORTED_ERRORS {
        details.push(ImportError {
            line: error.line,
            reason: error.reason.clone(),
        });
    }
    if let Some(w) = report {
        writeln!(
            w,
            "{}",
            serde_json::json!({"line": error.line, "reason": error.reason})
        )
        .context("Failed to write error report")?;
    }
    importer.record_error();
    Ok(())
}

/// Resolves the effective dimension, refusing a collection dimension a
/// `--dimension` flag or the file disagrees with, and refusing to guess when
/// no valid record exists.
pub(super) fn resolve_dimension(validation: &Validation, config: &ImportConfig) -> Result<usize> {
    config
        .dimension
        .or(validation.dimension)
        .context("Cannot determine vector dimension (no valid records and no --dimension)")
}

/// Dry-run stats for a validated file against an (optional) existing
/// collection.
pub(super) fn dry_run_stats(
    db: &Database,
    config: &ImportConfig,
    validation: Validation,
    start: std::time::Instant,
) -> Result<ImportStats> {
    let conflicts = match (
        config.on_conflict,
        db.get_vector_collection(&config.collection),
    ) {
        (OnConflict::Upsert, _) | (_, None) => 0,
        (_, Some(col)) => existing_ids(&col, &validation.ids)?.len(),
    };
    let imported = match config.on_conflict {
        OnConflict::Fail if conflicts > 0 => 0,
        OnConflict::Skip => validation.ids.len() - conflicts,
        _ => validation.ids.len(),
    };
    let errors = validation.error_count;
    let error_details = validation
        .errors
        .into_iter()
        .take(MAX_REPORTED_ERRORS)
        .collect();
    Ok(ImportStats {
        total: validation.records,
        imported,
        errors,
        skipped: 0,
        conflicts,
        error_details,
        duration_ms: start.elapsed().as_millis() as u64,
    })
}

/// Whether `id` should be inserted; mutates `seen`/`skipped` for the
/// `--on-conflict skip` policy, which also skips an id already imported
/// from an earlier record of the same file (first record wins).
/// Returns `true` to insert.
pub(super) fn take_conflict(
    id: u64,
    mode: OnConflict,
    conflicts: &HashSet<u64>,
    seen: &mut HashSet<u64>,
    skipped: &mut usize,
) -> bool {
    if mode != OnConflict::Skip {
        return true;
    }
    if conflicts.contains(&id) || !seen.insert(id) {
        *skipped += 1;
        return false;
    }
    true
}

/// Builds the conflict id set for `--on-conflict skip`, and refuses the whole
/// import up front under `--on-conflict fail`.
pub(super) fn resolve_conflicts(
    collection: &velesdb_core::VectorCollection,
    config: &ImportConfig,
    ids: &[u64],
) -> Result<HashSet<u64>> {
    match config.on_conflict {
        OnConflict::Upsert => Ok(HashSet::new()),
        OnConflict::Fail => {
            let conflicts = existing_ids(collection, ids)?;
            anyhow::ensure!(
                conflicts.is_empty(),
                "{} imported id(s) already exist in '{}' (first: {}); \
                 rerun with --on-conflict skip or upsert",
                conflicts.len(),
                config.collection,
                conflicts.iter().min().copied().unwrap_or_default()
            );
            Ok(conflicts)
        }
        OnConflict::Skip => existing_ids(collection, ids),
    }
}

/// Aborts on the first validation error unless `--continue-on-error` asks to
/// keep going. Runs after validation and before the collection is created, so
/// a bad file never leaves a partial import. `--dry-run` is exempt: its whole
/// job is reporting every error it finds.
pub(super) fn fail_fast_on_first_error(
    validation: &Validation,
    config: &ImportConfig,
) -> Result<()> {
    if !config.continue_on_error && !config.dry_run {
        if let Some(e) = validation.errors.first() {
            anyhow::bail!("record {}: {}", e.line, e.reason);
        }
    }
    Ok(())
}

/// Drops every record whose id is in `conflicts` (the `--on-conflict skip`
/// path for flat VRB1 data). Returns filtered vectors/ids and the skip count.
pub(super) fn filter_conflicting(
    vectors: &[f32],
    ids: &[u64],
    dimension: usize,
    conflicts: &HashSet<u64>,
) -> (Vec<f32>, Vec<u64>, usize) {
    let mut kept_ids = Vec::with_capacity(ids.len());
    let mut kept_vectors = Vec::with_capacity(vectors.len());
    let mut skipped = 0usize;
    for (i, &id) in ids.iter().enumerate() {
        if conflicts.contains(&id) {
            skipped += 1;
        } else {
            kept_ids.push(id);
            kept_vectors.extend_from_slice(&vectors[i * dimension..(i + 1) * dimension]);
        }
    }
    (kept_vectors, kept_ids, skipped)
}

/// Builds a JSON payload from non-id, non-vector CSV columns.
pub(super) fn build_csv_payload(
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
pub(super) fn parse_vector(s: &str) -> Result<Vec<f32>> {
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
