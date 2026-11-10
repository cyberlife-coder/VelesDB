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
/// (400 ms timeout). `None` = offline mode. A reachable server whose
/// model list parses but lacks the model returns `None` too — otherwise
/// `/api/generate` just 404s mid-loop.
pub fn backend() -> Option<Backend> {
    let url =
        std::env::var("VELESDB_AGENT_URL").unwrap_or_else(|_| "http://127.0.0.1:11434".to_string());
    let model = std::env::var("VELESDB_AGENT_MODEL").unwrap_or_else(|_| "qwen3:4b".to_string());
    let probe = format!("{url}/api/tags");
    let tags = runtime()?.block_on(async move {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(400))
            .build()
            .ok()?
            .get(&probe)
            .send()
            .await
            .ok()?
            .json::<serde_json::Value>()
            .await
            .ok()
    })?;
    // Unknown response shapes don't block the backend — a mid-loop
    // transport failure falls back to offline anyway.
    match tags.get("models").and_then(|m| m.as_array()) {
        Some(models) if !model_pulled(models, &model) => None,
        _ => Some(Backend { url, model }),
    }
}

/// `/api/tags` entries carry the tag (`"qwen3:14b"`); an untagged model
/// name means `<model>:latest` to Ollama.
pub(super) fn model_pulled(models: &[serde_json::Value], model: &str) -> bool {
    models
        .iter()
        .filter_map(|m| m.get("name").and_then(|n| n.as_str()))
        .any(|n| n == model || n == format!("{model}:latest"))
}

/// One-shot current-thread runtime — same pattern as the update check's
/// detached thread: the CLI is sync, each call owns its runtime.
fn runtime() -> Option<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()
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

/// Run the bounded Reason→Act loop against `backend`. `Err` means the
/// transport failed mid-loop (404, reset, timeout) — the caller then
/// falls back to the offline router rather than reporting an error.
pub fn run_loop(db: &Database, question: &str, b: &Backend) -> Result<AgentReport, String> {
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
                return Err(format!("{e}{remedy}"));
            }
        };
        if let Some(ans) = parse_final(&step) {
            return Ok(finish(iterations, ans, sources, b));
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
    Ok(finish(
        iterations,
        format!("no final answer within {MAX_ITERATIONS} iterations"),
        sources,
        b,
    ))
}

/// One LLM round-trip: prompt + transcript → model text (60 s budget).
fn step(b: &Backend, system: &str, transcript: &str) -> Result<String, String> {
    let body = serde_json::json!({
        "model": b.model,
        "prompt": format!("{system}\n\n{transcript}"),
        "stream": false,
        "options": {"temperature": 0.0, "num_predict": 512},
    });
    let url = format!("{}/api/generate", b.url);
    let rt = runtime().ok_or_else(|| "agent runtime unavailable".to_string())?;
    rt.block_on(async move {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|e| format!("{e}"))?;
        let resp = client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("{url}: {e}"))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(format!("{url}: status code {status}"));
        }
        let v: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("bad ollama response: {e}"))?;
        v.get("response")
            .and_then(|r| r.as_str())
            .map(str::to_string)
            .ok_or_else(|| "ollama response had no 'response' field".to_string())
    })
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
