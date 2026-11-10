//! `velesdb db stats|vacuum` — offline maintenance of a database directory.
//!
//! Both open the database like every other command (`helpers::open_database`),
//! which takes the directory's exclusive `velesdb.lock`: a running server or
//! REPL on the same directory makes them fail instead of racing it.

use std::path::Path;

use anyhow::{bail, Result};
use velesdb_core::{Database, VectorCollection};

use crate::collection_helpers::{self, TypedCollection};

/// One collection's line in `db stats`.
#[derive(serde::Serialize)]
struct CollectionStatsRow {
    name: String,
    kind: &'static str,
    /// Points for vector/metadata collections, nodes for graph collections.
    points: usize,
    /// Graph collections only.
    edges: Option<usize>,
    disk_bytes: u64,
    /// From the last persisted `ANALYZE`; `None` when it never ran.
    last_analyzed_epoch_ms: Option<u64>,
    deleted_count: Option<u64>,
}

/// Handles `db stats`: per-collection size and the last persisted `ANALYZE`.
///
/// The CLI opens the database without a `DatabaseObserver`, so
/// `Database::get_collection_stats` has no read gate to consult here — the
/// same reason `collection analyze` reads it directly.
pub fn handle_db_stats(path: &Path, format: &str) -> Result<()> {
    let db = crate::helpers::open_database(path)?;
    let mut rows = Vec::new();
    for name in db.list_collections() {
        rows.push(stats_row(&db, &name)?);
    }
    let total_bytes = dir_size(db.data_dir())?;
    if format == "json" {
        let report = serde_json::json!({
            "path": path.display().to_string(),
            "total_disk_bytes": total_bytes,
            "collections": rows,
        });
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_stats_table(path, &rows, total_bytes);
    }
    Ok(())
}

fn stats_row(db: &Database, name: &str) -> Result<CollectionStatsRow> {
    let (points, edges) = match collection_helpers::resolve_collection(db, name) {
        Some(TypedCollection::Vector(c)) => (c.len(), None),
        Some(TypedCollection::Graph(c)) => (c.len(), Some(c.edge_count())),
        Some(TypedCollection::Metadata(c)) => (c.len(), None),
        None => (0, None),
    };
    let analyzed = db.get_collection_stats(name)?;
    Ok(CollectionStatsRow {
        name: name.to_owned(),
        kind: collection_helpers::collection_type_label(db, name),
        points,
        edges,
        // `name` comes from `list_collections`, i.e. a directory the
        // database itself created under a validated name.
        disk_bytes: dir_size(&db.data_dir().join(name))?,
        last_analyzed_epoch_ms: analyzed.as_ref().and_then(|s| s.last_analyzed_epoch_ms),
        deleted_count: analyzed.map(|s| s.deleted_count),
    })
}

fn print_stats_table(path: &Path, rows: &[CollectionStatsRow], total_bytes: u64) {
    let mut table = comfy_table::Table::new();
    table.set_header([
        "Collection",
        "Kind",
        "Points",
        "Edges",
        "Disk bytes",
        "Deleted",
        "Last ANALYZE",
    ]);
    for row in rows {
        table.add_row([
            row.name.clone(),
            row.kind.to_owned(),
            row.points.to_string(),
            row.edges.map_or_else(|| "-".to_owned(), |e| e.to_string()),
            row.disk_bytes.to_string(),
            row.deleted_count
                .map_or_else(|| "-".to_owned(), |d| d.to_string()),
            row.last_analyzed_epoch_ms
                .map_or_else(|| "never".to_owned(), format_epoch_ms),
        ]);
    }
    println!("Database: {} ({total_bytes} bytes on disk)", path.display());
    println!("{table}");
}

fn format_epoch_ms(ms: u64) -> String {
    i64::try_from(ms)
        .ok()
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map_or_else(
            || ms.to_string(),
            |t| t.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
        )
}

/// Total size of the regular files under `dir`, not following symlinks.
pub(crate) fn dir_size(dir: &Path) -> Result<u64> {
    let mut total = 0;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            total += dir_size(&entry.path())?;
        } else if kind.is_file() {
            total += entry.metadata()?.len();
        }
    }
    Ok(total)
}

/// Handles `db vacuum`: rebuilds the HNSW index of each targeted vector
/// collection (dropping tombstones) and, with `compact`, rewrites its vector
/// storage — the core calls behind REST `POST /collections/{name}/vacuum` and
/// `/compact`. Without `collection`, every vector collection is vacuumed and
/// graph/metadata collections, which have no HNSW index, are skipped.
pub fn handle_db_vacuum(path: &Path, collection: Option<&str>, compact: bool) -> Result<()> {
    let db = crate::helpers::open_database(path)?;
    let targets = vacuum_targets(&db, collection)?;
    if targets.is_empty() {
        println!("No vector collection to vacuum.");
    }
    for (name, col) in &targets {
        vacuum_one(name, col, compact)?;
    }
    let failures = db.flush_all();
    if failures > 0 {
        bail!("vacuum done but {failures} collection(s) failed to flush");
    }
    Ok(())
}

fn vacuum_targets(db: &Database, only: Option<&str>) -> Result<Vec<(String, VectorCollection)>> {
    if let Some(name) = only {
        return match collection_helpers::resolve_collection(db, name) {
            Some(TypedCollection::Vector(c)) => Ok(vec![(name.to_owned(), c)]),
            Some(_) => {
                bail!("collection '{name}' has no HNSW index to vacuum (not a vector collection)")
            }
            None => bail!("collection '{name}' not found"),
        };
    }
    Ok(db
        .list_collections()
        .into_iter()
        .filter_map(
            |name| match collection_helpers::resolve_collection(db, &name) {
                Some(TypedCollection::Vector(c)) => Some((name, c)),
                _ => None,
            },
        )
        .collect())
}

fn vacuum_one(name: &str, col: &VectorCollection, compact: bool) -> Result<()> {
    let removed = col.rebuild_index()?;
    print!("{name}: index rebuilt, {removed} tombstoned entries removed");
    if compact {
        let reclaimed = col.compact_storage()?;
        print!(", storage compacted, {reclaimed} byte(s) reclaimed");
    }
    println!();
    Ok(())
}
