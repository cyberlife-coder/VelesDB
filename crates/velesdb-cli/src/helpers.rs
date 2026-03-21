//! Shared CLI helpers to eliminate duplication across modules.
//!
//! Extracted per Martin Fowler's "Extract Method" / "Parameterize Method"
//! refactoring patterns. Each helper consolidates a pattern that appeared
//! in two or more CLI modules.

use std::collections::HashMap;

use anyhow::Result;
use indicatif::{ProgressBar, ProgressStyle};
use velesdb_core::{Point, VectorCollection};

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
        pb.set_style(
            ProgressStyle::default_bar()
                .template(
                    "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})",
                )
                .expect("hardcoded progress bar template is valid")
                .progress_chars("#>-"),
        );
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
