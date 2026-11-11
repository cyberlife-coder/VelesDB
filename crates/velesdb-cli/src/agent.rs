//! Bounded ReAct agent over whitelisted `VelesQL` tools.
//!
//! `.agent <question>` in the REPL, or `velesdb agent <path> <question>`.
//! Reason→Act loop capped by [`MaxIterations`] (default [`MAX_ITERATIONS`]);
//! every iteration is logged into the transcript and the final answer lists
//! the tool outputs it used.
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

/// Default cap on Reason→Act cycles; the loop must terminate.
pub const MAX_ITERATIONS: usize = 8;

/// A validated Reason→Act cycle cap, `1..=`[`MaxIterations::LIMIT`].
/// Only [`FromStr`](std::str::FromStr) and [`Default`] build one, so every
/// loop bound the agent sees has been range-checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaxIterations(usize);

impl MaxIterations {
    /// Upper bound: each LLM round re-sends the growing transcript with a
    /// 60 s budget, so 32 rounds already means a worst case of 32 minutes.
    pub const LIMIT: usize = 32;

    /// The cap as a loop bound.
    #[must_use]
    pub fn get(self) -> usize {
        self.0
    }
}

impl Default for MaxIterations {
    fn default() -> Self {
        Self(MAX_ITERATIONS)
    }
}

impl std::str::FromStr for MaxIterations {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().parse::<usize>() {
            Ok(n) if (1..=Self::LIMIT).contains(&n) => Ok(Self(n)),
            _ => Err(format!(
                "max-iterations must be an integer in 1..={}, got '{s}'",
                Self::LIMIT
            )),
        }
    }
}

impl std::fmt::Display for MaxIterations {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Per-REPL-session agent state: the cap `\agent max-iterations` set and
/// the last report, which `\agent save` exports.
#[derive(Debug, Clone, Default)]
pub struct AgentSession {
    /// Cap applied to the next `\agent <question>`.
    pub max_iterations: MaxIterations,
    /// The most recent run, if any.
    pub last: Option<AgentReport>,
}

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
#[derive(Debug, Clone)]
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
pub fn run(db: &Database, question: &str, max: MaxIterations) -> AgentReport {
    let outcome = llm::backend().and_then(|b| {
        llm::run_loop(db, question, &b, max).map_err(|why| format!("llm failed: {why}"))
    });
    outcome.unwrap_or_else(|why| {
        let mut report = offline::run(db, question, max);
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

/// Write the full transcript ([`report_json`], pretty-printed) to `path`.
pub fn save_report(report: &AgentReport, path: &std::path::Path) -> std::io::Result<()> {
    let text = serde_json::to_string_pretty(&report_json(report))?;
    std::fs::write(path, text + "\n")
}

use colored::Colorize;

#[cfg(test)]
#[path = "agent_tests.rs"]
mod agent_tests;
