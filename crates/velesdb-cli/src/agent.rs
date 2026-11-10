//! Bounded ReAct agent over whitelisted `VelesQL` tools.
//!
//! `.agent <question>` in the REPL, or `velesdb agent <path> <question>`.
//! Reason→Act loop capped at [`MAX_ITERATIONS`]; every iteration is logged
//! into the transcript and the final answer lists the tool outputs it used.
//!
//! LLM backend is optional: when an Ollama instance answers
//! `GET {VELESDB_AGENT_URL}/api/tags` the loop delegates reasoning to it
//! (same extractor-style contract `velesdb-memory` uses: base URL + model).
//! When no LLM is reachable the agent runs a deterministic offline router —
//! keyword intents mapped to the same whitelisted tools — and says so in the
//! transcript. No LLM is ever required.

use velesdb_core::Database;

mod llm;
mod offline;
pub(crate) mod tools;

/// Hard cap on Reason→Act cycles; the loop must terminate.
pub const MAX_ITERATIONS: usize = 8;

/// One logged Reason→Act cycle.
#[derive(Debug, Clone)]
pub struct Iteration {
    /// 1-based iteration number.
    pub n: usize,
    /// The reasoning step (LLM `Thought:` or offline router note).
    pub thought: String,
    /// `tool(args)` actually dispatched.
    pub action: String,
    /// Tool output, or the refusal string for an unknown tool.
    pub observation: String,
}

/// What a run produced: final answer, which backend reasoned, transcript.
#[derive(Debug)]
pub struct AgentReport {
    /// Sourced final answer (or the honest "no answer within cap" note).
    pub answer: String,
    /// `"ollama(<model>@<url>)"` or `"offline"`.
    pub backend: String,
    /// Why an offline run is offline: the probe or mid-loop LLM failure
    /// (unreachable, 404, 500, timeout…). `None` when the LLM answered.
    pub fallback: Option<String>,
    /// Every iteration, in order — the audit trail.
    pub iterations: Vec<Iteration>,
    /// Tool outputs the answer drew on, as `tool(args)` strings.
    pub sources: Vec<String>,
}

/// Run the agent over `db`. Prefers the Ollama backend when reachable,
/// otherwise the deterministic offline router. A reachable-but-broken
/// LLM (model not pulled, dropped connection) also falls back to the
/// router — the run never fails because of the LLM.
pub fn run(db: &Database, question: &str) -> AgentReport {
    let outcome = llm::backend()
        .and_then(|b| llm::run_loop(db, question, &b).map_err(|why| format!("llm failed: {why}")));
    outcome.unwrap_or_else(|why| {
        let mut report = offline::run(db, question);
        report.fallback = Some(why);
        report
    })
}

/// Render a report for the terminal (transcript + sourced answer).
pub fn print_report(report: &AgentReport) {
    println!("\n{}", "Agent (ReAct)".bold().underline());
    println!("  {} {}", "Backend:".cyan(), report.backend);
    if let Some(why) = &report.fallback {
        println!(
            "  {} answered by the deterministic offline router — {}",
            "notice:".yellow(),
            why
        );
    }
    println!();
    for it in &report.iterations {
        println!("  {}", format!("── iteration {}", it.n).dimmed());
        println!("    {} {}", "thought:".dimmed(), it.thought.dimmed());
        println!("    {} {}", "action:".yellow(), it.action);
        println!(
            "    {} {}",
            "observation:".dimmed(),
            it.observation.dimmed()
        );
    }
    println!("\n{} {}", "Answer:".green().bold(), report.answer);
    if !report.sources.is_empty() {
        println!(
            "  {} {}",
            "sources:".dimmed(),
            report.sources.join(" · ").dimmed()
        );
    }
    println!();
}

/// Render a report as JSON for `--json` / scriptability.
pub fn report_json(report: &AgentReport) -> serde_json::Value {
    serde_json::json!({
        "answer": report.answer,
        "backend": report.backend,
        "fallback": report.fallback,
        "sources": report.sources,
        "iterations": report.iterations.iter().map(|it| serde_json::json!({
            "n": it.n,
            "thought": it.thought,
            "action": it.action,
            "observation": it.observation,
        })).collect::<Vec<_>>(),
    })
}

use colored::Colorize;

#[cfg(test)]
#[path = "agent_tests.rs"]
mod agent_tests;
