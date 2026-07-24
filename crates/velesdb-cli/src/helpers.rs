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
mod tests {
    use super::*;

    // -----------------------------------------------------------------
    // `open_database_with_config` (issue #1549 — CLI `--config` wiring)
    // -----------------------------------------------------------------
    //
    // These exercise `open_database_with_config` directly (the
    // process-global `open_database`/`set_config_path` pair is a thin
    // wrapper covered indirectly through the binary's `--config` flag).

    #[test]
    fn test_no_config_path_opens_with_core_defaults() {
        let dir = tempfile::tempdir().expect("test: temp dir");
        let db = open_database_with_config(dir.path(), None).expect("test: open without config");
        assert_eq!(
            db.config().limits.max_collections,
            velesdb_core::config::LimitsConfig::default().max_collections
        );
    }

    #[test]
    fn test_custom_toml_limit_is_actually_enforced_not_just_parsed() {
        let dir = tempfile::tempdir().expect("test: temp dir");
        let toml_dir = tempfile::tempdir().expect("test: config dir");
        let config_path = toml_dir.path().join("velesdb.toml");
        std::fs::write(&config_path, "[limits]\nmax_collections = 1\n")
            .expect("test: write config");

        let db = open_database_with_config(dir.path(), Some(&config_path))
            .expect("test: open with custom config");

        // Sanity: the value really was parsed onto the running config.
        assert_eq!(db.config().limits.max_collections, 1);

        // Proof it's *enforced*, not just parsed: first collection succeeds,
        // second is refused by the engine because of the configured cap.
        db.create_vector_collection_with_options(
            "first",
            4,
            velesdb_core::DistanceMetric::Cosine,
            velesdb_core::StorageMode::Full,
        )
        .expect("test: first collection under the limit should succeed");

        let err = db
            .create_vector_collection_with_options(
                "second",
                4,
                velesdb_core::DistanceMetric::Cosine,
                velesdb_core::StorageMode::Full,
            )
            .expect_err("test: second collection should be refused by the configured limit");
        assert!(
            err.to_string().contains("max_collections"),
            "unexpected error: {err}"
        );
    }

    /// Regression test (Fable review finding): a `velesdb.toml` shared with
    /// `velesdb-server` may legitimately have `[server] port = 443` (that
    /// binary's own HTTP bind port). Before the fix this also landed in
    /// `VelesConfig`'s own unrelated `server.port` field and was rejected
    /// by its `>= 1024` rule, so opening the *same* file from the CLI
    /// failed even though the CLI never reads `[server]` at all.
    #[test]
    fn test_shell_owned_server_section_does_not_block_cli_open() {
        let dir = tempfile::tempdir().expect("test: temp dir");
        let toml_dir = tempfile::tempdir().expect("test: config dir");
        let config_path = toml_dir.path().join("velesdb.toml");
        std::fs::write(
            &config_path,
            "[server]\nport = 443\n\n[limits]\nmax_collections = 5\n",
        )
        .expect("test: write config");

        let db = open_database_with_config(dir.path(), Some(&config_path))
            .expect("a shell-owned [server] port=443 must not block CLI database open");
        assert_eq!(db.config().limits.max_collections, 5);
    }

    #[test]
    fn test_explicit_missing_config_path_fails_fast_no_silent_default() {
        let dir = tempfile::tempdir().expect("test: temp dir");
        let missing = std::path::Path::new("/nonexistent/velesdb-issue-1549.toml");

        let err = match open_database_with_config(dir.path(), Some(missing)) {
            Err(e) => e,
            Ok(_) => panic!("test: missing explicit config path must error, not fall back"),
        };
        assert!(
            err.to_string().contains("config file not found"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_invalid_config_value_surfaces_typed_config_error_fail_fast() {
        let dir = tempfile::tempdir().expect("test: temp dir");
        let toml_dir = tempfile::tempdir().expect("test: config dir");
        let config_path = toml_dir.path().join("velesdb.toml");
        // max_collections = 0 is out of range (validate_limits requires >= 1).
        std::fs::write(&config_path, "[limits]\nmax_collections = 0\n")
            .expect("test: write config");

        let err = match open_database_with_config(dir.path(), Some(&config_path)) {
            Err(e) => e,
            Ok(_) => panic!("test: invalid value must fail fast, not silently default"),
        };
        assert!(
            err.to_string().contains("limits.max_collections"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_point_payload_to_row_with_payload() {
        let payload = Some(serde_json::json!({
            "title": "Hello",
            "score": 0.95
        }));

        let row = point_payload_to_row(42, &payload);

        assert_eq!(row.get("id"), Some(&serde_json::json!(42)));
        assert_eq!(row.get("title"), Some(&serde_json::json!("Hello")));
        assert_eq!(row.get("score"), Some(&serde_json::json!(0.95)));
        assert_eq!(row.len(), 3);
    }

    #[test]
    fn test_point_payload_to_row_without_payload() {
        let row = point_payload_to_row(7, &None);

        assert_eq!(row.get("id"), Some(&serde_json::json!(7)));
        assert_eq!(row.len(), 1);
    }

    #[test]
    fn test_point_payload_to_browse_row_truncates() {
        let long_string = "a".repeat(80);
        let payload = Some(serde_json::json!({
            "content": long_string,
            "short": "ok"
        }));

        let row = point_payload_to_browse_row(1, &payload);

        assert_eq!(row.get("id"), Some(&serde_json::json!(1)));
        // "short" stays unchanged
        assert_eq!(row.get("short"), Some(&serde_json::json!("ok")));
        // "content" is truncated to 47 chars + "..."
        let content = row.get("content").unwrap().as_str().unwrap();
        assert_eq!(content.len(), 50);
        assert!(content.ends_with("..."));
    }

    #[test]
    fn test_truncate_display_value_short_string() {
        let val = serde_json::json!("short text");
        let result = truncate_display_value(&val);
        assert_eq!(result, serde_json::json!("short text"));
    }

    #[test]
    fn test_truncate_display_value_long_string() {
        let long = "x".repeat(100);
        let result = truncate_display_value(&serde_json::json!(long));
        let s = result.as_str().unwrap();
        assert_eq!(s.len(), 50);
        assert!(s.ends_with("..."));
        assert!(s.starts_with("xxxxxxx"));
    }
}
