//! REPL `.agent` / `\agent` — the bounded ReAct agent and its session
//! controls.
//!
//! - `\agent <question>` — run the agent (cap: the session's max-iterations)
//! - `\agent tools` — list the whitelisted tools and their contracts
//! - `\agent max-iterations [N]` — show, or set for this session, the cap
//! - `\agent save <file>` — write the last run's transcript as JSON
//!
//! The three keywords are reserved only in exactly those shapes; any other
//! word sequence is a question.

use std::path::Path;

use colored::Colorize;
use velesdb_core::Database;

use crate::agent::{self, MaxIterations};
use crate::repl::{OutputFormat, ReplConfig};
use crate::repl_commands::CommandResult;

/// Dispatch `.agent …` on its arguments.
pub(crate) fn cmd_agent(db: &Database, config: &mut ReplConfig, parts: &[&str]) -> CommandResult {
    match parts.get(1..).unwrap_or_default() {
        [] => print_help(config.agent.max_iterations),
        ["tools"] => print_tools(),
        ["max-iterations"] => {
            println!("  max-iterations = {}", config.agent.max_iterations);
            CommandResult::Continue
        }
        ["max-iterations", n] => set_max_iterations(config, n),
        ["save", path] => save_last(config, Path::new(path)),
        words => ask(db, config, &words.join(" ")),
    }
}

fn print_help(max: MaxIterations) -> CommandResult {
    println!(
        "\n{}\n\n  {}  bounded ReAct loop over whitelisted VelesQL tools\n  \
         {}  list the tools\n  {}  show / set the cap (now {}, 1..={})\n  \
         {}  write the last transcript as JSON\n\n  \
         Ollama backend when reachable, else the deterministic offline router.\n  \
         Agent memory management lives in the {} or {}.\n",
        "Agent (ReAct)".bold().underline(),
        ".agent <question>".yellow(),
        ".agent tools".yellow(),
        ".agent max-iterations [N]".yellow(),
        max,
        MaxIterations::LIMIT,
        ".agent save <file>".yellow(),
        "Python/TypeScript SDK".cyan(),
        "REST API".cyan(),
    );
    CommandResult::Continue
}

fn print_tools() -> CommandResult {
    println!("\n{}\n", "Agent tools (whitelist)".bold().underline());
    for line in agent::tools::TOOL_DOCS.lines() {
        println!("  {line}");
    }
    println!();
    CommandResult::Continue
}

fn set_max_iterations(config: &mut ReplConfig, n: &str) -> CommandResult {
    match n.parse::<MaxIterations>() {
        Ok(max) => {
            config.agent.max_iterations = max;
            println!("  max-iterations = {max}");
            CommandResult::Continue
        }
        Err(e) => CommandResult::Error(e),
    }
}

fn save_last(config: &ReplConfig, path: &Path) -> CommandResult {
    let Some(report) = &config.agent.last else {
        return CommandResult::Error(
            "no agent transcript yet — run `.agent <question>` first".to_string(),
        );
    };
    match agent::save_report(report, path) {
        Ok(()) => {
            println!("  transcript saved to {}", path.display());
            CommandResult::Continue
        }
        Err(e) => CommandResult::Error(format!("cannot write {}: {e}", path.display())),
    }
}

fn ask(db: &Database, config: &mut ReplConfig, question: &str) -> CommandResult {
    let report = agent::run(db, question, config.agent.max_iterations);
    if matches!(config.format, OutputFormat::Json) {
        println!("{}", agent::report_json(&report));
    } else {
        agent::print_report(&report);
    }
    config.agent.last = Some(report);
    CommandResult::Continue
}

#[cfg(test)]
#[path = "repl_agent_cmds_tests.rs"]
mod repl_agent_cmds_tests;
