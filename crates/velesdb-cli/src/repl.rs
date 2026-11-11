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
    pub agent: crate::agent::AgentSession,
}

impl Default for ReplConfig {
    fn default() -> Self {
        Self {
            timing: true,
            format: OutputFormat::Table,
            session: SessionSettings::new(),
            agent: crate::agent::AgentSession::default(),
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
pub(crate) struct ReplHelper;

/// Line editor type used by the REPL loop (rustyline with the default
/// file-backed history). Exposed crate-wide so meta commands
/// (`\history`, `\load`, `\edit`) can drive the same editor.
pub(crate) type ReplEditor = Editor<ReplHelper, DefaultHistory>;

/// An attached database handle: the `Database` plus the path it was
/// opened from (shown by `\databases`).
pub(crate) struct AttachedDb {
    pub(crate) db: Database,
    pub(crate) path: PathBuf,
}

/// Mutable per-session state the daily-driver meta commands operate on:
/// the attached database handles (`\attach`/`\connect`/`\detach`),
/// the last submitted query (`\edit`, `\watch`, `\save`) and the last
/// successful result (`\save --result`, `\copy`).
///
/// `last_query` is recorded even when the query fails — fixing a failed
/// query is exactly what `\edit` is for.
pub(crate) struct ReplState {
    /// Attached databases as (alias, handle) pairs in attach order;
    /// `"main"` is the database the REPL was started on. `active` names
    /// the alias every command runs on. A `Vec`, not a map — a session
    /// holds a handful of aliases, so a linear scan is exact, preserves
    /// attach order for `\databases`, and instantiates no map machinery.
    pub(crate) databases: Vec<(String, AttachedDb)>,
    pub(crate) active: String,
    pub(crate) last_query: Option<String>,
    pub(crate) last_result: Option<QueryResult>,
    /// Depth of nested `\load` script execution; a script that (transitively)
    /// loads itself errors instead of overflowing the stack.
    pub(crate) load_depth: u8,
}

/// The alias the database the REPL opened on is registered under.
pub(crate) const MAIN_ALIAS: &str = "main";

impl ReplState {
    pub(crate) fn new(db: Database, path: PathBuf) -> Self {
        Self {
            databases: vec![(MAIN_ALIAS.to_string(), AttachedDb { db, path })],
            active: MAIN_ALIAS.to_string(),
            last_query: None,
            last_result: None,
            load_depth: 0,
        }
    }

    /// The database commands run against — the `\connect`-selected alias.
    pub(crate) fn current(&self) -> &Database {
        // `active` is only ever set to an alias this vec holds (\attach and
        // \connect are the single writers), so the lookup cannot miss.
        self.db_by_alias(&self.active)
            .map_or_else(|| unreachable!("active alias always present"), |a| &a.db)
    }

    /// The attached database registered under `alias`, if any.
    pub(crate) fn db_by_alias(&self, alias: &str) -> Option<&AttachedDb> {
        self.databases
            .iter()
            .find(|(name, _)| name == alias)
            .map(|(_, attached)| attached)
    }
}

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
    let mut state = ReplState::new(db, path);

    let mut rl: ReplEditor = Editor::new()?;
    rl.set_helper(Some(ReplHelper));

    let history_path = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".velesdb_history");
    let _ = rl.load_history(&history_path);

    let mut config = ReplConfig::default();

    loop {
        match rl.readline(&prompt()) {
            Ok(line) => {
                if handle_input(&mut state, &mut rl, &line, &mut config) == LoopAction::Quit {
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
pub(crate) enum LoopAction {
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

/// Handle a single line of input: record it in the line-editor history,
/// then dispatch it.
fn handle_input(
    state: &mut ReplState,
    rl: &mut ReplEditor,
    line: &str,
    config: &mut ReplConfig,
) -> LoopAction {
    let line = line.trim();
    if line.is_empty() {
        return LoopAction::Continue;
    }

    let _ = rl.add_history_entry(line);
    run_line(state, rl, line, config)
}

/// Dispatch one input line — the shared path for typed lines, `\load`
/// script lines and `\edit` output: dot/backslash commands first, then
/// CLI-style subcommand aliases, then raw `VelesQL`.
pub(crate) fn run_line(
    state: &mut ReplState,
    rl: &mut ReplEditor,
    line: &str,
    config: &mut ReplConfig,
) -> LoopAction {
    let line = line.trim();
    if line.is_empty() {
        return LoopAction::Continue;
    }

    if line.starts_with('.') || line.starts_with('\\') {
        handle_dot_command(state, rl, line, config)
    } else {
        match crate::repl_cli_alias::translate_cli_line(line) {
            crate::repl_cli_alias::CliLine::DotCommand(cmd) => {
                handle_dot_command(state, rl, &cmd, config)
            }
            crate::repl_cli_alias::CliLine::Query(query) => {
                run_query(state, &query, config);
                LoopAction::Continue
            }
            crate::repl_cli_alias::CliLine::Hint(hint) => {
                println!("{hint}");
                LoopAction::Continue
            }
            crate::repl_cli_alias::CliLine::NotCli => {
                run_query(state, line, config);
                LoopAction::Continue
            }
        }
    }
}

/// Dispatch a dot/backslash command and map its result to a [`LoopAction`].
/// The daily-driver meta commands (`repl_meta_cmds`) get first pick;
/// anything they do not claim goes to the standard dispatch.
fn handle_dot_command(
    state: &mut ReplState,
    rl: &mut ReplEditor,
    line: &str,
    config: &mut ReplConfig,
) -> LoopAction {
    if let Some(result) = crate::repl_meta_cmds::try_handle(state, rl, line, config) {
        return map_result(result);
    }
    map_result(crate::repl_commands::handle_command(
        state.current(),
        line,
        config,
    ))
}

/// Map a [`CommandResult`] to a [`LoopAction`], printing errors in red.
fn map_result(result: crate::repl_commands::CommandResult) -> LoopAction {
    match result {
        crate::repl_commands::CommandResult::Continue => LoopAction::Continue,
        crate::repl_commands::CommandResult::Quit => LoopAction::Quit,
        crate::repl_commands::CommandResult::Error(e) => {
            println!("{} {}", "Error:".red().bold(), e);
            LoopAction::Continue
        }
    }
}

/// Execute a `VelesQL` query line, print its result or error, and record
/// the query (and its result, on success) in the session state for
/// `\edit`, `\watch`, `\save` and `\copy`.
fn run_query(state: &mut ReplState, line: &str, config: &ReplConfig) {
    state.last_query = Some(line.to_string());
    match execute_query(
        state.current(),
        line,
        config.session.active_collection(),
        Some(&config.session),
    ) {
        Ok(result) => {
            let fmt = match config.format {
                OutputFormat::Table => "table",
                OutputFormat::Json => "json",
            };
            // A global `--output` on the command line wins over `.format`.
            let fmt = crate::output::effective_format(fmt);
            print_result(&result, &fmt);
            if config.timing {
                println!(
                    "\n{} rows ({:.2}ms)\n",
                    result.rows.len().to_string().green(),
                    result.duration_ms
                );
            }
            state.last_result = Some(result);
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
