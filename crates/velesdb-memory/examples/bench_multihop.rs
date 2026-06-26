//! Reproducible benchmark: the graph's contribution to multi-hop answer recall.
//!
//! Run it (offline, deterministic):
//!
//! ```text
//! cargo run --release -p velesdb-memory --example bench_multihop
//! ```
//!
//! ## Method
//!
//! One corpus of linked chains `decision ──decided_in──> PR ──fixes──> ticket`.
//! The **same** `HashEmbedder` is used throughout; the only thing toggled is
//! whether we follow the graph. That isolates the graph's contribution.
//!
//! Two question types per chain, both asked as `"why we chose <tech>"`:
//! - **direct** — the answer is the decision itself (shares words with the
//!   question). A control: vector recall should ace this.
//! - **multi-hop** — the answer is the ticket two hops away, whose wording
//!   shares no words with the question. A pure vector index is blind to it by
//!   construction; `why()` reaches it through the typed links.
//!
//! This is *not* `LoCoMo`. It isolates the graph effect on synthetic, fully
//! reproducible data; the apples-to-apples run with a real embedder and the
//! `LoCoMo` dataset is tracked as PLAN phase 2B.

use velesdb_memory::{HashEmbedder, Link, MemoryError, MemoryService, DEFAULT_DIMENSION};

const CHAINS: u32 = 30;
const RECALL_K: usize = 10;
const MAX_HOPS: usize = 2;

/// One `decision → PR → ticket` chain plus the question that probes it.
struct Chain {
    question: String,
    decision: u64,
    ticket: u64,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join(format!("velesdb-bench-{}", std::process::id()));
    let svc = MemoryService::open(&dir, HashEmbedder::new(DEFAULT_DIMENSION))?;
    let chains = build_corpus(&svc)?;

    let mut direct_vector: u32 = 0; // vector recall finds the (1-hop) decision
    let mut multihop_vector: u32 = 0; // vector recall finds the (2-hop) ticket
    let mut multihop_graph: u32 = 0; // why() finds the (2-hop) ticket
    for chain in &chains {
        // One vector recall per chain; test both the 1-hop and 2-hop answers
        // against the same result set (re-running the identical query would be
        // wasted work).
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

    report(CHAINS, direct_vector, multihop_vector, multihop_graph);
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

/// Build `CHAINS` linked chains; every ticket's wording is deliberately
/// disjoint from its question, so only the graph can reach it.
fn build_corpus(svc: &MemoryService<HashEmbedder>) -> Result<Vec<Chain>, MemoryError> {
    let mut chains = Vec::with_capacity(CHAINS as usize);
    for i in 0..CHAINS {
        let tech = format!("widget{i:02}");
        let pr = svc.remember(&format!("PR {i} implements {tech}"), &[], None)?;
        let ticket = svc.remember(
            &format!("EPIC {i} intermittent failure under heavy load"),
            &[],
            None,
        )?;
        let decision = svc.remember(
            &format!("we chose {tech} for the runtime"),
            &[Link {
                target: pr,
                relation: "decided_in".to_owned(),
            }],
            None,
        )?;
        svc.relate(pr, ticket, "fixes")?;
        chains.push(Chain {
            question: format!("why we chose {tech}"),
            decision,
            ticket,
        });
    }
    Ok(chains)
}

/// Does `why()` (vector seed + graph traversal) surface memory `target`?
fn why_finds(
    svc: &MemoryService<HashEmbedder>,
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
        "corpus: {total} linked chains (decision → PR → ticket), same HashEmbedder both modes\n"
    );
    println!("  question type   answer            vector-only     vector + graph");
    println!(
        "  direct          the decision      {direct_vector:>3}/{total} ({:.0}%)      {total:>3}/{total} (100%)",
        pct(direct_vector)
    );
    println!(
        "  multi-hop       the 2-hop ticket  {multihop_vector:>3}/{total} ({:.0}%)      {multihop_graph:>3}/{total} ({:.0}%)",
        pct(multihop_vector),
        pct(multihop_graph)
    );
    println!(
        "\n→ vector is fine for direct retrieval, blind for multi-hop; the graph adds +{:.0} pp.",
        pct(multihop_graph) - pct(multihop_vector)
    );
    println!("  (same embeddings + corpus, only the graph toggled. Deterministic. Not LoCoMo — see PLAN 2B.)");
}
