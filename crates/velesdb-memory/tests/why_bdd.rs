//! BDD integration tests for `why` — the multi-hop explanation path.
//!
//! This is the differentiator: `why` returns the *connected subgraph* behind a
//! decision, surfacing related memories a purely vector recall is blind to.
//!
//! Categories: Nominal (≥60%), Edge (~20%), Negative (≥20%).

mod common;

use common::service;
use tempfile::TempDir;
use velesdb_memory::{HashEmbedder, Link, MemoryService};

const DECISION: &str = "we chose parking_lot to avoid lock poisoning";

/// Build the canonical chain: decision -[`decided_in`]-> PR -[`tracked_by`]-> ticket.
/// The ticket wording is deliberately dissimilar to the decision text so that
/// only the graph (not vector similarity) can reach it.
fn seeded_chain() -> (TempDir, MemoryService<HashEmbedder>, u64, u64, u64) {
    let (dir, svc) = service();
    let decision = svc
        .remember(DECISION, &[], None)
        .expect("remember decision");
    let pr = svc
        .remember("PR #42 swaps the mutex implementation", &[], None)
        .expect("remember pr");
    let ticket = svc
        .remember("EPIC-317 xyzzy quux frobnicate", &[], None)
        .expect("remember ticket");
    svc.relate(decision, pr, "decided_in")
        .expect("relate decision->pr");
    svc.relate(pr, ticket, "tracked_by")
        .expect("relate pr->ticket");
    (dir, svc, decision, pr, ticket)
}

/// Node ids of the subgraph `why(DECISION, hops)` returns.
fn why_ids(svc: &MemoryService<HashEmbedder>, hops: usize) -> Vec<u64> {
    svc.why(DECISION, hops, None)
        .expect("why")
        .nodes
        .iter()
        .map(|n| n.id)
        .collect()
}

// --- Nominal ---------------------------------------------------------------

#[test]
fn why_returns_the_full_connected_subgraph() {
    let (_dir, svc, decision, pr, ticket) = seeded_chain();

    let explanation = svc.why(DECISION, 2, None).expect("why");

    let ids: Vec<u64> = explanation.nodes.iter().map(|n| n.id).collect();
    assert!(
        ids.contains(&decision),
        "subgraph must contain the decision"
    );
    assert!(ids.contains(&pr), "subgraph must contain the linked PR");
    assert!(
        ids.contains(&ticket),
        "subgraph must contain the 2-hop ticket"
    );
    assert_eq!(
        explanation.edges.len(),
        2,
        "two typed edges connect the chain"
    );
}

#[test]
fn why_assigns_hop_distances_from_the_seed() {
    let (_dir, svc, decision, pr, ticket) = seeded_chain();

    let explanation = svc.why(DECISION, 2, None).expect("why");
    let hop = |id: u64| explanation.nodes.iter().find(|n| n.id == id).map(|n| n.hop);

    assert_eq!(hop(decision), Some(0), "seed is hop 0");
    assert_eq!(hop(pr), Some(1), "PR is one hop away");
    assert_eq!(hop(ticket), Some(2), "ticket is two hops away");
}

#[test]
fn why_reaches_what_vector_recall_alone_misses() {
    let (_dir, svc, _decision, _pr, ticket) = seeded_chain();

    // The best single semantic match for the decision is NOT the ticket:
    // their wording shares no tokens.
    let top = svc.recall(DECISION, 1, None).expect("recall");
    assert!(
        top.iter().all(|h| h.id != ticket),
        "vector recall alone misses the ticket"
    );

    // The graph traversal reaches it anyway.
    let explanation = svc.why(DECISION, 2, None).expect("why");
    assert!(
        explanation.nodes.iter().any(|n| n.id == ticket),
        "the graph surfaces the connected ticket the vector is blind to"
    );
}

// --- Edge ------------------------------------------------------------------

#[test]
fn why_with_zero_hops_returns_only_the_seed() {
    let (_dir, svc, decision, _pr, _ticket) = seeded_chain();

    let explanation = svc.why(DECISION, 0, None).expect("why");

    assert_eq!(explanation.nodes.len(), 1, "no traversal at zero hops");
    assert_eq!(explanation.nodes[0].id, decision);
    assert!(explanation.edges.is_empty(), "no edges at zero hops");
}

#[test]
fn why_stops_at_the_hop_budget() {
    let (_dir, svc, decision, pr, ticket) = seeded_chain();

    let ids = why_ids(&svc, 1);

    assert!(
        ids.contains(&decision) && ids.contains(&pr),
        "one hop reaches the PR"
    );
    assert!(
        !ids.contains(&ticket),
        "one hop must not reach the two-hop ticket"
    );
}

#[test]
fn why_on_isolated_memory_returns_just_that_memory() {
    let (_dir, svc) = service();
    let lone = svc
        .remember("a fact with no relations", &[], None)
        .expect("remember");

    let explanation = svc.why("a fact with no relations", 3, None).expect("why");

    assert_eq!(explanation.nodes.len(), 1);
    assert_eq!(explanation.nodes[0].id, lone);
    assert!(explanation.edges.is_empty());
}

// --- Negative --------------------------------------------------------------

#[test]
fn why_on_empty_store_is_empty() {
    let (_dir, svc) = service();

    let explanation = svc.why("anything", 3, None).expect("why on empty store");

    assert!(explanation.nodes.is_empty(), "no seed, no explanation");
    assert!(explanation.edges.is_empty());
}

#[test]
fn why_via_links_argument_builds_the_same_graph() {
    // `remember(fact, links)` must produce edges traversable by `why`,
    // equivalent to explicit `relate` calls.
    let (_dir, svc) = service();
    let pr = svc
        .remember("PR #99 refactors the lock layer", &[], None)
        .expect("remember pr");
    let decision = svc
        .remember(
            DECISION,
            &[Link {
                target: pr,
                relation: "decided_in".to_owned(),
            }],
            None,
        )
        .expect("remember decision with link");

    let ids = why_ids(&svc, 1);

    assert!(
        ids.contains(&decision) && ids.contains(&pr),
        "link arg is traversable by why"
    );
}

#[test]
fn why_drops_edges_to_forgotten_targets() {
    let (_dir, svc) = service();
    let decision = svc
        .remember(DECISION, &[], None)
        .expect("remember decision");
    let pr = svc
        .remember("PR #7 implements the change", &[], None)
        .expect("remember pr");
    svc.relate(decision, pr, "decided_in").expect("relate");
    svc.forget(pr).expect("forget pr");

    let explanation = svc.why(DECISION, 2, None).expect("why");

    let node_ids: std::collections::HashSet<u64> = explanation.nodes.iter().map(|n| n.id).collect();
    assert!(!node_ids.contains(&pr), "forgotten target is not a node");
    for edge in &explanation.edges {
        assert!(
            node_ids.contains(&edge.from) && node_ids.contains(&edge.to),
            "every edge endpoint must be a node — no dangling edge to the forgotten target"
        );
    }
}

#[test]
fn why_on_blank_decision_is_empty() {
    let (_dir, svc, _decision, _pr, _ticket) = seeded_chain();

    let explanation = svc.why("   ", 2, None).expect("why on blank decision");

    assert!(explanation.nodes.is_empty() && explanation.edges.is_empty());
}
