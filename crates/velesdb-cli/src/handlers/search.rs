//! Handler for the `multi-search` subcommand.

use std::path::Path;

use anyhow::Result;
use colored::Colorize;
use velesdb_core::{FusionParams, FusionStrategy};

/// Handles the `multi-search` subcommand: multi-query vector fusion search.
pub fn handle_multi_search(
    path: &Path,
    collection: &str,
    vectors: &str,
    top_k: usize,
    strategy: &str,
    rrf_k: u32,
    format: &str,
) -> Result<()> {
    let col = open_vector_collection(path, collection)?;
    let parsed_vectors = parse_query_vectors(vectors)?;
    let fusion_strategy = parse_fusion_strategy(strategy, rrf_k)?;
    let results = execute_multi_search(&col, &parsed_vectors, top_k, fusion_strategy)?;

    print_results(&results, strategy, parsed_vectors.len(), format)
}

/// Opens a database and returns the named vector collection.
fn open_vector_collection(path: &Path, collection: &str) -> Result<velesdb_core::VectorCollection> {
    let db = crate::helpers::open_database(path)?;
    db.get_vector_collection(collection)
        .ok_or_else(|| anyhow::anyhow!("Collection '{}' not found", collection))
}

/// Parses and validates the query vectors JSON.
fn parse_query_vectors(raw: &str) -> Result<Vec<Vec<f32>>> {
    let vectors: Vec<Vec<f32>> =
        serde_json::from_str(raw).map_err(|e| anyhow::anyhow!("Invalid vectors JSON: {e}"))?;
    if vectors.is_empty() {
        anyhow::bail!("At least one query vector is required");
    }
    Ok(vectors)
}

/// Executes the multi-query search against a collection.
fn execute_multi_search(
    col: &velesdb_core::VectorCollection,
    parsed_vectors: &[Vec<f32>],
    top_k: usize,
    fusion_strategy: FusionStrategy,
) -> Result<Vec<velesdb_core::SearchResult>> {
    let query_refs: Vec<&[f32]> = parsed_vectors.iter().map(Vec::as_slice).collect();
    col.multi_query_search(&query_refs, top_k, fusion_strategy, None)
        .map_err(|e| anyhow::anyhow!("Search failed: {e}"))
}

/// Parses a fusion strategy name into the core enum, through core's name
/// grammar ([`FusionStrategy::from_name`]): its aliases, its case rule and its
/// defaults, so `weighted` here ranks as it does on every other surface.
/// Shared by `multi-search` and the REPL's `.hybrid-sparse`.
pub(crate) fn parse_fusion_strategy(strategy: &str, rrf_k: u32) -> Result<FusionStrategy> {
    FusionStrategy::from_name(strategy, FusionParams::default().with_rrf_k(Some(rrf_k)))
        .map_err(|e| anyhow::anyhow!("{e}"))
}

/// Dispatches search results to the appropriate output format.
fn print_results(
    results: &[velesdb_core::SearchResult],
    strategy: &str,
    vector_count: usize,
    format: &str,
) -> Result<()> {
    if format == "json" {
        print_search_json(results)
    } else {
        print_search_table(results, strategy, vector_count);
        Ok(())
    }
}

/// Prints multi-search results as JSON.
fn print_search_json(results: &[velesdb_core::SearchResult]) -> Result<()> {
    let output: Vec<_> = results
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.point.id,
                "score": r.score,
                "payload": r.point.payload
            })
        })
        .collect();
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

/// Prints multi-search results as a colored table.
fn print_search_table(results: &[velesdb_core::SearchResult], strategy: &str, vector_count: usize) {
    println!(
        "\n{} (strategy: {}, {} vectors)",
        "Multi-Query Search Results".bold().underline(),
        strategy.green(),
        vector_count
    );
    if results.is_empty() {
        println!("  No results found.\n");
        return;
    }
    for (i, r) in results.iter().enumerate() {
        println!(
            "  {}. ID: {} (score: {:.4})",
            i + 1,
            r.point.id.to_string().green(),
            r.score
        );
        if let Some(payload) = &r.point.payload {
            println!("     Payload: {payload}");
        }
    }
    println!("\n  Total: {} result(s)\n", results.len());
}

/// Handles the `batch-search` subcommand: independent parallel searches.
pub fn handle_batch_search(
    path: &Path,
    collection: &str,
    vectors: &str,
    top_k: usize,
    format: &str,
) -> Result<()> {
    let col = open_vector_collection(path, collection)?;
    let parsed_vectors = parse_query_vectors(vectors)?;

    let query_refs: Vec<&[f32]> = parsed_vectors.iter().map(Vec::as_slice).collect();
    let no_filters: Vec<Option<velesdb_core::Filter>> = vec![None; parsed_vectors.len()];

    let batch_results = col
        .search_batch_with_filters(&query_refs, top_k, &no_filters)
        .map_err(|e| anyhow::anyhow!("Batch search failed: {e}"))?;

    if format == "json" {
        print_batch_json(&batch_results)
    } else {
        print_batch_table(&batch_results);
        Ok(())
    }
}

/// Prints batch search results as JSON.
fn print_batch_json(batch: &[Vec<velesdb_core::SearchResult>]) -> Result<()> {
    let output: Vec<_> = batch
        .iter()
        .enumerate()
        .map(|(i, results)| {
            serde_json::json!({
                "query": i,
                "results": results.iter().map(|r| {
                    serde_json::json!({
                        "id": r.point.id,
                        "score": r.score,
                        "payload": r.point.payload
                    })
                }).collect::<Vec<_>>()
            })
        })
        .collect();
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

/// Prints batch search results as a colored table.
fn print_batch_table(batch: &[Vec<velesdb_core::SearchResult>]) {
    for (i, results) in batch.iter().enumerate() {
        println!(
            "\n{} (query {})",
            "Batch Search Results".bold().underline(),
            (i + 1).to_string().cyan()
        );
        if results.is_empty() {
            println!("  No results found.");
            continue;
        }
        for (j, r) in results.iter().enumerate() {
            println!(
                "  {}. ID: {} (score: {:.4})",
                j + 1,
                r.point.id.to_string().green(),
                r.score
            );
            if let Some(payload) = &r.point.payload {
                println!("     Payload: {payload}");
            }
        }
        println!("  Total: {} result(s)", results.len());
    }
    println!();
}

#[cfg(test)]
#[path = "search_tests.rs"]
mod tests;
