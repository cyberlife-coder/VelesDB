//! REPL commands for advanced search and agent features.
//!
//! Covers: `.sparse-search`, `.hybrid-sparse`, `.agent`, `.guardrails`.

use colored::Colorize;
use velesdb_core::Database;

use crate::repl_commands::{parse_flag, CommandResult};

/// Display search results with a title header.
fn print_search_results(results: &[velesdb_core::SearchResult], title: &str, extra: &str) {
    if results.is_empty() {
        println!("No results found.\n");
    } else {
        println!(
            "\n{} ({} results{})\n",
            title.bold().underline(),
            results.len(),
            extra
        );
        for r in results {
            println!(
                "  id={} score={:.6}",
                r.point.id.to_string().cyan(),
                r.score
            );
        }
        println!();
    }
}

/// Parse a JSON string into a `SparseVector`, returning a `CommandResult::Error` on failure.
fn parse_sparse_json(json: &str) -> Result<velesdb_core::sparse_index::SparseVector, String> {
    let pairs: Vec<(u32, f32)> = serde_json::from_str(json).map_err(|e| {
        format!("Invalid sparse vector JSON: {e}\nExpected format: [[index, weight], ...]")
    })?;
    Ok(velesdb_core::sparse_index::SparseVector::new(pairs))
}

pub(crate) fn cmd_sparse_search(db: &Database, parts: &[&str]) -> CommandResult {
    if parts.len() < 4 {
        println!("Usage: .sparse-search <collection> <index_name> <json_sparse_vector> [k]\n");
        println!(
            "  JSON sparse vector format: {}",
            "[[index, weight], ...]".italic().white()
        );
        println!(
            "  Example: {} my_col \"\" [[0,1.5],[3,0.8]] 10\n",
            ".sparse-search".yellow()
        );
        return CommandResult::Continue;
    }
    let name = parts[1];
    let index_name = parts[2];
    let sparse_json = parts[3];
    let k: usize = parts.get(4).and_then(|s| s.parse().ok()).unwrap_or(10);

    let sparse_vec = match parse_sparse_json(sparse_json) {
        Ok(v) => v,
        Err(e) => return CommandResult::Error(e),
    };

    match db.get_vector_collection(name) {
        Some(col) => match col.sparse_search(&sparse_vec, k, index_name) {
            Ok(results) => print_search_results(&results, "Sparse Search Results", ""),
            Err(e) => return CommandResult::Error(format!("Sparse search error: {e}")),
        },
        None => {
            return CommandResult::Error(format!("Collection '{name}' not found"));
        }
    }
    CommandResult::Continue
}

#[allow(clippy::too_many_lines)]
pub(crate) fn cmd_hybrid_sparse(db: &Database, parts: &[&str]) -> CommandResult {
    if parts.len() < 4 {
        println!(
            "Usage: .hybrid-sparse <collection> <dense_json> <sparse_json> [k] \
             [--strategy rrf|average|max] [--index <name>]\n"
        );
        println!("  Dense vector:  {}", "[0.1, 0.2, ...]".italic().white());
        println!(
            "  Sparse vector: {}",
            "[[index, weight], ...]".italic().white()
        );
        println!(
            "  Example: {} docs [0.1,0.2,0.3,0.4] [[0,1.5],[3,0.8]] 10 --strategy rrf\n",
            ".hybrid-sparse".yellow()
        );
        return CommandResult::Continue;
    }
    let name = parts[1];
    let dense_json = parts[2];
    let sparse_json = parts[3];
    let k: usize = parts.get(4).and_then(|s| s.parse().ok()).unwrap_or(10);

    let strategy_str = parse_flag(parts, "--strategy").unwrap_or_else(|| "rrf".to_string());
    let index_name = parse_flag(parts, "--index").unwrap_or_default();

    let dense_vector: Vec<f32> = match serde_json::from_str(dense_json) {
        Ok(v) => v,
        Err(e) => {
            return CommandResult::Error(format!(
                "Invalid dense vector JSON: {e}\nExpected format: [0.1, 0.2, ...]"
            ));
        }
    };

    let sparse_vec = match parse_sparse_json(sparse_json) {
        Ok(v) => v,
        Err(e) => return CommandResult::Error(e),
    };

    let strategy = match strategy_str.as_str() {
        "rrf" => velesdb_core::FusionStrategy::rrf_default(),
        "average" => velesdb_core::FusionStrategy::Average,
        "max" | "maximum" => velesdb_core::FusionStrategy::Maximum,
        other => {
            return CommandResult::Error(format!(
                "Unknown fusion strategy: '{other}'. Use rrf, average, or max."
            ));
        }
    };

    match db.get_vector_collection(name) {
        Some(col) => {
            match col.hybrid_sparse_search(&dense_vector, &sparse_vec, k, &index_name, &strategy) {
                Ok(results) => {
                    let extra = format!(", strategy={}", strategy_str.cyan());
                    print_search_results(&results, "Hybrid Sparse Search Results", &extra);
                }
                Err(e) => return CommandResult::Error(format!("Hybrid search error: {e}")),
            }
        }
        None => {
            return CommandResult::Error(format!("Collection '{name}' not found"));
        }
    }
    CommandResult::Continue
}

pub(crate) fn cmd_agent(parts: &[&str]) -> CommandResult {
    if parts.len() < 2 {
        print_agent_help();
        return CommandResult::Continue;
    }
    match parts[1] {
        "help" => print_agent_help(),
        sub => {
            println!(
                "{} Agent subcommand '{}' is not yet implemented.\n\
                 Agent memory management is primarily used via SDK/server.\n",
                "\u{2139}".cyan(),
                sub
            );
        }
    }
    CommandResult::Continue
}

fn print_agent_help() {
    println!("\n{}", "Agent Commands (Preview)".bold().underline());
    println!();
    println!("  Agent memory management is primarily used via the SDK or REST server.");
    println!("  CLI support is planned for a future release.\n");
    println!("  Planned subcommands:");
    println!("    {} Store a memory entry", ".agent store".yellow());
    println!("    {} Recall memories", ".agent recall".yellow());
    println!("    {} List stored memories", ".agent list".yellow());
    println!("    {} Clear agent memory", ".agent clear".yellow());
    println!();
}

pub(crate) fn cmd_guardrails() -> CommandResult {
    let limits = velesdb_core::guardrails::QueryLimits::default();
    println!("\n{}", "Query Guard-Rails Configuration".bold().underline());
    println!();
    println!("  {} {} ms", "Timeout:".cyan(), limits.timeout_ms);
    println!("  {} {}", "Max Depth:".cyan(), limits.max_depth);
    println!("  {} {}", "Max Cardinality:".cyan(), limits.max_cardinality);
    println!(
        "  {} {} bytes ({:.0} MB)",
        "Memory Limit:".cyan(),
        limits.memory_limit_bytes,
        limits.memory_limit_bytes as f64 / (1024.0 * 1024.0)
    );
    println!("  {} {} qps", "Rate Limit:".cyan(), limits.rate_limit_qps);
    println!(
        "  {} {} failures",
        "Circuit Breaker Threshold:".cyan(),
        limits.circuit_failure_threshold
    );
    println!(
        "  {} {} s",
        "Circuit Recovery:".cyan(),
        limits.circuit_recovery_seconds
    );
    println!();
    CommandResult::Continue
}
