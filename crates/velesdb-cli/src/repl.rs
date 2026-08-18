#![allow(clippy::doc_markdown)]
#![allow(clippy::uninlined_format_args)]
//! REPL (Read-Eval-Print-Loop) for `VelesQL` queries
//!
//! This module owns the I/O loop (`run`) and query execution.
//! Command dispatch is delegated to [`crate::repl_commands`].

use anyhow::{Context, Result};
use colored::Colorize;
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

/// The kind of query that was executed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum QueryKind {
    /// Standard SELECT query.
    Select,
    /// DDL statement (CREATE/DROP COLLECTION).
    Ddl,
    /// DML statement (INSERT/UPDATE/DELETE).
    Dml,
    /// TRAIN statement.
    Train,
    /// Introspection statement (SHOW COLLECTIONS / DESCRIBE / EXPLAIN).
    Introspection,
    /// Admin statement (FLUSH).
    Admin,
}

/// Query execution result
#[derive(Debug)]
pub struct QueryResult {
    pub rows: Vec<HashMap<String, serde_json::Value>>,
    pub duration_ms: f64,
    /// What kind of statement produced this result.
    pub kind: QueryKind,
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

    let db = crate::helpers::open_database(&path).context("Failed to open database")?;

    let mut rl: Editor<ReplHelper, DefaultHistory> = Editor::new()?;
    rl.set_helper(Some(ReplHelper));

    let history_path = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".velesdb_history");
    let _ = rl.load_history(&history_path);

    let mut config = ReplConfig::default();

    loop {
        match rl.readline(&prompt()) {
            Ok(line) => {
                if handle_input(&db, &mut rl, &line, &mut config) == LoopAction::Quit {
                    break;
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

/// Whether the REPL loop should keep reading input or terminate.
#[derive(PartialEq)]
enum LoopAction {
    Continue,
    Quit,
}

/// Build the input prompt with platform-correct ANSI escaping.
///
/// On Unix, wrap ANSI codes in `\x01..\x02` so rustyline (readline backend)
/// correctly computes the visible prompt width. On Windows, rustyline uses the
/// crossterm backend which handles ANSI natively — the `\x01\x02` markers would
/// appear as literal garbage there and must be omitted.
fn prompt() -> String {
    #[cfg(windows)]
    {
        "\x1b[1;34mvelesdb> \x1b[0m".to_string()
    }
    #[cfg(not(windows))]
    {
        "\x01\x1b[1;34m\x02velesdb> \x01\x1b[0m\x02".to_string()
    }
}

/// Handle a single line of input: dot-commands vs. `VelesQL` queries.
fn handle_input(
    db: &Database,
    rl: &mut Editor<ReplHelper, DefaultHistory>,
    line: &str,
    config: &mut ReplConfig,
) -> LoopAction {
    let line = line.trim();
    if line.is_empty() {
        return LoopAction::Continue;
    }

    let _ = rl.add_history_entry(line);

    if line.starts_with('.') || line.starts_with('\\') {
        handle_dot_command(db, line, config)
    } else {
        run_query(db, line, config);
        LoopAction::Continue
    }
}

/// Dispatch a dot/backslash command and map its result to a [`LoopAction`].
fn handle_dot_command(db: &Database, line: &str, config: &mut ReplConfig) -> LoopAction {
    match crate::repl_commands::handle_command(db, line, config) {
        crate::repl_commands::CommandResult::Continue => LoopAction::Continue,
        crate::repl_commands::CommandResult::Quit => LoopAction::Quit,
        crate::repl_commands::CommandResult::Error(e) => {
            println!("{} {}", "Error:".red().bold(), e);
            LoopAction::Continue
        }
    }
}

/// Execute a `VelesQL` query line and print its result or error.
fn run_query(db: &Database, line: &str, config: &ReplConfig) {
    match execute_query(
        db,
        line,
        config.session.active_collection(),
        Some(&config.session),
    ) {
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

/// Execute a `VelesQL` query and return results.
///
/// Delegates to [`crate::repl_execute::execute_query`]. `session` is `Some` in
/// the interactive REPL (so `\set` settings are applied) and `None` for one-shot
/// `query execute` (no session).
pub fn execute_query(
    db: &Database,
    query: &str,
    active_collection: Option<&str>,
    session: Option<&SessionSettings>,
) -> Result<QueryResult> {
    crate::repl_execute::execute_query(db, query, active_collection, session)
}

/// Print query results in the specified format
pub fn print_result(result: &QueryResult, format: &str) {
    crate::repl_output::print_result(result, format);
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
#[path = "repl_tests.rs"]
mod repl_tests;
