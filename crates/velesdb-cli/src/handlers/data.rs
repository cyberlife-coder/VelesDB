//! Handlers for data commands: `export`, `import`, `get`, `upsert`, `delete-points`, `stream-insert`.

use std::path::{Path, PathBuf};

use anyhow::Result;
use colored::Colorize;

use crate::cli_types::MetricArg;
use crate::cli_types::StorageModeArg;
use crate::import;

/// Handles the `export` subcommand: exports a vector collection to JSON.
pub fn handle_export(
    path: &Path,
    collection: &str,
    output: Option<PathBuf>,
    include_vectors: bool,
) -> Result<()> {
    let db = velesdb_core::Database::open(path)?;
    let col = db.get_vector_collection(collection).ok_or_else(|| {
        anyhow::anyhow!(
            "Vector collection '{}' not found. Export requires a vector collection.",
            collection
        )
    })?;

    let cfg = col.config();
    let output_path = output.unwrap_or_else(|| PathBuf::from(format!("{collection}.json")));

    println!(
        "Exporting {} records from {}...",
        cfg.point_count,
        collection.green()
    );

    let records = collect_export_records(&col, cfg.point_count, include_vectors);
    std::fs::write(&output_path, serde_json::to_string_pretty(&records)?)?;
    println!(
        "{} Exported {} records to {}",
        "\u{2713}".green(),
        records.len(),
        output_path.display().to_string().green()
    );
    Ok(())
}

/// Collects all records from a vector collection for export.
fn collect_export_records(
    col: &velesdb_core::VectorCollection,
    point_count: usize,
    include_vectors: bool,
) -> Vec<serde_json::Value> {
    let mut records = Vec::new();
    let batch_size = 1000;

    for batch_start in (0..point_count).step_by(batch_size) {
        let ids: Vec<u64> =
            ((batch_start as u64 + 1)..=((batch_start + batch_size) as u64)).collect();
        let points = col.get(&ids);

        for point in points.into_iter().flatten() {
            let mut record = serde_json::Map::new();
            record.insert("id".to_string(), serde_json::json!(point.id));
            if include_vectors {
                record.insert("vector".to_string(), serde_json::json!(point.vector));
            }
            if let Some(payload) = &point.payload {
                record.insert("payload".to_string(), payload.clone());
            }
            records.push(serde_json::Value::Object(record));
        }
    }
    records
}

/// Handles the `import` subcommand: imports data from CSV or JSONL.
#[allow(clippy::too_many_arguments)] // Reason: mirrors clap subcommand field count directly
pub fn handle_import(
    file: &Path,
    database: &Path,
    collection: String,
    dimension: Option<usize>,
    metric: MetricArg,
    storage_mode: StorageModeArg,
    id_column: String,
    vector_column: String,
    batch_size: usize,
    progress: bool,
) -> Result<()> {
    let db = velesdb_core::Database::open(database)?;
    let config = import::ImportConfig {
        collection,
        dimension,
        metric: metric.into(),
        storage_mode: storage_mode.into(),
        batch_size,
        id_column,
        vector_column,
        show_progress: progress,
    };

    let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("");

    let stats = match ext.to_lowercase().as_str() {
        "jsonl" | "ndjson" => import::import_jsonl(&db, file, &config)?,
        "csv" => import::import_csv(&db, file, &config)?,
        _ => {
            anyhow::bail!("Unsupported file format: {}. Use .csv or .jsonl", ext);
        }
    };

    print_import_summary(&stats);
    Ok(())
}

/// Prints a summary after a successful import.
fn print_import_summary(stats: &import::ImportStats) {
    println!("\n{}", "Import Summary".green().bold());
    println!("  Total records:    {}", stats.total);
    println!("  Imported:         {}", stats.imported.to_string().green());
    if stats.errors > 0 {
        println!("  Errors:           {}", stats.errors.to_string().red());
    }
    println!("  Duration:         {} ms", stats.duration_ms);
    println!(
        "  Throughput:       {:.0} records/sec",
        stats.records_per_sec()
    );
}

/// Handles the `get` subcommand: retrieves a single point by ID.
pub fn handle_get(path: &Path, collection: &str, id: u64, format: &str) -> Result<()> {
    let db = velesdb_core::Database::open(path)?;
    let col = db
        .get_vector_collection(collection)
        .ok_or_else(|| anyhow::anyhow!("Collection '{}' not found", collection))?;

    let points = col.get(&[id]);

    if format == "json" {
        print_point_json(points);
    } else {
        print_point_table(points, id);
    }
    Ok(())
}

/// Prints a point as JSON.
fn print_point_json(points: Vec<Option<velesdb_core::Point>>) {
    if let Some(point) = points.into_iter().flatten().next() {
        let output = serde_json::json!({
            "id": point.id,
            "vector": point.vector,
            "payload": point.payload
        });
        // Reason: doc output to stdout; formatting failure is not recoverable
        if let Ok(json) = serde_json::to_string_pretty(&output) {
            println!("{json}");
        }
    } else {
        println!("null");
    }
}

/// Prints a point as a colored table.
fn print_point_table(points: Vec<Option<velesdb_core::Point>>, id: u64) {
    if let Some(point) = points.into_iter().flatten().next() {
        println!("\n{}", "Point Found".bold().underline());
        println!("  ID: {}", point.id.to_string().green());
        println!("  Vector: [{} dimensions]", point.vector.len());
        if let Some(payload) = &point.payload {
            println!("  Payload: {payload}");
        }
    } else {
        println!("{} Point with ID {} not found", "\u{274c}".red(), id);
    }
}

/// Handles the `upsert` subcommand: inserts or updates a single point.
pub fn handle_upsert(
    path: &Path,
    collection: &str,
    id: u64,
    vector: Option<String>,
    payload: Option<String>,
) -> Result<()> {
    let db = velesdb_core::Database::open(path)?;
    let col = db
        .get_vector_collection(collection)
        .ok_or_else(|| anyhow::anyhow!("Vector collection '{}' not found", collection))?;

    let vec_data = parse_vector_json(vector)?;
    let payload_data = parse_payload_json(payload)?;

    let point = velesdb_core::Point::new(id, vec_data, payload_data);
    col.upsert(vec![point])
        .map_err(|e| anyhow::anyhow!("Upsert failed: {e}"))?;

    println!(
        "{} Upserted point {} into '{}'",
        "\u{2705}".green(),
        id.to_string().green(),
        collection.cyan()
    );
    Ok(())
}

/// Parses an optional vector JSON string into a `Vec<f32>`.
fn parse_vector_json(raw: Option<String>) -> Result<Vec<f32>> {
    match raw {
        Some(v) => {
            serde_json::from_str(&v).map_err(|e| anyhow::anyhow!("Invalid vector JSON: {e}"))
        }
        None => Ok(vec![]),
    }
}

/// Parses an optional payload JSON string into a `serde_json::Value`.
fn parse_payload_json(raw: Option<String>) -> Result<Option<serde_json::Value>> {
    match raw {
        Some(p) => {
            let v = serde_json::from_str(&p)
                .map_err(|e| anyhow::anyhow!("Invalid payload JSON: {e}"))?;
            Ok(Some(v))
        }
        None => Ok(None),
    }
}

/// Handles the `delete-points` subcommand: removes points by ID.
pub fn handle_delete_points(path: &Path, collection: &str, ids: &[u64]) -> Result<()> {
    let db = velesdb_core::Database::open(path)?;
    let col = db
        .get_vector_collection(collection)
        .ok_or_else(|| anyhow::anyhow!("Vector collection '{}' not found", collection))?;

    col.delete(ids)
        .map_err(|e| anyhow::anyhow!("Delete failed: {e}"))?;

    println!(
        "{} Deleted {} point(s) from '{}'",
        "\u{2705}".green(),
        ids.len(),
        collection.cyan()
    );
    Ok(())
}

/// Handles the `scroll` subcommand: cursor-based pagination through a collection.
pub fn handle_scroll(
    path: &Path,
    collection: &str,
    batch_size: usize,
    cursor: Option<u64>,
    format: &str,
) -> Result<()> {
    let db = velesdb_core::Database::open(path)?;
    let col = db
        .get_vector_collection(collection)
        .ok_or_else(|| anyhow::anyhow!("Collection '{}' not found", collection))?;

    let batch = col
        .scroll_batch(cursor, batch_size, None)
        .map_err(|e| anyhow::anyhow!("Scroll failed: {e}"))?;

    if format == "json" {
        print_scroll_json(&batch)
    } else {
        print_scroll_table(&batch, collection);
        Ok(())
    }
}

/// Prints scroll results as JSON (suitable for piping).
fn print_scroll_json(batch: &velesdb_core::ScrollBatch) -> Result<()> {
    let output = serde_json::json!({
        "points": batch.points.iter().map(|p| {
            serde_json::json!({
                "id": p.id,
                "vector": p.vector,
                "payload": p.payload
            })
        }).collect::<Vec<_>>(),
        "nextCursor": batch.next_cursor
    });
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

/// Prints scroll results as a colored table.
fn print_scroll_table(batch: &velesdb_core::ScrollBatch, collection: &str) {
    println!(
        "\n{} ({} points)",
        format!("Scroll: {collection}").bold().underline(),
        batch.points.len()
    );
    for p in &batch.points {
        println!(
            "  ID: {} [{} dims]",
            p.id.to_string().green(),
            p.vector.len()
        );
        if let Some(payload) = &p.payload {
            println!("    Payload: {payload}");
        }
    }
    if let Some(next) = batch.next_cursor {
        println!(
            "\n  Next cursor: {} (pass --cursor {} to continue)",
            next.to_string().cyan(),
            next
        );
    } else {
        println!("\n  {} End of collection", "\u{2713}".green());
    }
}

/// Handles the `stream-insert` subcommand: reads JSONL from stdin and upserts in micro-batches.
///
/// Each line must be a JSON object with `id` (number), `vector` (array of numbers),
/// and optional `payload` (object). Example:
/// ```json
/// {"id": 1, "vector": [0.1, 0.2, 0.3], "payload": {"title": "Doc 1"}}
/// ```
pub fn handle_stream_insert(path: &Path, collection: &str, batch_size: usize) -> Result<()> {
    use std::io::BufRead;

    let db = velesdb_core::Database::open(path)?;
    let col = db
        .get_vector_collection(collection)
        .ok_or_else(|| anyhow::anyhow!("Vector collection '{}' not found", collection))?;

    let stdin = std::io::stdin();
    let reader = stdin.lock();

    let mut batch: Vec<velesdb_core::Point> = Vec::with_capacity(batch_size);
    let mut total_inserted: usize = 0;
    let mut total_errors: usize = 0;

    for line_result in reader.lines() {
        let line = line_result?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        match parse_point_json(trimmed) {
            Ok(point) => {
                batch.push(point);
                if batch.len() >= batch_size {
                    let count = batch.len();
                    flush_batch(&col, &mut batch)?;
                    total_inserted += count;
                    eprint!("\r  Inserted: {total_inserted}");
                }
            }
            Err(e) => {
                total_errors += 1;
                eprintln!("\r  Skipping invalid line: {e}");
            }
        }
    }

    // Flush remaining batch
    if !batch.is_empty() {
        let count = batch.len();
        flush_batch(&col, &mut batch)?;
        total_inserted += count;
    }

    eprintln!();
    println!(
        "{} Stream insert complete: {} inserted, {} errors",
        "\u{2705}".green(),
        total_inserted.to_string().green(),
        format_error_count(total_errors),
    );
    Ok(())
}

/// Upserts a batch of points and drains the buffer via `std::mem::take`.
fn flush_batch(
    col: &velesdb_core::VectorCollection,
    batch: &mut Vec<velesdb_core::Point>,
) -> Result<()> {
    col.upsert(std::mem::take(batch))
        .map_err(|e| anyhow::anyhow!("Upsert failed: {e}"))
}

/// Formats the error count: red if non-zero, plain "0" otherwise.
fn format_error_count(count: usize) -> String {
    if count > 0 {
        count.to_string().red().to_string()
    } else {
        "0".to_string()
    }
}

/// Parses a single JSONL line into a [`velesdb_core::Point`].
///
/// Expected format: `{"id": <u64>, "vector": [<f32>, ...], "payload": {...}}`
fn parse_point_json(json_str: &str) -> Result<velesdb_core::Point> {
    let v: serde_json::Value =
        serde_json::from_str(json_str).map_err(|e| anyhow::anyhow!("JSON parse error: {e}"))?;

    let id = v
        .get("id")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| anyhow::anyhow!("Missing or invalid 'id' field"))?;

    let vector = parse_vector_array(&v)?;
    let payload = v.get("payload").cloned();

    Ok(velesdb_core::Point::new(id, vector, payload))
}

/// Extracts the `"vector"` field from a JSON value as `Vec<f32>`.
fn parse_vector_array(v: &serde_json::Value) -> Result<Vec<f32>> {
    let arr = v
        .get("vector")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("Missing or invalid 'vector' field"))?;

    arr.iter()
        .map(|n| {
            n.as_f64()
                .map(|f| f as f32)
                .ok_or_else(|| anyhow::anyhow!("Non-numeric value in vector"))
        })
        .collect()
}
