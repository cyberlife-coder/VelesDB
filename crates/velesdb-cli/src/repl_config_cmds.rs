//! REPL commands for session configuration and REPL settings.
//!
//! Covers: `.timing`, `.format`, `.clear`, `\set`, `\show`, `\reset`,
//! `\use`, `\info`.

use colored::Colorize;
use velesdb_core::Database;

use crate::collection_helpers;
use crate::repl::{OutputFormat, ReplConfig};
use crate::repl_commands::CommandResult;
use crate::session::SessionSettings;

const VERSION: &str = env!("CARGO_PKG_VERSION");

pub(crate) fn cmd_timing(config: &mut ReplConfig, parts: &[&str]) -> CommandResult {
    if parts.len() < 2 {
        println!("Timing is {}", if config.timing { "ON" } else { "OFF" });
    } else {
        match parts[1].to_lowercase().as_str() {
            "on" | "true" | "1" => {
                config.timing = true;
                println!("Timing ON");
            }
            "off" | "false" | "0" => {
                config.timing = false;
                println!("Timing OFF");
            }
            _ => {
                return CommandResult::Error("Use: .timing on|off".to_string());
            }
        }
    }
    println!();
    CommandResult::Continue
}

pub(crate) fn cmd_format(config: &mut ReplConfig, parts: &[&str]) -> CommandResult {
    if parts.len() < 2 {
        println!("Format is {:?}", config.format);
    } else {
        match parts[1].to_lowercase().as_str() {
            "table" => {
                config.format = OutputFormat::Table;
                println!("Format: table");
            }
            "json" => {
                config.format = OutputFormat::Json;
                println!("Format: json");
            }
            _ => {
                return CommandResult::Error("Use: .format table|json".to_string());
            }
        }
    }
    println!();
    CommandResult::Continue
}

pub(crate) fn cmd_clear() -> CommandResult {
    print!("\x1B[2J\x1B[1;1H");
    CommandResult::Continue
}

pub(crate) fn cmd_set(config: &mut ReplConfig, parts: &[&str]) -> CommandResult {
    if parts.len() < 3 {
        println!("Usage: \\set <setting> <value>\n");
        println!("Settings: mode, ef_search, timeout_ms, rerank, max_results\n");
        return CommandResult::Continue;
    }
    let key = parts[1];
    let value = parts[2];
    match config.session.set(key, value) {
        Ok(()) => {
            println!("{} = {}", key.cyan(), value.green());
            warn_if_unwired(key);
            println!();
        }
        Err(e) => return CommandResult::Error(e),
    }
    CommandResult::Continue
}

/// Warns that a stored setting is display-only because it has no channel into
/// `Database::execute_query` yet, so `\set` does not silently claim it applies.
fn warn_if_unwired(key: &str) {
    if crate::session::is_unwired_setting(key) {
        println!(
            "{}",
            format!(
                "Note: '{key}' is recorded but not yet applied to query execution (display-only)."
            )
            .yellow()
        );
    }
}

pub(crate) fn cmd_show(db: &Database, config: &ReplConfig, parts: &[&str]) -> CommandResult {
    let key = parts.get(1).copied();
    let settings = match shown_settings(db, &config.session, key) {
        Ok(settings) => settings,
        Err(message) => return CommandResult::Error(message),
    };
    if key.is_none() {
        println!("\n{}", "Session Settings".bold().underline());
        for (key, value) in settings {
            println!("  {} = {}", key.cyan(), value.green());
        }
        println!();
    } else {
        for (key, value) in settings {
            println!("{} = {}\n", key.cyan(), value.green());
        }
    }
    CommandResult::Continue
}

/// What `\show` prints: every setting, or the one `key` names, each with
/// the value in force. An unset mode shows the default `db` was opened with,
/// marked `(configured default)` (#2303).
pub(crate) fn shown_settings(
    db: &Database,
    session: &SessionSettings,
    key: Option<&str>,
) -> Result<Vec<(String, String)>, String> {
    let configured = db.config().search.resolved_quality();
    match key {
        None => Ok(session.all_settings(configured)),
        Some(key) => session
            .get(key, configured)
            .map(|value| vec![(key.to_string(), value)])
            .ok_or_else(|| format!("Unknown setting: {key}")),
    }
}

pub(crate) fn cmd_reset(config: &mut ReplConfig, parts: &[&str]) -> CommandResult {
    let key = parts.get(1).copied();
    config.session.reset(key);
    if let Some(k) = key {
        println!("Reset {}\n", k.cyan());
    } else {
        println!("All settings reset to defaults\n");
    }
    CommandResult::Continue
}

pub(crate) fn cmd_use(db: &Database, config: &mut ReplConfig, parts: &[&str]) -> CommandResult {
    if parts.len() < 2 {
        match config.session.active_collection() {
            Some(name) => println!("Active collection: {}\n", name.green()),
            None => println!("No active collection. Usage: \\use <collection>\n"),
        }
    } else {
        let name = parts[1];
        if collection_helpers::resolve_collection(db, name).is_some() {
            config.session.use_collection(Some(name.to_string()));
            println!(
                "Using collection: {} [{}]\n",
                name.green(),
                collection_helpers::collection_type_label(db, name).cyan()
            );
        } else {
            return CommandResult::Error(format!("Collection '{name}' not found"));
        }
    }
    CommandResult::Continue
}

pub(crate) fn cmd_info(db: &Database, config: &ReplConfig) -> CommandResult {
    println!("\n{}", "VelesDB Information".bold().underline());
    println!("  {} {}", "Version:".cyan(), VERSION.green());
    println!("  {} {}", "Database:".cyan(), "active".green());

    let collections = db.list_collections();
    println!("  {} {}", "Collections:".cyan(), collections.len());

    let total_points: usize = collections
        .iter()
        .filter_map(|name| db.get_vector_collection(name))
        .map(|col| col.config().point_count)
        .sum();
    println!("  {} {}", "Total Points:".cyan(), total_points);

    if let Some(col_name) = config.session.active_collection() {
        println!("  {} {}", "Active Collection:".cyan(), col_name.green());
    }
    println!();
    CommandResult::Continue
}

#[cfg(test)]
#[path = "repl_config_cmds_tests.rs"]
mod repl_config_cmds_tests;
