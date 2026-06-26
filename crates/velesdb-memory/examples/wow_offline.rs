//! Wow demo — offline: three engines, one memory. Vector finds the work,
//! the `ColumnStore` scopes it, the graph connects it to the answer.
//!
//! ```text
//! cargo run -p velesdb-memory --example wow_offline           # all three beats
//! cargo run -p velesdb-memory --example wow_offline -- recall # vector only
//! cargo run -p velesdb-memory --example wow_offline -- filter # vector + ColumnStore
//! cargo run -p velesdb-memory --example wow_offline -- why    # + graph (the answer)
//! ```
//!
//! Two projects both touched "billing retries". A vector recall returns the work
//! from both (noisy) and never the person. A metadata filter scopes it to one
//! project (`ColumnStore`). `why()` then hops topic → PR → author and lands on the
//! right person. Network-free, deterministic, single binary.

use velesdb_memory::mcp::DynEmbedder;
use velesdb_memory::{HashEmbedder, MemoryService, Metadata, DEFAULT_DIMENSION};

const DIM: &str = "\x1b[2m";
const RED: &str = "\x1b[31m";
const GREEN: &str = "\x1b[32m";
const BLUE: &str = "\x1b[36m";
const BOLD: &str = "\x1b[1m";
const OFF: &str = "\x1b[0m";

const QUESTION: &str = "who should I ask about billing retries";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let dir = std::env::temp_dir().join(format!("velesdb-wow-{}", std::process::id()));
    let svc = MemoryService::open(&dir, embedder()?)?;
    let answer = seed(&svc)?;

    if mode.is_empty() || mode == "recall" {
        show_recall(&svc)?;
    }
    if mode.is_empty() || mode == "filter" {
        show_filter(&svc)?;
    }
    if mode.is_empty() || mode == "why" {
        show_why(&svc, answer)?;
    }
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

/// Pick the embedder: deterministic `HashEmbedder` by default (offline,
/// reproducible); a real on-device model via Ollama when built with
/// `--features ollama` and `VELESDB_MEMORY_EMBEDDER=ollama` is set — that gives
/// genuine semantic scores instead of token hashes.
// Without the `ollama` feature this can't fail (it always returns the hash
// embedder), but the `Result` is part of the signature for the feature-on path.
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

/// A metadata map with a single `project` field, for the `ColumnStore` filter.
fn project(name: &str) -> Metadata {
    let mut meta = Metadata::new();
    meta.insert("project".to_owned(), name.into());
    meta
}

/// Seed two projects that both touched "billing retries": each a `topic → PR →
/// author` chain, tagged with its `project`. Returns the checkout author's id.
fn seed(svc: &MemoryService<DynEmbedder>) -> Result<u64, Box<dyn std::error::Error>> {
    let checkout = project("checkout");
    let topic_c = svc.remember(
        "checkout — billing retries now use exponential backoff",
        &[],
        Some(&checkout),
    )?;
    let pr_c = svc.remember(
        "PR #88 — exponential backoff for billing retries",
        &[],
        Some(&checkout),
    )?;
    let marcus = svc.remember("Marcus Lee — staff engineer", &[], Some(&checkout))?;
    svc.relate(topic_c, pr_c, "shipped_in")?;
    svc.relate(pr_c, marcus, "authored_by")?;

    let subs = project("subscriptions");
    let topic_s = svc.remember(
        "subscriptions — billing retries capped at 5 attempts",
        &[],
        Some(&subs),
    )?;
    let pr_s = svc.remember(
        "PR #57 — cap billing retries at 5 attempts",
        &[],
        Some(&subs),
    )?;
    let dana = svc.remember("Dana Park — staff engineer", &[], Some(&subs))?;
    svc.relate(topic_s, pr_s, "shipped_in")?;
    svc.relate(pr_s, dana, "authored_by")?;

    Ok(marcus)
}

/// Beat 1 — vector: the work surfaces, from BOTH projects, and never the person.
fn show_recall(svc: &MemoryService<DynEmbedder>) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n{BOLD}Someone asks:{OFF}  {QUESTION}?");
    println!("\n{BOLD}recall(){OFF}  {DIM}vector similarity — finds the work{OFF}");
    for hit in svc.recall(QUESTION, 4, None)? {
        println!("   {DIM}{:.2}{OFF}  {}", hit.score, hit.content);
    }
    println!("   {RED}✗  two projects touched it — and not one name in sight.{OFF}");
    Ok(())
}

/// Beat 2 — `ColumnStore`: an exact-match metadata filter scopes it to a project.
fn show_filter(svc: &MemoryService<DynEmbedder>) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n{BOLD}recall(filter = project:checkout){OFF}  {DIM}+ ColumnStore — scope it{OFF}");
    for hit in svc.recall(QUESTION, 2, Some(&project("checkout")))? {
        println!("   {BLUE}{:.2}{OFF}  {}", hit.score, hit.content);
    }
    println!("   {BLUE}↳  just checkout now — structured filtering, exact match.{OFF}");
    Ok(())
}

/// Beat 3 — graph: `why()` hops topic → PR → author and lands on the person.
fn show_why(
    svc: &MemoryService<DynEmbedder>,
    answer: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    assert!(
        svc.why(QUESTION, 2, Some(&project("checkout")))?
            .nodes
            .iter()
            .any(|node| node.id == answer),
        "the graph must reach the checkout author",
    );
    println!("\n{BOLD}why(filter = project:checkout){OFF}  {DIM}+ graph — connect it{OFF}");
    println!(
        "   \"billing retries\"  {DIM}─shipped_in→{OFF}  PR #88  {DIM}─authored_by→{OFF}  {GREEN}{BOLD}Marcus Lee{OFF}"
    );
    println!("   {GREEN}✓  ask Marcus Lee — he shipped it in checkout.{OFF}");
    println!("\n{BOLD}Vector finds. ColumnStore scopes. The graph connects.{OFF}");
    println!("{DIM}Offline. No API key. One ~9 MB binary.{OFF}   velesdb.com\n");
    Ok(())
}
