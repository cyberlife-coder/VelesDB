//! Wow demo — offline, one binary: the graph surfaces what the vector misses.
//!
//! Run it with no network, no API key:
//!
//! ```text
//! cargo run -p velesdb-memory --example wow_offline
//! ```
//!
//! It plays out a real agent memory: during a work session the agent remembers
//! a decision, the PR that implemented it, and the ticket it fixed — linking
//! them. Days later, offline, it asks *why* the decision was made. A plain
//! similarity search never surfaces the ticket (no shared words); the graph
//! traversal does.

use velesdb_memory::{HashEmbedder, Link, MemoryService, Metadata, DEFAULT_DIMENSION};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // A throwaway store in a temp dir — nothing ever leaves this machine.
    let dir = std::env::temp_dir().join(format!("velesdb-wow-{}", std::process::id()));
    let svc = MemoryService::open(&dir, HashEmbedder::new(DEFAULT_DIMENSION))?;

    let mut meta = Metadata::new();
    meta.insert("project".to_owned(), "veles".into());

    println!("── During the work session ─────────────────────────────────");
    let pr = svc.remember(
        "PR #42 swaps the std Mutex for parking_lot",
        &[],
        Some(&meta),
    )?;
    let ticket = svc.remember(
        "EPIC-317: intermittent CI hang under load",
        &[],
        Some(&meta),
    )?;
    svc.remember(
        "we chose parking_lot to avoid lock poisoning after a panic",
        &[Link {
            target: pr,
            relation: "decided_in".to_owned(),
        }],
        Some(&meta),
    )?;
    svc.relate(pr, ticket, "fixes")?;
    println!("remembered:  decision ──decided_in──> PR #42 ──fixes──> EPIC-317");
    println!("             (all tagged project=veles)\n");

    println!("── Days later. Wi-Fi off. Fresh session. ───────────────────");
    let question = "why we chose parking_lot";

    println!("\nrecall(\"{question}\")   [vector similarity only]");
    for hit in svc.recall(question, 2, None)? {
        println!("   {:.2}  {}", hit.score, hit.content);
    }
    println!("   └─ EPIC-317 is nowhere here — it shares no words with the question.");

    println!("\nwhy(\"{question}\")      [vector seed + graph traversal]");
    let explanation = svc.why(question, 3, None)?;
    for node in &explanation.nodes {
        println!("   hop {}  {}", node.hop, node.content);
    }
    let reached_ticket = explanation.nodes.iter().any(|n| n.id == ticket);
    println!(
        "   └─ the graph reached EPIC-317: {}",
        if reached_ticket {
            "the very ticket the decision fixed."
        } else {
            "(unexpected)"
        }
    );

    println!("\nThe vector was blind to it. The graph connected it. Offline. One binary.");

    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}
