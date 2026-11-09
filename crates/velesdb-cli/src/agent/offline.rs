//! Deterministic offline backend — no LLM, ever. Keyword intents map to the
//! same whitelisted tools; the composed answer cites each tool output it used.
//! This mode is documented as such: it answers structural questions about the
//! data honestly, and says when the question is outside what its router can
//! ground.

use super::{tools, AgentReport, Iteration};
use velesdb_core::Database;

struct Step {
    thought: String,
    tool: String,
    args: String,
}

/// Run the deterministic plan: `list` grounds the run, then at most two
/// intent-driven tool calls, then a composed answer. Well under the cap.
pub fn run(db: &Database, question: &str) -> AgentReport {
    let mut iterations = Vec::new();
    let mut sources = Vec::new();

    let names = db.list_collections();
    act(
        db,
        &mut iterations,
        &mut sources,
        Step {
            thought: "ground the run: enumerate collections".to_string(),
            tool: "list".to_string(),
            args: String::new(),
        },
    );

    for step in plan(&names, question) {
        act(db, &mut iterations, &mut sources, step);
        if iterations.len() >= super::MAX_ITERATIONS - 1 {
            break;
        }
    }
    AgentReport {
        answer: compose(&iterations, question),
        backend: "offline".to_string(),
        iterations,
        sources,
    }
}

/// Execute one step, log the iteration, remember the source.
fn act(db: &Database, iterations: &mut Vec<Iteration>, sources: &mut Vec<String>, step: Step) {
    let obs = tools::call(db, &step.tool, &step.args).unwrap_or_else(|o| o);
    sources.push(format!("{}({})", step.tool, step.args));
    iterations.push(Iteration {
        n: iterations.len() + 1,
        thought: step.thought,
        action: format!("{}({})", step.tool, step.args),
        observation: obs,
    });
}

/// Map the question to ≤2 intent steps using real collection names.
fn plan(names: &[String], q: &str) -> Vec<Step> {
    let ql = q.to_lowercase();
    let coll = names
        .iter()
        .find(|n| ql.contains(&n.to_lowercase()))
        .cloned()
        .or_else(|| names.first().cloned());
    let digits = first_u64(&ql);

    if mentions(&ql, &["graph", "traverse", "linked", "edge", "node"]) {
        return vec![graph_step(names, digits)];
    }
    if mentions(&ql, &["count", "how many", "aggregate", "group by", "per "]) {
        return vec![aggregate_step(coll, &ql)];
    }
    if mentions(
        &ql,
        &["describe", "schema", "structure", "columns", "fields"],
    ) {
        return coll.map_or_else(Vec::new, |c| {
            vec![step("describe", &c, "inspect the named collection")]
        });
    }
    if mentions(&ql, &["search", "find", "about", "similar", "match"]) {
        return vec![search_step(coll, &ql)];
    }
    // Default: describe up to two collections so the answer has real content.
    names
        .iter()
        .take(2)
        .map(|c| step("describe", c, "default: describe available collections"))
        .collect()
}

fn graph_step(names: &[String], digits: Option<u64>) -> Step {
    let g = names
        .iter()
        .find(|n| n.contains("graph") || n.contains("kg") || n.contains("authors"))
        .cloned()
        .unwrap_or_else(|| names.first().cloned().unwrap_or_default());
    Step {
        thought: format!("graph intent → BFS '{g}' from {}", digits.unwrap_or(1)),
        tool: "graph".to_string(),
        args: format!("{g} {}", digits.unwrap_or(1)),
    }
}

fn aggregate_step(coll: Option<String>, ql: &str) -> Step {
    let field = ["category", "kind", "type", "status", "price"]
        .iter()
        .find(|f| ql.contains(**f))
        .copied()
        .unwrap_or("category");
    Step {
        thought: format!("aggregate intent → GROUP BY {field}"),
        tool: "aggregate".to_string(),
        args: format!("{} {field}", coll.unwrap_or_default()),
    }
}

fn search_step(coll: Option<String>, ql: &str) -> Step {
    let term = ql
        .split_whitespace()
        .filter(|w| w.len() > 3 && !STOPWORDS.contains(w))
        .next_back()
        .unwrap_or("docs")
        .to_string();
    Step {
        thought: format!("search intent → MATCH '{term}' on content"),
        tool: "search".to_string(),
        args: format!("{} content {term}", coll.unwrap_or_default()),
    }
}

fn step(tool: &str, args: &str, thought: &str) -> Step {
    Step {
        thought: thought.to_string(),
        tool: tool.to_string(),
        args: args.to_string(),
    }
}

fn mentions(q: &str, words: &[&str]) -> bool {
    words.iter().any(|w| q.contains(w))
}

fn first_u64(q: &str) -> Option<u64> {
    q.split(|c: char| !c.is_ascii_digit())
        .find(|s| !s.is_empty())
        .and_then(|s| s.parse().ok())
}

/// Compose the final answer: embed each observation verbatim so the answer
/// is only ever what the tools actually returned.
fn compose(iterations: &[Iteration], question: &str) -> String {
    if iterations.len() <= 1 {
        return format!(
            "offline mode: '{question}' did not match a known intent and there is no data \
             to describe — ask about collections / describe <c> / search / aggregate / graph."
        );
    }
    let mut ans = String::from("offline mode — what the tools saw:\n");
    for it in iterations.iter().skip(1) {
        ans.push_str(&format!("• {}\n", it.observation));
    }
    if iterations.len() == 1 {
        ans.push_str(&iterations[0].observation);
    }
    ans
}

const STOPWORDS: &[&str] = &[
    "about",
    "find",
    "search",
    "show",
    "what",
    "which",
    "with",
    "from",
    "docs",
    "collection",
    "documents",
    "matching",
    "similar",
    "that",
    "have",
    "does",
];
