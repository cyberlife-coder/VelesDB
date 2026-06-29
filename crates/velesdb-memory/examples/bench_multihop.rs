//! Reproducible benchmark: the graph's contribution to multi-hop answer recall.
//!
//! Run it (offline, deterministic by default):
//!
//! ```text
//! cargo run --release -p velesdb-memory --example bench_multihop
//! # real semantic embedder (genuine scores), still local:
//! cargo build --release -p velesdb-memory --features ollama && ollama pull all-minilm
//! VELESDB_MEMORY_EMBEDDER=ollama cargo run --release -p velesdb-memory --features ollama --example bench_multihop
//! ```
//!
//! ## Method
//!
//! One corpus of realistic `decision → PR → problem` chains: each decision
//! adopted some tech to fix an original problem, linked through the PR. The
//! **same** embedder is used throughout — only the graph is toggled — so any
//! gap is the graph's doing.
//!
//! Two question types per chain, both asked as `"why did we adopt <tech> for
//! <topic>"`:
//! - **direct** — the answer is the decision itself (shares words). A control:
//!   vector recall should ace this.
//! - **multi-hop** — the answer is the *problem it fixed*, two hops away, whose
//!   wording shares nothing with the question. Pure similarity can't reach it;
//!   `why()` follows the links.
//!
//! This isolates the graph effect on controlled data. It is *not* `LoCoMo`:
//! that apples-to-apples comparison (vs mem0/Zep) tests an end-to-end
//! extraction pipeline, which this server intentionally does not ship — see the
//! crate README and PLAN 2B.

use velesdb_memory::DynEmbedder;
use velesdb_memory::{HashEmbedder, MemoryError, MemoryService, DEFAULT_DIMENSION};

const RECALL_K: usize = 10;
const MAX_HOPS: usize = 2;

/// `(topic, tech, the original problem it fixed)` — realistic, distinct chains.
const SCENARIOS: &[(&str, &str, &str)] = &[
    (
        "session caching",
        "Redis",
        "users were silently logged out on every deploy",
    ),
    (
        "event storage",
        "Postgres partitioning",
        "the analytics dashboard kept timing out",
    ),
    (
        "the message bus",
        "NATS",
        "orders were processed twice under load",
    ),
    (
        "image delivery",
        "a CDN",
        "mobile users saw blank product photos",
    ),
    (
        "the search index",
        "OpenSearch",
        "small typos returned zero results",
    ),
    (
        "rate limiting",
        "a token bucket",
        "one client could exhaust the whole API",
    ),
    (
        "background jobs",
        "a worker queue",
        "confirmation emails arrived hours late",
    ),
    (
        "the lock primitive",
        "parking_lot",
        "a panic poisoned every mutex",
    ),
    (
        "config rollout",
        "feature flags",
        "a bad change hit all users at once",
    ),
    (
        "the build cache",
        "sccache",
        "CI took twenty minutes per run",
    ),
    (
        "payment retries",
        "exponential backoff",
        "customers saw duplicate charges in outages",
    ),
    (
        "schema changes",
        "online migrations",
        "deploys locked the orders table",
    ),
    (
        "auth tokens",
        "short-lived sessions",
        "a leaked token stayed valid for weeks",
    ),
    (
        "the API gateway",
        "Envoy",
        "one slow service stalled every request",
    ),
    (
        "data export",
        "streaming responses",
        "large exports ran the server out of memory",
    ),
    (
        "password storage",
        "Argon2",
        "a database dump exposed weak hashes",
    ),
    (
        "the pricing cache",
        "write-through caching",
        "stale prices showed after updates",
    ),
    (
        "log shipping",
        "structured logging",
        "incident triage dragged on for hours",
    ),
    (
        "the test database",
        "ephemeral containers",
        "flaky tests from shared state",
    ),
    (
        "file uploads",
        "presigned URLs",
        "uploads bottlenecked the app server",
    ),
    (
        "the metrics store",
        "Prometheus",
        "we were blind during the last outage",
    ),
    (
        "the retry policy",
        "idempotency keys",
        "refunds were sometimes issued twice",
    ),
    (
        "dependency pinning",
        "a lockfile",
        "a transitive update broke production",
    ),
    (
        "read scaling",
        "a follower replica",
        "reports hammered the primary database",
    ),
];

/// One chain plus the question that probes it.
struct Chain {
    question: String,
    decision: u64,
    ticket: u64,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join(format!("velesdb-bench-{}", std::process::id()));
    let svc = MemoryService::open(&dir, embedder()?)?;
    let chains = build_corpus(&svc)?;
    let total = u32::try_from(chains.len()).unwrap_or(0);

    let mut direct_vector: u32 = 0; // vector recall finds the (1-hop) decision
    let mut multihop_vector: u32 = 0; // vector recall finds the (2-hop) problem
    let mut multihop_graph: u32 = 0; // why() finds the (2-hop) problem
    for chain in &chains {
        let hits = svc.recall(&chain.question, RECALL_K, None)?;
        if hits.iter().any(|hit| hit.id == chain.decision) {
            direct_vector += 1;
        }
        if hits.iter().any(|hit| hit.id == chain.ticket) {
            multihop_vector += 1;
        }
        if why_finds(&svc, &chain.question, chain.ticket)? {
            multihop_graph += 1;
        }
    }

    report(total, direct_vector, multihop_vector, multihop_graph);
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

/// Deterministic `HashEmbedder` by default; a real on-device model via Ollama
/// when built `--features ollama` with `VELESDB_MEMORY_EMBEDDER=ollama`.
// Without the `ollama` feature this can't fail; the `Result` is for the on path.
#[allow(clippy::unnecessary_wraps)]
fn embedder() -> Result<DynEmbedder, Box<dyn std::error::Error>> {
    #[cfg(feature = "ollama")]
    if std::env::var("VELESDB_MEMORY_EMBEDDER").as_deref() == Ok("ollama") {
        use velesdb_memory::{OllamaEmbedder, DEFAULT_OLLAMA_MODEL, DEFAULT_OLLAMA_URL};
        return Ok(Box::new(OllamaEmbedder::new(
            DEFAULT_OLLAMA_URL,
            DEFAULT_OLLAMA_MODEL,
        )?));
    }
    Ok(Box::new(HashEmbedder::new(DEFAULT_DIMENSION)))
}

/// Human label for the active embedder, for the report header.
fn embedder_label() -> &'static str {
    #[cfg(feature = "ollama")]
    if std::env::var("VELESDB_MEMORY_EMBEDDER").as_deref() == Ok("ollama") {
        return "ollama / all-minilm (real semantic)";
    }
    "hash (deterministic, offline)"
}

/// Seed every scenario as a `decision → PR → problem` chain.
fn build_corpus(svc: &MemoryService<DynEmbedder>) -> Result<Vec<Chain>, MemoryError> {
    let mut chains = Vec::with_capacity(SCENARIOS.len());
    for &(topic, tech, problem) in SCENARIOS {
        let decision = svc.remember(&format!("we adopted {tech} for {topic}"), &[], None)?;
        let pr = svc.remember(&format!("PR — {tech} for {topic}"), &[], None)?;
        let ticket = svc.remember(problem, &[], None)?;
        svc.relate(decision, pr, "decided_in")?;
        svc.relate(pr, ticket, "fixes")?;
        chains.push(Chain {
            question: format!("why did we adopt {tech} for {topic}"),
            decision,
            ticket,
        });
    }
    Ok(chains)
}

/// Does `why()` (vector seed + graph traversal) surface memory `target`?
fn why_finds(
    svc: &MemoryService<DynEmbedder>,
    question: &str,
    target: u64,
) -> Result<bool, MemoryError> {
    Ok(svc
        .why(question, MAX_HOPS, None)?
        .nodes
        .iter()
        .any(|node| node.id == target))
}

/// Print the result table.
fn report(total: u32, direct_vector: u32, multihop_vector: u32, multihop_graph: u32) {
    let pct = |n: u32| 100.0 * f64::from(n) / f64::from(total);
    println!("VelesDB-memory — graph contribution to multi-hop answer recall");
    println!(
        "embedder: {}   ·   {total} linked chains (decision → PR → problem)\n",
        embedder_label()
    );
    println!("  question type   answer             vector-only      vector + graph");
    println!(
        "  direct          the decision      {direct_vector:>3}/{total} ({:>3.0}%)      {total:>3}/{total} (100%)",
        pct(direct_vector)
    );
    println!(
        "  multi-hop       the 2-hop problem {multihop_vector:>3}/{total} ({:>3.0}%)      {multihop_graph:>3}/{total} ({:>3.0}%)",
        pct(multihop_vector),
        pct(multihop_graph)
    );
    println!(
        "\n→ graph contribution on multi-hop: +{:.0} pp (vector {:.0}% → graph {:.0}%).",
        pct(multihop_graph) - pct(multihop_vector),
        pct(multihop_vector),
        pct(multihop_graph)
    );
}
