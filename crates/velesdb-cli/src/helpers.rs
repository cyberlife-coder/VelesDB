//! Shared CLI helpers to eliminate duplication across modules.
//!
//! Extracted per Martin Fowler's "Extract Method" / "Parameterize Method"
//! refactoring patterns. Each helper consolidates a pattern that appeared
//! in two or more CLI modules.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::Result;
use indicatif::{ProgressBar, ProgressStyle};
use velesdb_core::{Database, Point, VectorCollection};

// ---------------------------------------------------------------------------
// Global `--config` wiring (issue #1549)
// ---------------------------------------------------------------------------
//
// Every CLI command that opens a database — the REPL and every one-shot
// subcommand — funnels through `open_database` below, so the top-level
// `--config`/`VELESDB_CONFIG` flag only needs to be captured once (in
// `main::cli_main`) rather than threaded through two dozen handler
// signatures. The explicit-parameter variant (`open_database_with_config`)
// is what's actually unit-tested; the global is a thin, side-effect-free
// lookup on top of it.

/// Process-wide `--config` path, set once at startup by `main::cli_main`.
/// `None` means the flag was not passed — every command opens the database
/// with core defaults, exactly like before this flag existed.
static CONFIG_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Records the `--config`/`VELESDB_CONFIG` path parsed from the CLI.
///
/// Must be called exactly once, before any command dispatch that might open
/// a database. Idempotent by construction (`OnceLock`): a second call is a
/// silent no-op, which only matters for tests that exercise `cli_main`
/// in-process — production `main()` calls it exactly once.
pub fn set_config_path(path: Option<PathBuf>) {
    let _ = CONFIG_PATH.set(path);
}

/// Opens a database at `path`, honouring the global `--config` path if
/// [`set_config_path`] recorded one.
///
/// # Errors
///
/// See [`open_database_with_config`].
pub fn open_database(path: &Path) -> Result<Database> {
    let config_path = CONFIG_PATH
        .get()
        .and_then(Option::as_ref)
        .map(PathBuf::as_path);
    open_database_with_config(path, config_path)
}

/// Opens a database at `path`, optionally loading the core
/// [`velesdb_core::config::VelesConfig`] (search/HNSW/storage/limits/
/// quantization/WAL batching) from `config_path` first.
///
/// - `config_path: None` — behaves exactly like [`Database::open`] (core
///   defaults), unchanged from before this flag existed.
/// - `config_path: Some(file)` — the TOML file **must** exist and pass
///   [`velesdb_core::config::VelesConfig::load_from_path_engine_only`]
///   validation. A missing file or an invalid engine value is a fail-fast,
///   actionable error — never a silent fallback to defaults. The
///   underlying typed [`velesdb_core::config::ConfigError`] is preserved
///   as the error source.
///
///   Only the engine sections (`[search]`/`[hnsw]`/`[storage]`/`[limits]`/
///   `[quantization]`/`[wal_batch]`) are read; any other top-level table —
///   notably `[server]`/`[auth]`/`[tls]`/`[cors]`, which `--config` files
///   shared with `velesdb-server` legitimately use for its own HTTP
///   transport — is silently ignored rather than being parsed into
///   `VelesConfig`'s own same-named (but unrelated) `server`/`logging`
///   fields and rejected by validation rules that were never meant for
///   that value (e.g. `velesdb-server`'s `[server] port = 443` would
///   otherwise trip `VelesConfig`'s `server.port >= 1024` rule). `VELESDB_*`
///   env vars still override values from the (filtered) file.
///
/// # Errors
///
/// Returns an error if `config_path` is `Some` and the file is missing or
/// fails to parse/validate, or if opening the database itself fails (e.g.
/// a stale lock file, a corrupt collection on disk).
pub fn open_database_with_config(path: &Path, config_path: Option<&Path>) -> Result<Database> {
    match config_path {
        None => Ok(Database::open(path)?),
        Some(cfg) => {
            if !cfg.exists() {
                anyhow::bail!("config file not found: {}", cfg.display());
            }
            let config = velesdb_core::config::VelesConfig::load_from_path_engine_only(cfg)
                .map_err(|e| {
                    anyhow::anyhow!("failed to load VelesDB config from {}: {e}", cfg.display())
                })?;
            Ok(Database::open_with_config(path, config)?)
        }
    }
}

// ---------------------------------------------------------------------------
// Import batch helpers
// ---------------------------------------------------------------------------

/// Manages batched upsert of points with progress tracking.
///
/// Encapsulates the batch-accumulate-flush loop shared by `import_jsonl`
/// and `import_csv`. Callers push individual points; the importer flushes
/// to the collection automatically when the batch reaches capacity.
pub struct BatchImporter<'a> {
    collection: &'a VectorCollection,
    batch: Vec<Point>,
    batch_size: usize,
    pub stats: ImportAccumulator,
}

/// Mutable counters accumulated during an import run.
#[derive(Debug, Default)]
pub struct ImportAccumulator {
    /// Successfully imported records.
    pub imported: usize,
    /// Records skipped due to parse/dimension errors.
    pub errors: usize,
}

impl<'a> BatchImporter<'a> {
    /// Creates a new batch importer targeting `collection`.
    pub fn new(collection: &'a VectorCollection, batch_size: usize) -> Self {
        Self {
            collection,
            batch: Vec::with_capacity(batch_size),
            batch_size,
            stats: ImportAccumulator::default(),
        }
    }

    /// Pushes a valid point into the current batch.
    ///
    /// When the batch reaches capacity it is flushed via `upsert_bulk`.
    ///
    /// # Errors
    ///
    /// Propagates any error from `upsert_bulk`.
    pub fn push(&mut self, point: Point) -> Result<()> {
        self.batch.push(point);
        self.stats.imported += 1;

        if self.batch.len() >= self.batch_size {
            self.collection.upsert_bulk(&self.batch)?;
            self.batch.clear();
        }
        Ok(())
    }

    /// Records a skipped/errored record.
    pub fn record_error(&mut self) {
        self.stats.errors += 1;
    }

    /// Flushes any remaining points in the batch.
    ///
    /// # Errors
    ///
    /// Propagates any error from `upsert_bulk`.
    pub fn flush(self) -> Result<ImportAccumulator> {
        if !self.batch.is_empty() {
            self.collection.upsert_bulk(&self.batch)?;
        }
        Ok(self.stats)
    }
}

/// Creates a progress bar, hidden when `show` is false.
#[must_use]
pub fn create_progress_bar(total: usize, show: bool) -> ProgressBar {
    if show {
        let pb = ProgressBar::new(total as u64);
        if let Ok(style) = ProgressStyle::default_bar().template(
            "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})",
        ) {
            pb.set_style(style.progress_chars("#>-"));
        }
        pb
    } else {
        ProgressBar::hidden()
    }
}

/// Sets the import progress message with record count and file size.
pub fn set_import_message(progress: &ProgressBar, total: usize, file_size: u64, show: bool) {
    if show {
        #[allow(clippy::cast_precision_loss)]
        let size_mb = file_size as f64 / (1024.0 * 1024.0);
        progress.set_message(format!("Importing {total} vectors ({size_mb:.1} MB)"));
    }
}

// ---------------------------------------------------------------------------
// Row conversion helpers (REPL commands)
// ---------------------------------------------------------------------------

/// Converts a `Point`'s payload into a row map for table display.
///
/// Inserts the point ID under `"id"` and flattens any JSON object payload
/// into top-level keys.
pub fn point_payload_to_row(
    id: u64,
    payload: &Option<serde_json::Value>,
) -> HashMap<String, serde_json::Value> {
    let mut row = HashMap::new();
    row.insert("id".to_string(), serde_json::json!(id));
    if let Some(serde_json::Value::Object(map)) = payload {
        for (k, v) in map {
            row.insert(k.clone(), v.clone());
        }
    }
    row
}

/// Converts a `Point`'s payload into a row map, truncating string values
/// longer than 50 characters for browsing display.
pub fn point_payload_to_browse_row(
    id: u64,
    payload: &Option<serde_json::Value>,
) -> HashMap<String, serde_json::Value> {
    let mut row = HashMap::new();
    row.insert("id".to_string(), serde_json::json!(id));
    if let Some(serde_json::Value::Object(map)) = payload {
        for (k, v) in map {
            row.insert(k.clone(), truncate_display_value(v));
        }
    }
    row
}

/// Truncates a JSON string value to 47 chars + "..." if it exceeds 50 characters.
///
/// Non-string values are returned unchanged.
fn truncate_display_value(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::String(s) if s.len() > 50 => {
            let truncated: String = s.chars().take(47).collect();
            serde_json::json!(format!("{truncated}..."))
        }
        other => other.clone(),
    }
}

/// Serializes a value as pretty JSON and prints it to stdout.
///
/// # Errors
///
/// Returns an error if JSON serialization fails.
pub fn print_json(data: &serde_json::Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(data)?);
    Ok(())
}

// ---------------------------------------------------------------------------
// Export helpers (REPL commands)
// ---------------------------------------------------------------------------

/// Builds an export record from a point, optionally including the vector.
pub fn point_to_export_record(
    id: u64,
    vector: Option<&[f32]>,
    payload: &Option<serde_json::Value>,
) -> serde_json::Value {
    let mut record = serde_json::Map::new();
    record.insert("id".to_string(), serde_json::json!(id));
    if let Some(v) = vector {
        record.insert("vector".to_string(), serde_json::json!(v));
    }
    if let Some(p) = payload {
        record.insert("payload".to_string(), p.clone());
    }
    serde_json::Value::Object(record)
}

/// Serializes records to JSON and writes them to a file.
///
/// # Errors
///
/// Returns a `CommandResult::Error` string if serialization or file I/O fails.
pub fn write_export_file(records: &[serde_json::Value], filename: &str) -> Result<(), String> {
    let json_str = serde_json::to_string_pretty(records)
        .map_err(|e| format!("Failed to serialize records: {e}"))?;
    std::fs::write(filename, json_str).map_err(|e| format!("Failed to write file: {e}"))?;
    Ok(())
}

#[cfg(test)]
#[path = "helpers_tests.rs"]
mod helpers_tests;
