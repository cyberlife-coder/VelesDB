//! REPL commands for data operations (bench and export).
//!
//! Covers: `.bench`, `.export`.

use colored::Colorize;
use instant::Instant;
use velesdb_core::Database;

use crate::collection_helpers;
use crate::helpers;
use crate::repl_commands::CommandResult;

pub(crate) fn cmd_bench(
    db: &Database,
    config: &crate::repl::ReplConfig,
    parts: &[&str],
) -> CommandResult {
    if parts.len() < 2 {
        println!("Usage: .bench <collection_name> [n_queries] [k]\n");
        return CommandResult::Continue;
    }
    let name = parts[1];
    let n_queries: usize = parts
        .get(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(100)
        .max(1);
    let k: usize = parts.get(3).and_then(|s| s.parse().ok()).unwrap_or(10);

    match db.get_vector_collection(name) {
        Some(col) => {
            let cfg = col.config();
            println!(
                "\nBenchmarking {} ({} points, {}D)...\n",
                name.green(),
                cfg.point_count,
                cfg.dimension
            );
            println!(
                "  {} queries, k={}, mode={:?}",
                n_queries,
                k,
                config.session.mode()
            );

            // Generate random query vectors
            let start = Instant::now();
            let mut total_results = 0usize;

            for i in 0..n_queries {
                // Use deterministic pseudo-random for reproducibility
                let query: Vec<f32> = (0..cfg.dimension)
                    .map(|j| ((i * 31 + j * 17) % 1000) as f32 / 1000.0)
                    .collect();

                if let Ok(results) = col.search(&query, k) {
                    total_results += results.len();
                }
            }

            let elapsed = start.elapsed();
            let qps = n_queries as f64 / elapsed.as_secs_f64();
            let avg_latency_ms = elapsed.as_millis() as f64 / n_queries as f64;

            println!("\n{}", "Results:".bold());
            println!("  {} {:.2} queries/sec", "Throughput:".cyan(), qps);
            println!("  {} {:.2} ms", "Avg Latency:".cyan(), avg_latency_ms);
            println!("  {} {} results", "Total Results:".cyan(), total_results);
            println!();
        }
        None => {
            return CommandResult::Error(format!("Collection '{name}' not found"));
        }
    }
    CommandResult::Continue
}

pub(crate) fn cmd_export(db: &Database, parts: &[&str]) -> CommandResult {
    if parts.len() < 2 {
        println!("Usage: .export <collection_name> [filename.json]\n");
        return CommandResult::Continue;
    }
    let name = parts[1];
    let filename = parts
        .get(2)
        .map_or_else(|| format!("{name}.json"), std::string::ToString::to_string);

    match collection_helpers::resolve_collection(db, name) {
        Some(collection_helpers::TypedCollection::Vector(col)) => {
            let all_ids = col.all_ids();
            let total = all_ids.len();
            println!("Exporting {} records from {}...", total, name.green());

            let records = export_points(&all_ids, |batch| col.get(batch), true);
            match finish_export(&records, &filename) {
                Ok(()) => {}
                Err(msg) => return CommandResult::Error(msg),
            }
        }
        Some(collection_helpers::TypedCollection::Metadata(col)) => {
            let all_ids = col.all_ids();
            let total = all_ids.len();
            println!(
                "Exporting {} records from {} (Metadata)...",
                total,
                name.green()
            );

            let records = export_points(&all_ids, |batch| col.get(batch), false);
            match finish_export(&records, &filename) {
                Ok(()) => {}
                Err(msg) => return CommandResult::Error(msg),
            }
        }
        Some(collection_helpers::TypedCollection::Graph(_)) => {
            return CommandResult::Error(
                "Export is not supported for Graph collections. Use 'velesdb graph get-edges' instead.".to_string()
            );
        }
        None => {
            return CommandResult::Error(format!("Collection '{name}' not found"));
        }
    }
    CommandResult::Continue
}

// ============================================================================
// Export helpers
// ============================================================================

/// Exports points in batches, building a JSON record for each.
///
/// `include_vector` controls whether the vector field is included (true for
/// Vector collections, false for Metadata).
fn export_points<F>(all_ids: &[u64], get_batch: F, include_vector: bool) -> Vec<serde_json::Value>
where
    F: Fn(&[u64]) -> Vec<Option<velesdb_core::Point>>,
{
    let batch_size = 1000;
    let mut records = Vec::new();

    for batch in all_ids.chunks(batch_size) {
        let points = get_batch(batch);
        for point in points.into_iter().flatten() {
            let vector = if include_vector {
                Some(point.vector.as_slice())
            } else {
                None
            };
            records.push(helpers::point_to_export_record(
                point.id,
                vector,
                &point.payload,
            ));
        }
    }
    records
}

/// Serializes records and writes them, printing a success message.
fn finish_export(records: &[serde_json::Value], filename: &str) -> Result<(), String> {
    helpers::write_export_file(records, filename)?;
    println!(
        "{} Exported {} records to {}\n",
        "\u{2713}".green(),
        records.len(),
        filename.green()
    );
    Ok(())
}
