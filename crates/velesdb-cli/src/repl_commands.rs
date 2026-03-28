//! REPL command handlers extracted from repl.rs
//!
//! Each command is implemented in a domain-specific sub-module:
//! - [`crate::repl_collection_cmds`] — collection inspection (.describe, .count, etc.)
//! - [`crate::repl_query_cmds`] — query analysis and index management
//! - [`crate::repl_search_cmds`] — sparse/hybrid search, agent, guardrails
//! - [`crate::repl_graph_cmds`] — graph operations (.graph ...)
//! - [`crate::repl_config_cmds`] — session settings (\set, \show, etc.)
//!
//! This module owns the command dispatch logic and shared utilities.

#![allow(clippy::doc_markdown)]

use velesdb_core::Database;

use crate::repl::ReplConfig;
use crate::repl_collection_cmds;
use crate::repl_config_cmds;
use crate::repl_data_cmds;
use crate::repl_graph_cmds;
use crate::repl_query_cmds;
use crate::repl_search_cmds;

/// Result of a REPL command execution.
pub enum CommandResult {
    Continue,
    Quit,
    Error(String),
}

/// Handle a REPL command (line starting with '.')
#[allow(clippy::too_many_lines, clippy::cognitive_complexity)]
pub fn handle_command(db: &Database, line: &str, config: &mut ReplConfig) -> CommandResult {
    let parts: Vec<&str> = line.split_whitespace().collect();
    let cmd = parts.first().map(|s| s.to_lowercase()).unwrap_or_default();

    match cmd.as_str() {
        ".quit" | ".exit" | ".q" => CommandResult::Quit,
        ".help" | ".h" => {
            print_help();
            CommandResult::Continue
        }
        // Collection inspection commands
        ".collections" | ".tables" => repl_collection_cmds::cmd_collections(db),
        ".schema" => repl_collection_cmds::cmd_schema(db, &parts),
        ".describe" | ".desc" => repl_collection_cmds::cmd_describe(db, &parts),
        ".count" => repl_collection_cmds::cmd_count(db, &parts),
        ".sample" => repl_collection_cmds::cmd_sample(db, &parts),
        ".browse" => repl_collection_cmds::cmd_browse(db, &parts),
        ".stats" => repl_collection_cmds::cmd_stats(db, &parts),
        ".bench" | "\\bench" => repl_data_cmds::cmd_bench(db, config, &parts),
        ".export" => repl_data_cmds::cmd_export(db, &parts),
        ".nodes" => repl_collection_cmds::cmd_nodes(db, &parts),
        // Config / session commands
        ".timing" => repl_config_cmds::cmd_timing(config, &parts),
        ".format" => repl_config_cmds::cmd_format(config, &parts),
        ".clear" => repl_config_cmds::cmd_clear(),
        // Query / index commands
        ".explain" => repl_query_cmds::cmd_explain(db, &parts),
        ".analyze" => repl_query_cmds::cmd_analyze(db, &parts),
        ".indexes" => repl_query_cmds::cmd_indexes(db, &parts),
        ".delete" => repl_query_cmds::cmd_delete(db, &parts),
        ".flush" => repl_query_cmds::cmd_flush(db, &parts),
        ".create-index" => repl_query_cmds::cmd_create_index(db, &parts),
        ".drop-index" => repl_query_cmds::cmd_drop_index(db, &parts),
        // Advanced search commands
        ".sparse-search" => repl_search_cmds::cmd_sparse_search(db, &parts),
        ".hybrid-sparse" => repl_search_cmds::cmd_hybrid_sparse(db, &parts),
        ".agent" => repl_search_cmds::cmd_agent(&parts),
        ".guardrails" => repl_search_cmds::cmd_guardrails(),
        // Graph commands
        ".graph" => repl_graph_cmds::cmd_graph(db, &parts),
        // Session commands (backslash style)
        "\\set" | ".set" => repl_config_cmds::cmd_set(config, &parts),
        "\\show" | ".show" => repl_config_cmds::cmd_show(config, &parts),
        "\\reset" | ".reset" => repl_config_cmds::cmd_reset(config, &parts),
        "\\use" | ".use" => repl_config_cmds::cmd_use(db, config, &parts),
        "\\info" | ".info" => repl_config_cmds::cmd_info(db, config),
        _ => CommandResult::Error(format!("Unknown command: {cmd}")),
    }
}

// ============================================================================
// Shared utilities
// ============================================================================

/// Parse a `--flag value` pair from command parts.
pub(crate) fn parse_flag(parts: &[&str], flag: &str) -> Option<String> {
    parts
        .iter()
        .position(|&p| p == flag)
        .and_then(|i| parts.get(i + 1))
        .map(|s| s.to_string())
}

/// Print help text for REPL commands (delegates to repl_output for the full text).
pub fn print_help() {
    crate::repl_output::print_help();
}
