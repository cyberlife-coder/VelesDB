//! Global output options for the CLI: `--output`, `--quiet`, `--no-color`,
//! plus the process exit-code mapping documented in
//! `docs/guides/CLI_COMMAND_REFERENCE.md`.
//!
//! The options are recorded once at startup in a [`OnceLock`] (mirroring
//! `helpers::CONFIG_PATH`) and are read-only afterwards, so no lock is needed
//! on the read path.

use clap::ValueEnum;
use std::sync::OnceLock;

/// Global `--output` format, honored by every command that prints data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum GlobalFormat {
    /// Human-readable table (default).
    Table,
    /// Pretty-printed JSON document.
    Json,
    /// One compact JSON value per line (JSON Lines).
    Jsonl,
    /// Comma-separated values (array of objects only).
    Csv,
}

impl GlobalFormat {
    /// Canonical lowercase name, as accepted on the command line.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Table => "table",
            Self::Json => "json",
            Self::Jsonl => "jsonl",
            Self::Csv => "csv",
        }
    }
}

/// Global output options recorded once at startup.
#[derive(Debug, Clone, Copy, Default)]
pub struct OutputOptions {
    /// Global `--output`; overrides any per-command `--format` when set.
    pub format: Option<GlobalFormat>,
    /// `--quiet`: suppress human-readable status output (data still prints).
    pub quiet: bool,
    /// `--no-color`: disable colored output (same as `NO_COLOR=1`).
    pub no_color: bool,
}

static OPTIONS: OnceLock<OutputOptions> = OnceLock::new();

/// Record the global output options. Called once from `cli_main` before any
/// dispatch; applies the color override immediately.
pub fn init(options: OutputOptions) {
    if options.no_color {
        colored::control::set_override(false);
    }
    let _ = OPTIONS.set(options);
}

/// Whether `--quiet` was passed.
pub fn quiet() -> bool {
    OPTIONS.get().is_some_and(|o| o.quiet)
}

/// Resolve the effective format: the global `--output` when set, else the
/// per-command `--format` fallback.
pub fn resolve_format(global: Option<GlobalFormat>, fallback: &str) -> String {
    global.map_or_else(|| fallback.to_string(), |g| g.as_str().to_string())
}

/// Effective format for a command whose own `--format` is `fallback`.
pub fn effective_format(fallback: &str) -> String {
    resolve_format(OPTIONS.get().and_then(|o| o.format), fallback)
}

/// Whether `format` is a machine-readable format (json, jsonl, csv).
pub fn is_machine(format: &str) -> bool {
    matches!(
        format.to_ascii_lowercase().as_str(),
        "json" | "jsonl" | "csv"
    )
}

/// Serialize `value` as lines for a machine format. Pure, so tests can pin it.
pub fn machine_lines(value: &serde_json::Value, format: &str) -> Result<Vec<String>, String> {
    match format.to_ascii_lowercase().as_str() {
        "json" => serde_json::to_string_pretty(value)
            .map(|s| vec![s])
            .map_err(|e| e.to_string()),
        "jsonl" => jsonl_lines(value),
        "csv" => csv_lines(value),
        other => Err(format!("unsupported machine format: {other}")),
    }
}

/// Print `value` in a machine format to stdout.
pub fn print_machine<T: serde::Serialize>(value: &T, format: &str) -> anyhow::Result<()> {
    let value = serde_json::to_value(value)?;
    for line in machine_lines(&value, format).map_err(anyhow::Error::msg)? {
        println!("{line}");
    }
    Ok(())
}

/// JSON Lines: one compact line per array element, or a single line otherwise.
fn jsonl_lines(value: &serde_json::Value) -> Result<Vec<String>, String> {
    match value {
        serde_json::Value::Array(items) => items
            .iter()
            .map(|item| serde_json::to_string(item).map_err(|e| e.to_string()))
            .collect(),
        other => Ok(vec![
            serde_json::to_string(other).map_err(|e| e.to_string())?
        ]),
    }
}

/// CSV: header from the union of object keys (first-seen order), then one row
/// per array element. Cells are JSON-encoded (strings stay quoted).
fn csv_lines(value: &serde_json::Value) -> Result<Vec<String>, String> {
    let rows = match value {
        serde_json::Value::Array(items) => items.clone(),
        other => vec![other.clone()],
    };
    let mut header: Vec<String> = Vec::new();
    for row in &rows {
        if let Some(obj) = row.as_object() {
            for key in obj.keys() {
                if !header.contains(key) {
                    header.push(key.clone());
                }
            }
        }
    }
    if header.is_empty() {
        // No object rows: emit one raw line per value, no header.
        return rows
            .iter()
            .map(|r| serde_json::to_string(r).map_err(|e| e.to_string()))
            .collect();
    }
    let mut lines = vec![header.join(",")];
    for row in &rows {
        let cells: Vec<String> = header
            .iter()
            .map(|k| {
                serde_json::to_string(row.get(k).unwrap_or(&serde_json::Value::Null))
                    .unwrap_or_else(|_| "null".to_string())
            })
            .collect();
        lines.push(cells.join(","));
    }
    Ok(lines)
}

/// Process exit code for a failed run. `2` (usage) is produced by clap itself
/// before `main` runs; `0` is success. See the exit-code table in
/// `docs/guides/CLI_COMMAND_REFERENCE.md`.
pub fn exit_code(err: &anyhow::Error) -> i32 {
    // Walk the chain so `.context(...)` wrappers keep the typed cause.
    for cause in err.chain() {
        if let Some(core_err) = cause.downcast_ref::<velesdb_core::Error>() {
            return core_exit_code(core_err);
        }
        if cause.downcast_ref::<std::io::Error>().is_some() {
            return 4;
        }
    }
    1
}

/// Map a core error to its exit code: not-found 3, io 4, parse 5, else 1.
fn core_exit_code(err: &velesdb_core::Error) -> i32 {
    use velesdb_core::Error;
    match err {
        Error::CollectionNotFound(_)
        | Error::PointNotFound(_)
        | Error::EdgeNotFound(_)
        | Error::NodeNotFound(_) => 3,
        Error::Io(_) => 4,
        Error::Query(_) => 5,
        _ => 1,
    }
}

/// Print a human-readable status line unless `--quiet` was passed.
///
/// Data output (tables, machine formats) must keep using `println!` directly;
/// only status messages go through this macro.
#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => {
        if !$crate::output::quiet() {
            println!($($arg)*);
        }
    };
}

#[cfg(test)]
#[path = "output_tests.rs"]
mod output_tests;
