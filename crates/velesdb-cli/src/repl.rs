#![allow(clippy::doc_markdown)]
#![allow(clippy::uninlined_format_args)]
//! REPL (Read-Eval-Print-Loop) for `VelesQL` queries
//!
//! This module owns the I/O loop (`run`) and query execution.
//! Command dispatch is delegated to [`crate::repl_commands`].

use anyhow::{Context, Result};
use colored::Colorize;
use instant::Instant;
use rustyline::error::ReadlineError;
use rustyline::history::DefaultHistory;
use rustyline::{Completer, Editor, Helper, Highlighter, Hinter, Validator};
use std::collections::HashMap;
use std::path::PathBuf;
use velesdb_core::Database;

use crate::session::SessionSettings;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// REPL configuration
#[derive(Debug, Clone)]
pub struct ReplConfig {
    pub timing: bool,
    pub format: OutputFormat,
    pub session: SessionSettings,
}

impl Default for ReplConfig {
    fn default() -> Self {
        Self {
            timing: true,
            format: OutputFormat::Table,
            session: SessionSettings::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OutputFormat {
    Table,
    Json,
}

/// Query execution result
#[derive(Debug)]
pub struct QueryResult {
    pub rows: Vec<HashMap<String, serde_json::Value>>,
    pub duration_ms: f64,
}

#[derive(Completer, Helper, Highlighter, Hinter, Validator)]
struct ReplHelper;

/// Run the interactive REPL
#[allow(clippy::needless_pass_by_value)] // PathBuf ownership required for Database::open
pub fn run(path: PathBuf) -> Result<()> {
    println!(
        "\n{}",
        format!("VelesDB v{VERSION} - VelesQL REPL").bold().cyan()
    );
    println!("Database: {}", path.display().to_string().green());
    println!(
        "Type {} for commands, {} to exit\n",
        ".help".yellow(),
        ".quit".yellow()
    );

    let db = Database::open(&path).context("Failed to open database")?;

    let mut rl: Editor<ReplHelper, DefaultHistory> = Editor::new()?;
    rl.set_helper(Some(ReplHelper));

    let history_path = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".velesdb_history");
    let _ = rl.load_history(&history_path);

    let mut config = ReplConfig::default();

    loop {
        let prompt = "velesdb> ".bold().blue().to_string();
        match rl.readline(&prompt) {
            Ok(line) => {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }

                let _ = rl.add_history_entry(line);

                if line.starts_with('.') || line.starts_with('\\') {
                    match crate::repl_commands::handle_command(&db, line, &mut config) {
                        crate::repl_commands::CommandResult::Continue => (),
                        crate::repl_commands::CommandResult::Quit => break,
                        crate::repl_commands::CommandResult::Error(e) => {
                            println!("{} {}", "Error:".red().bold(), e);
                        }
                    }
                } else {
                    match execute_query(&db, line) {
                        Ok(result) => {
                            let fmt = match config.format {
                                OutputFormat::Table => "table",
                                OutputFormat::Json => "json",
                            };
                            print_result(&result, fmt);
                            if config.timing {
                                println!(
                                    "\n{} rows ({:.2}ms)\n",
                                    result.rows.len().to_string().green(),
                                    result.duration_ms
                                );
                            }
                        }
                        Err(e) => {
                            println!("{} {}\n", "Error:".red().bold(), e);
                        }
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("Use .quit to exit");
            }
            Err(ReadlineError::Eof) => {
                break;
            }
            Err(err) => {
                println!("{} {:?}", "Error:".red().bold(), err);
                break;
            }
        }
    }

    let _ = rl.save_history(&history_path);
    println!("Goodbye!");
    Ok(())
}

/// Execute a `VelesQL` query and return results.
///
/// Delegates to [`Database::execute_query`] which handles JOINs, plan caching
/// and validation centrally -- the CLI no longer calls `Collection::execute_query`
/// directly.
pub fn execute_query(db: &Database, query: &str) -> Result<QueryResult> {
    let start = Instant::now();

    // Parse the query
    let parsed = velesdb_core::velesql::Parser::parse(query)
        .map_err(|e| anyhow::anyhow!("Parse error: {}", e.message))?;

    // Check if there's a vector search requiring parameters
    let has_param_vector = parsed
        .select
        .where_clause
        .as_ref()
        .is_some_and(contains_param_vector);

    if has_param_vector {
        // Vector search with parameter requires external input
        println!(
            "{}",
            "Note: Vector search with $parameter requires REST API. Use literal vectors or metadata-only queries."
                .yellow()
        );
        let duration_ms = start.elapsed().as_secs_f64() * 1000.0;
        return Ok(QueryResult {
            rows: Vec::new(),
            duration_ms,
        });
    }

    // Delegate to Database::execute_query which handles JOINs and plan caching
    let params = HashMap::new();
    let results = db
        .execute_query(&parsed, &params)
        .map_err(|e| anyhow::anyhow!("Query error: {e}"))?;

    // Convert SearchResult to row format
    let rows: Vec<HashMap<String, serde_json::Value>> = results
        .into_iter()
        .map(|r| {
            let mut row = HashMap::new();
            row.insert("id".to_string(), serde_json::json!(r.point.id));
            row.insert("score".to_string(), serde_json::json!(r.score));

            if let Some(serde_json::Value::Object(map)) = &r.point.payload {
                for (k, v) in map {
                    row.insert(k.clone(), v.clone());
                }
            }
            row
        })
        .collect();

    let duration_ms = start.elapsed().as_secs_f64() * 1000.0;

    Ok(QueryResult { rows, duration_ms })
}

fn contains_param_vector(condition: &velesdb_core::velesql::Condition) -> bool {
    use velesdb_core::velesql::{Condition, SparseVectorExpr, VectorExpr};
    match condition {
        Condition::VectorSearch(vs) => matches!(vs.vector, VectorExpr::Parameter(_)),
        Condition::VectorFusedSearch(vfs) => vfs
            .vectors
            .iter()
            .any(|v| matches!(v, VectorExpr::Parameter(_))),
        Condition::SparseVectorSearch(svs) => {
            matches!(svs.vector, SparseVectorExpr::Parameter(_))
        }
        Condition::Similarity(sim) => matches!(sim.vector, VectorExpr::Parameter(_)),
        Condition::And(left, right) | Condition::Or(left, right) => {
            contains_param_vector(left) || contains_param_vector(right)
        }
        Condition::Not(inner) | Condition::Group(inner) => contains_param_vector(inner),
        Condition::Comparison(_)
        | Condition::In(_)
        | Condition::Between(_)
        | Condition::Like(_)
        | Condition::IsNull(_)
        | Condition::Match(_)
        | Condition::GraphMatch(_) => false,
    }
}

#[cfg(test)]
fn contains_vector_search(condition: &velesdb_core::velesql::Condition) -> bool {
    use velesdb_core::velesql::Condition;
    match condition {
        Condition::VectorSearch(_) => true,
        Condition::And(left, right) | Condition::Or(left, right) => {
            contains_vector_search(left) || contains_vector_search(right)
        }
        Condition::Group(inner) => contains_vector_search(inner),
        _ => false,
    }
}

/// Print query results in the specified format
pub fn print_result(result: &QueryResult, format: &str) {
    crate::repl_output::print_result(result, format);
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use velesdb_core::velesql::{
        CompareOp, Comparison, Condition, FusionConfig, SimilarityCondition, SparseVectorExpr,
        SparseVectorSearch, Value, VectorExpr, VectorFusedSearch, VectorSearch,
    };

    // =========================================================================
    // Tests for ReplConfig
    // =========================================================================

    #[test]
    fn test_repl_config_default() {
        let config = ReplConfig::default();
        assert!(config.timing);
        assert_eq!(config.format, OutputFormat::Table);
    }

    #[test]
    fn test_output_format_eq() {
        assert_eq!(OutputFormat::Table, OutputFormat::Table);
        assert_eq!(OutputFormat::Json, OutputFormat::Json);
        assert_ne!(OutputFormat::Table, OutputFormat::Json);
    }

    // =========================================================================
    // Tests for QueryResult
    // =========================================================================

    #[test]
    fn test_query_result_empty() {
        let result = QueryResult {
            rows: vec![],
            duration_ms: 0.0,
        };
        assert!(result.rows.is_empty());
        assert!((result.duration_ms - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_query_result_with_data() {
        let mut row = HashMap::new();
        row.insert("id".to_string(), json!(1));
        row.insert("name".to_string(), json!("test"));

        let result = QueryResult {
            rows: vec![row],
            duration_ms: 1.5,
        };

        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0].get("id"), Some(&json!(1)));
        assert!((result.duration_ms - 1.5).abs() < f64::EPSILON);
    }

    // =========================================================================
    // Tests for contains_vector_search
    // =========================================================================

    #[test]
    fn test_contains_vector_search_with_vector() {
        let condition = Condition::VectorSearch(VectorSearch {
            vector: VectorExpr::Literal(vec![0.1, 0.2]),
        });
        assert!(contains_vector_search(&condition));
    }

    #[test]
    fn test_contains_vector_search_without_vector() {
        let condition = Condition::Comparison(Comparison {
            column: "category".to_string(),
            operator: CompareOp::Eq,
            value: Value::String("tech".to_string()),
        });
        assert!(!contains_vector_search(&condition));
    }

    #[test]
    fn test_contains_vector_search_nested_and() {
        let vector_cond = Condition::VectorSearch(VectorSearch {
            vector: VectorExpr::Literal(vec![0.1]),
        });
        let other_cond = Condition::Comparison(Comparison {
            column: "x".to_string(),
            operator: CompareOp::Eq,
            value: Value::Integer(1),
        });
        let combined = Condition::And(Box::new(other_cond), Box::new(vector_cond));
        assert!(contains_vector_search(&combined));
    }

    #[test]
    fn test_contains_vector_search_nested_or() {
        let vector_cond = Condition::VectorSearch(VectorSearch {
            vector: VectorExpr::Literal(vec![0.1]),
        });
        let other_cond = Condition::Comparison(Comparison {
            column: "x".to_string(),
            operator: CompareOp::Eq,
            value: Value::Integer(1),
        });
        let combined = Condition::Or(Box::new(other_cond), Box::new(vector_cond));
        assert!(contains_vector_search(&combined));
    }

    #[test]
    fn test_contains_vector_search_group() {
        let vector_cond = Condition::VectorSearch(VectorSearch {
            vector: VectorExpr::Literal(vec![0.1]),
        });
        let grouped = Condition::Group(Box::new(vector_cond));
        assert!(contains_vector_search(&grouped));
    }

    #[test]
    fn test_contains_vector_search_no_match() {
        let cond_a = Condition::Comparison(Comparison {
            column: "a".to_string(),
            operator: CompareOp::Eq,
            value: Value::Integer(1),
        });
        let cond_b = Condition::Comparison(Comparison {
            column: "b".to_string(),
            operator: CompareOp::Gt,
            value: Value::Integer(2),
        });
        let condition = Condition::And(Box::new(cond_a), Box::new(cond_b));
        assert!(!contains_vector_search(&condition));
    }

    // =========================================================================
    // Tests for contains_param_vector (Phase 1.1 -- exhaustive variants)
    // =========================================================================

    #[test]
    fn test_contains_param_vector_vector_search_param() {
        let cond = Condition::VectorSearch(VectorSearch {
            vector: VectorExpr::Parameter("v".to_string()),
        });
        assert!(contains_param_vector(&cond));
    }

    #[test]
    fn test_contains_param_vector_vector_search_literal() {
        let cond = Condition::VectorSearch(VectorSearch {
            vector: VectorExpr::Literal(vec![0.1, 0.2]),
        });
        assert!(!contains_param_vector(&cond));
    }

    #[test]
    fn test_contains_param_vector_fused_search_param() {
        let cond = Condition::VectorFusedSearch(VectorFusedSearch {
            vectors: vec![
                VectorExpr::Literal(vec![0.1]),
                VectorExpr::Parameter("q".to_string()),
            ],
            fusion: FusionConfig::default(),
        });
        assert!(contains_param_vector(&cond));
    }

    #[test]
    fn test_contains_param_vector_fused_search_all_literal() {
        let cond = Condition::VectorFusedSearch(VectorFusedSearch {
            vectors: vec![
                VectorExpr::Literal(vec![0.1]),
                VectorExpr::Literal(vec![0.2]),
            ],
            fusion: FusionConfig::default(),
        });
        assert!(!contains_param_vector(&cond));
    }

    #[test]
    fn test_contains_param_vector_sparse_search_param() {
        let cond = Condition::SparseVectorSearch(SparseVectorSearch {
            vector: SparseVectorExpr::Parameter("sv".to_string()),
            index_name: None,
        });
        assert!(contains_param_vector(&cond));
    }

    #[test]
    fn test_contains_param_vector_sparse_search_literal() {
        use velesdb_core::sparse_index::SparseVector;
        let cond = Condition::SparseVectorSearch(SparseVectorSearch {
            vector: SparseVectorExpr::Literal(SparseVector::new(vec![(0, 1.0)])),
            index_name: None,
        });
        assert!(!contains_param_vector(&cond));
    }

    #[test]
    fn test_contains_param_vector_similarity_param() {
        let cond = Condition::Similarity(SimilarityCondition {
            field: "embedding".to_string(),
            vector: VectorExpr::Parameter("q".to_string()),
            operator: CompareOp::Gt,
            threshold: 0.8,
        });
        assert!(contains_param_vector(&cond));
    }

    #[test]
    fn test_contains_param_vector_similarity_literal() {
        let cond = Condition::Similarity(SimilarityCondition {
            field: "embedding".to_string(),
            vector: VectorExpr::Literal(vec![0.1, 0.2]),
            operator: CompareOp::Gt,
            threshold: 0.8,
        });
        assert!(!contains_param_vector(&cond));
    }

    #[test]
    fn test_contains_param_vector_not_recurses() {
        let inner = Condition::VectorSearch(VectorSearch {
            vector: VectorExpr::Parameter("v".to_string()),
        });
        let cond = Condition::Not(Box::new(inner));
        assert!(contains_param_vector(&cond));
    }

    #[test]
    fn test_contains_param_vector_not_no_param() {
        let inner = Condition::Comparison(Comparison {
            column: "x".to_string(),
            operator: CompareOp::Eq,
            value: Value::Integer(1),
        });
        let cond = Condition::Not(Box::new(inner));
        assert!(!contains_param_vector(&cond));
    }

    #[test]
    fn test_contains_param_vector_comparison_false() {
        let cond = Condition::Comparison(Comparison {
            column: "cat".to_string(),
            operator: CompareOp::Eq,
            value: Value::String("tech".to_string()),
        });
        assert!(!contains_param_vector(&cond));
    }

    #[test]
    fn test_contains_param_vector_nested_and_with_similarity() {
        let sim = Condition::Similarity(SimilarityCondition {
            field: "vec".to_string(),
            vector: VectorExpr::Parameter("q".to_string()),
            operator: CompareOp::Gt,
            threshold: 0.5,
        });
        let comp = Condition::Comparison(Comparison {
            column: "status".to_string(),
            operator: CompareOp::Eq,
            value: Value::String("active".to_string()),
        });
        let combined = Condition::And(Box::new(sim), Box::new(comp));
        assert!(contains_param_vector(&combined));
    }

    // =========================================================================
    // Tests for print_result (output format logic)
    // =========================================================================

    #[test]
    fn test_print_result_empty() {
        let result = QueryResult {
            rows: vec![],
            duration_ms: 0.0,
        };
        // Should not panic on empty results
        print_result(&result, "table");
        print_result(&result, "json");
    }

    #[test]
    fn test_print_result_json_format() {
        let mut row = HashMap::new();
        row.insert("id".to_string(), json!(1));

        let result = QueryResult {
            rows: vec![row],
            duration_ms: 1.0,
        };
        // Should not panic
        print_result(&result, "json");
    }

    #[test]
    fn test_print_result_table_format() {
        let mut row = HashMap::new();
        row.insert("id".to_string(), json!(42));
        row.insert("name".to_string(), json!("test"));

        let result = QueryResult {
            rows: vec![row],
            duration_ms: 2.0,
        };
        // Should not panic
        print_result(&result, "table");
    }
}
