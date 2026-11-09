//! Optional Ollama backend — same extractor-style contract `velesdb-memory`
//! documents (base URL + model, env-configured), probed with a short timeout.
//! Absent or unreachable means the caller falls back to the offline router;
//! the agent never *requires* an LLM.

use super::{tools, AgentReport, Iteration, MAX_ITERATIONS};
use velesdb_core::Database;

/// Resolved backend endpoint.
pub struct Backend {
    /// Base URL, e.g. `http://127.0.0.1:11434`.
    pub url: String,
    /// Model tag, e.g. `qwen3:4b`.
    pub model: String,
}

/// `VELESDB_AGENT_URL`/`VELESDB_AGENT_MODEL` → probe `{url}/api/tags`
/// (400 ms timeout). `None` = offline mode.
pub fn backend() -> Option<Backend> {
    let url =
        std::env::var("VELESDB_AGENT_URL").unwrap_or_else(|_| "http://127.0.0.1:11434".to_string());
    let model = std::env::var("VELESDB_AGENT_MODEL").unwrap_or_else(|_| "qwen3:4b".to_string());
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_millis(400))
        .build();
    agent
        .get(&format!("{url}/api/tags"))
        .call()
        .ok()
        .map(|_| Backend { url, model })
}

/// Classic ReAct prompt: tools contract + strict line format.
fn system_prompt(question: &str) -> String {
    format!(
        "You are a VelesDB agent. Answer the question using ONLY these tools.\n\
         Tools:\n{}\n\n\
         Format each reply EXACTLY as one of:\n\
         Thought: <why>\nAction: <tool>(<args>)\n\
         or\n\
         Final Answer: <answer>\n\n\
         Rules: max {} rounds; never invent data; cite the observations.\n\n\
         Question: {}",
        tools::TOOL_DOCS,
        MAX_ITERATIONS,
        question
    )
}

/// Run the bounded Reason→Act loop against `backend`.
pub fn run_loop(db: &Database, question: &str, b: &Backend) -> AgentReport {
    let mut transcript = String::new();
    let mut iterations = Vec::new();
    let mut sources = Vec::new();
    for n in 1..=MAX_ITERATIONS {
        let step = match step(b, &system_prompt(question), &transcript) {
            Ok(s) => s,
            Err(e) => {
                let remedy = if e.contains("404") {
                    format!(
                        " — model '{}' not pulled? try `ollama pull {}`",
                        b.model, b.model
                    )
                } else {
                    String::new()
                };
                return finish(iterations, format!("LLM error: {e}{remedy}"), sources, b);
            }
        };
        if let Some(ans) = parse_final(&step) {
            return finish(iterations, ans, sources, b);
        }
        let (thought, action) = parse_action(&step);
        let (name, args) = split_call(&action);
        let obs = tools::call(db, name, args).unwrap_or_else(|o| o);
        let call_str = format!("{name}({args})");
        sources.push(call_str.clone());
        transcript.push_str(&format!(
            "Thought: {thought}\nAction: {call_str}\nObservation: {obs}\n"
        ));
        iterations.push(Iteration {
            n,
            thought,
            action: call_str,
            observation: obs,
        });
    }
    finish(
        iterations,
        format!("no final answer within {MAX_ITERATIONS} iterations"),
        sources,
        b,
    )
}

/// One LLM round-trip: prompt + transcript → model text (60 s budget).
fn step(b: &Backend, system: &str, transcript: &str) -> Result<String, String> {
    let body = serde_json::json!({
        "model": b.model,
        "prompt": format!("{system}\n\n{transcript}"),
        "stream": false,
        "options": {"temperature": 0.0, "num_predict": 512},
    });
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(60))
        .build();
    let resp = agent
        .post(&format!("{}/api/generate", b.url))
        .send_json(&body)
        .map_err(|e| format!("{e}"))?;
    let v: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("bad ollama response: {e}"))?;
    v.get("response")
        .and_then(|r| r.as_str())
        .map(str::to_string)
        .ok_or_else(|| "ollama response had no 'response' field".to_string())
}

fn parse_final(step: &str) -> Option<String> {
    step.lines()
        .find(|l| l.starts_with("Final Answer:"))
        .map(|l| l.trim_start_matches("Final Answer:").trim().to_string())
}

fn parse_action(step: &str) -> (String, String) {
    let thought = step
        .lines()
        .find(|l| l.starts_with("Thought:"))
        .map_or_else(|| "…".to_string(), |l| l[8..].trim().to_string());
    let action = step
        .lines()
        .find(|l| l.starts_with("Action:"))
        .map_or_else(|| step.trim().to_string(), |l| l[7..].trim().to_string());
    (thought, action)
}

/// `name(args)` → ("name", "args"); bare `name` → ("name", "").
fn split_call(action: &str) -> (&str, &str) {
    let action = action.trim();
    match action.find('(') {
        Some(p) => (
            action[..p].trim(),
            action[p + 1..].trim_end_matches(')').trim(),
        ),
        None => (action, ""),
    }
}

fn finish(
    iterations: Vec<Iteration>,
    answer: String,
    mut sources: Vec<String>,
    b: &Backend,
) -> AgentReport {
    sources.sort();
    sources.dedup();
    AgentReport {
        answer,
        backend: format!("ollama({}@{})", b.model, b.url),
        iterations,
        sources,
    }
}
