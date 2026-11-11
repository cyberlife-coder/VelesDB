//! Streaming export for `data export`.
//!
//! Pages the collection through `scroll_batch` (bounded memory at any
//! collection size) and writes JSON, JSONL or CSV. `--where` accepts a VelesQL
//! filter condition lowered through [`Filter::try_from_where`], which fails
//! closed on anything a row-level predicate cannot express.

use std::collections::BTreeSet;
use std::io::{BufWriter, Write};
use std::path::Path;

use anyhow::{Context, Result};
use velesdb_core::velesql::Parser;
use velesdb_core::{Filter, Point, VectorCollection};

use crate::cli_types::ExportFormatArg;
use crate::helpers;

const PAGE_SIZE: usize = 1000;

/// Parses a `--where` clause into a [`Filter`].
///
/// The clause is wrapped in `SELECT * FROM export WHERE {clause}` and parsed
/// as a full query. The parsed query must equal the bare
/// `SELECT * FROM export` apart from its `where_clause` — any `LIMIT`,
/// `ORDER BY`, `WITH`, `MATCH` clause or second statement smuggled past the
/// keyword would otherwise be silently ignored.
///
/// # Errors
///
/// Rejects an unparseable clause, extra query structure, or a condition a
/// row-level predicate cannot express (vector search, `similarity()`,
/// `MATCH`, `$parameter`, subqueries).
pub fn parse_where_filter(clause: &str) -> Result<Filter> {
    let mut parsed = Parser::parse(&format!("SELECT * FROM export WHERE {clause}"))
        .map_err(|e| anyhow::anyhow!("Invalid --where clause: {e}"))?;
    let condition = parsed
        .select
        .where_clause
        .take()
        .context("--where requires a filter condition")?;
    let base = Parser::parse("SELECT * FROM export")
        .map_err(|e| anyhow::anyhow!("Invalid --where clause: {e}"))?;
    anyhow::ensure!(
        parsed == base,
        "--where accepts a filter condition only (no LIMIT, ORDER BY, WITH, MATCH or extra statements)"
    );
    Filter::try_from_where(condition).map_err(|e| anyhow::anyhow!("Invalid --where clause: {e}"))
}

/// Iterates a collection's points page by page, applying `filter`.
fn for_each_point<F>(col: &VectorCollection, filter: Option<&Filter>, mut visit: F) -> Result<()>
where
    F: FnMut(&Point) -> Result<()>,
{
    let mut cursor = None;
    loop {
        let batch = col
            .scroll_batch(cursor, PAGE_SIZE, filter)
            .map_err(|e| anyhow::anyhow!("Export scan failed: {e}"))?;
        for point in &batch.points {
            visit(point)?;
        }
        match batch.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    Ok(())
}

/// Serializes one point to the shared export record shape
/// (`id`, `vector` when included, `payload` when present).
fn record_of(point: &Point, include_vectors: bool) -> serde_json::Value {
    let vector = include_vectors.then_some(point.vector.as_slice());
    helpers::point_to_export_record(point.id, vector, &point.payload)
}

/// Streams records as a pretty JSON array, byte-identical to
/// `serde_json::to_string_pretty` of the full `Vec<Value>`.
fn write_json<W: Write>(
    mut out: W,
    col: &VectorCollection,
    filter: Option<&Filter>,
    include_vectors: bool,
) -> Result<usize> {
    let mut count = 0usize;
    for_each_point(col, filter, |point| {
        if count == 0 {
            writeln!(out, "[")?;
        } else {
            writeln!(out, ",")?;
        }
        let pretty = serde_json::to_string_pretty(&record_of(point, include_vectors))?;
        for (i, line) in pretty.lines().enumerate() {
            if i > 0 {
                writeln!(out)?;
            }
            write!(out, "  {line}")?;
        }
        count += 1;
        Ok(())
    })?;
    if count == 0 {
        writeln!(out, "[]")?;
    } else {
        writeln!(out, "\n]")?;
    }
    Ok(count)
}

/// Streams records as JSON Lines (one compact object per line).
fn write_jsonl<W: Write>(
    mut out: W,
    col: &VectorCollection,
    filter: Option<&Filter>,
    include_vectors: bool,
) -> Result<usize> {
    let mut count = 0usize;
    for_each_point(col, filter, |point| {
        serde_json::to_writer(&mut out, &record_of(point, include_vectors))?;
        writeln!(out)?;
        count += 1;
        Ok(())
    })
    .map(|()| count)
}

/// Collects the union of top-level payload object keys as CSV columns.
fn payload_columns(col: &VectorCollection, filter: Option<&Filter>) -> Result<Vec<String>> {
    let mut keys = BTreeSet::new();
    for_each_point(col, filter, |point| {
        if let Some(map) = point.payload.as_ref().and_then(|p| p.as_object()) {
            keys.extend(map.keys().cloned());
        }
        Ok(())
    })?;
    Ok(keys.into_iter().collect())
}

/// Renders one payload value as a CSV cell: scalars inline, nested values
/// as compact JSON, missing keys empty.
fn csv_cell(payload: &Point, key: &str) -> String {
    let Some(map) = payload.payload.as_ref().and_then(|p| p.as_object()) else {
        return String::new();
    };
    match map.get(key) {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(v) => serde_json::to_string(v).unwrap_or_default(),
        None => String::new(),
    }
}

/// Streams records as CSV: `id`, `vector` (JSON array) when included, then one
/// column per payload key. `data import` reads this layout back — the
/// `vector` JSON array and remaining columns re-enter the payload.
fn write_csv<W: Write>(
    out: W,
    col: &VectorCollection,
    filter: Option<&Filter>,
    include_vectors: bool,
) -> Result<usize> {
    let columns = payload_columns(col, filter)?;
    let mut writer = csv::Writer::from_writer(out);

    let mut header: Vec<String> = vec!["id".to_string()];
    if include_vectors {
        header.push("vector".to_string());
    }
    header.extend(columns.iter().cloned());
    writer.write_record(&header)?;

    let mut count = 0usize;
    for_each_point(col, filter, |point| {
        let mut row: Vec<String> = vec![point.id.to_string()];
        if include_vectors {
            row.push(serde_json::to_string(&point.vector)?);
        }
        row.extend(columns.iter().map(|key| csv_cell(point, key)));
        writer.write_record(&row)?;
        count += 1;
        Ok(())
    })?;
    writer.flush()?;
    Ok(count)
}

/// Exports `col` to `path` in `format`; returns the number of records written.
///
/// # Errors
///
/// Propagates scan and write failures. The file is created before the scan
/// starts; a mid-export error leaves a partial file.
pub fn export_collection(
    col: &VectorCollection,
    path: &Path,
    format: ExportFormatArg,
    include_vectors: bool,
    filter: Option<&Filter>,
) -> Result<usize> {
    let file = std::fs::File::create(path)
        .with_context(|| format!("Failed to create {}", path.display()))?;
    let out = BufWriter::with_capacity(128 * 1024, file);
    match format {
        ExportFormatArg::Json => write_json(out, col, filter, include_vectors),
        ExportFormatArg::Jsonl => write_jsonl(out, col, filter, include_vectors),
        ExportFormatArg::Csv => write_csv(out, col, filter, include_vectors),
    }
}

#[cfg(test)]
#[path = "export_tests.rs"]
mod export_tests;
