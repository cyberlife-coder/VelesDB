//! GATE 5 — edges (#1762, PR C2a)
//!
//! The fact export is proven. Edges are the other half, and everything below
//! rests on one claim the feasibility note made on #1762: the fact walk and the
//! edge walk already agree on which endpoints are live, so an edge export can
//! be defined as "the edges between exported facts" without inventing a filter.
//!
//! That claim was reasoned, not measured, and reasoning got it wrong once in
//! each direction while this file was being written. It is now MEASURED, in
//! [`the_two_walks_agree_on_which_endpoints_are_live`], because the mechanism is
//! not where reading `AgentMemory::with_dimension` suggests. `MemoryTtl` is
//! indeed built empty there (`velesdb-core/src/agent/memory.rs:82`) and
//! `is_expired` does read that in-process map only
//! (`velesdb-core/src/agent/ttl.rs:191`) — but every subsystem constructor
//! immediately refills it from the durable payload key via
//! `memory_helpers::rebuild_ttl_from_payloads`
//! (`velesdb-core/src/agent/semantic_memory.rs:62`). The convergence is real,
//! and it hangs entirely on that one call: drop it and the edge export starts
//! carrying edges into facts the fact export refused, silently.

use super::*;
use velesdb_core::agent::AgentMemory;
use velesdb_core::collection::graph::GraphEdge;

/// Facts the export keeps.
const LIVE: u64 = 1;
const ALSO_LIVE: u64 = 2;
/// A fact that is live when its edge is written and expires afterwards — the
/// ordinary lifecycle, and the only one that could strand an edge on an
/// endpoint the fact export drops.
const LATER_EXPIRED: u64 = 3;
/// An epoch second in the past, so the engine's `expires_at <= now` holds.
const PAST: u64 = 1_000_000;
/// ...and one far enough ahead that it does not.
const FUTURE: u64 = 4_000_000_000;

/// An `AgentMemory` as a migration opens one: brand new, over a store that
/// already holds everything.
fn fresh_memory(dir: &std::path::Path) -> AgentMemory {
    let db = std::sync::Arc::new(velesdb_core::Database::open(dir).expect("open db"));
    AgentMemory::with_dimension(db, DIM).expect("agent memory")
}

fn outgoing_targets(dir: &std::path::Path, id: u64) -> BTreeSet<u64> {
    fresh_memory(dir)
        .semantic()
        .relations(id)
        .expect("relations")
        .iter()
        .map(GraphEdge::target)
        .collect()
}

/// Three facts and two edges out of [`LIVE`], one of them into
/// [`LATER_EXPIRED`]. Nothing is expired yet.
fn seeded_with_edges(dir: &std::path::Path) {
    let store = NativeStore::open(dir, DIM).expect("open source");
    for id in [LIVE, ALSO_LIVE, LATER_EXPIRED] {
        store
            .store_with_metadata(id, &format!("fact {id}"), &EMBEDDING, &meta(&[]))
            .expect("seed");
    }
    store
        .relate(LIVE, ALSO_LIVE, "mentions")
        .expect("live edge");
    store
        .relate(LIVE, LATER_EXPIRED, "mentions")
        .expect("edge that may dangle");
}

#[test]
fn the_two_walks_agree_on_which_endpoints_are_live() {
    let dir = tempfile::tempdir().expect("tempdir");
    seeded_with_edges(dir.path());

    // ARM A — a plain reopen. Establishes that edges are durable at all, so a
    // later disappearance is a filter and not a lost write.
    assert_eq!(
        outgoing_targets(dir.path(), LIVE),
        BTreeSet::from([ALSO_LIVE, LATER_EXPIRED]),
        "positive control: both edges must survive a reopen, or every arm below \
         is measuring an edge index that simply forgot"
    );

    // ARM B — the endpoint is rewritten by a RAW upsert carrying a FUTURE
    // expiry. This is the arm that rules out the rival explanation: if the raw
    // rewrite cost the edge its place in the index, the edge would vanish here
    // too, and arm C would prove nothing about expiry.
    super::preservation::seed_raw(dir.path(), LATER_EXPIRED, "fact 3", Some(FUTURE));
    assert_eq!(
        outgoing_targets(dir.path(), LIVE),
        BTreeSet::from([ALSO_LIVE, LATER_EXPIRED]),
        "rewriting the endpoint must not cost the edge its place; if it did, the \
         disappearance in arm C would be about the WRITE, not about the expiry"
    );

    // ARM C — the same rewrite, one field different: the expiry is now past.
    super::preservation::seed_raw(dir.path(), LATER_EXPIRED, "fact 3", Some(PAST));
    assert_eq!(
        outgoing_targets(dir.path(), LIVE),
        BTreeSet::from([ALSO_LIVE]),
        "REGRESSION GUARD for `rebuild_ttl_from_payloads`: the edge walk must drop \
         an edge into a DURABLY expired fact on a store it has only just opened. \
         The filter is `MemoryTtl`, whose map is refilled from the payload key at \
         construction — remove that refill and this assertion is the only thing \
         standing between a rebuild and an edge pointing at nothing"
    );

    // ...and the fact walk, reading the durable key directly, drops exactly the
    // same fact. Same store, same instant, two engines, one answer.
    assert_eq!(
        super::preservation::read_out(dir.path(), "_semantic_memory")
            .iter()
            .map(|fact| fact.id)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([LIVE, ALSO_LIVE]),
        "the fact walk must exclude the same endpoint the edge walk just did — \
         this agreement is what lets `export_edges` be defined as the edges \
         between exported facts, with no filter of its own to get wrong"
    );
}

// ---------------------------------------------------------------------------
// THE EXPORT ITSELF
//
// Everything above establishes that "the edges between exported facts" is a
// well-defined set. These prove the export carries it WHOLE — every field of
// every tuple — and refuses, loudly, the one shape it cannot carry honestly.
// ---------------------------------------------------------------------------

/// An edge reduced to something orderable, so two collections of edges can be
/// compared as SETS rather than as sequences. Properties are canonicalised
/// through a `BTreeMap`, whose JSON rendering is key-ordered and therefore
/// stable — a `HashMap` is neither `Ord` nor `Hash`, and comparing its debug
/// rendering would compare an iteration order.
type EdgeTuple = (u64, u64, u64, String, String);

fn tuple(edge: &GraphEdge) -> EdgeTuple {
    let props: std::collections::BTreeMap<_, _> = edge.properties().iter().collect();
    (
        edge.id(),
        edge.source(),
        edge.target(),
        edge.label().to_owned(),
        serde_json::to_string(&props).expect("properties render"),
    )
}

fn tuples(edges: &[GraphEdge]) -> BTreeSet<EdgeTuple> {
    edges.iter().map(tuple).collect()
}

fn weight(value: f64) -> serde_json::Map<String, Value> {
    let mut props = serde_json::Map::new();
    props.insert("weight".to_owned(), Value::from(value));
    props.insert("note".to_owned(), Value::from("carried verbatim"));
    props
}

/// Facts 1..=3 and several edges, two of them carrying PROPERTIES — the field
/// `MemoryEdge` does not even have, and therefore the one a transport type
/// chosen for convenience would silently drop.
fn source_with_rich_edges(dir: &std::path::Path) -> BTreeSet<EdgeTuple> {
    seeded_with_edges(dir);
    let memory = fresh_memory(dir);
    memory
        .semantic()
        .relate(ALSO_LIVE, LIVE, "contradicts", Some(&weight(0.75)))
        .expect("edge with properties");
    memory
        .semantic()
        .relate(LIVE, ALSO_LIVE, "supports", Some(&weight(0.25)))
        .expect("second edge with properties");
    let mut all = Vec::new();
    for id in [LIVE, ALSO_LIVE, LATER_EXPIRED] {
        all.extend(memory.semantic().relations(id).expect("relations"));
    }
    let observed = tuples(&all);
    assert!(
        observed.iter().any(|(.., props)| props.contains("weight")),
        "positive control: the fixture must actually carry properties, or the \
         export test below cannot notice them being dropped"
    );
    observed
}

fn opened(
    dir: &std::path::Path,
    dimension: usize,
) -> (std::sync::Arc<velesdb_core::Database>, AgentMemory) {
    let db = std::sync::Arc::new(velesdb_core::Database::open(dir).expect("open db"));
    let memory =
        AgentMemory::with_dimension(std::sync::Arc::clone(&db), dimension).expect("agent memory");
    (db, memory)
}

#[test]
fn every_agent_collection_is_dispatchable() {
    // `edges.rs` matches the three subsystems by name against
    // `AGENT_COLLECTIONS`. Adding a fourth collection to that constant without
    // teaching the dispatch about it would not fail to compile — it would fail
    // at run time, on a real store, halfway through a migration.
    let dir = tempfile::tempdir().expect("tempdir");
    seeded_with_edges(dir.path());
    let (db, memory) = opened(dir.path(), DIM);

    for collection in super::super::AGENT_COLLECTIONS {
        super::super::export_edges(&memory, &db, collection, 1024)
            .unwrap_or_else(|e| panic!("`{collection}` must be dispatchable, got {e}"));
    }

    let refused = super::super::export_edges(&memory, &db, "_not_a_subsystem", 1024).expect_err(
        "positive control: an unknown collection must be refused, or \
                     the loop above proves only that nothing ever errors",
    );
    assert!(
        refused.to_string().contains("_not_a_subsystem"),
        "the refusal must name the collection; got {refused}"
    );
}

#[test]
fn export_edges_carries_every_field_of_every_tuple() {
    let dir = tempfile::tempdir().expect("tempdir");
    let expected = source_with_rich_edges(dir.path());

    let (db, memory) = opened(dir.path(), DIM);
    let exported =
        super::super::export_edges(&memory, &db, "_semantic_memory", 1024).expect("export edges");

    assert_eq!(
        tuples(&exported),
        expected,
        "the export must carry id, source, target, label AND properties; a \
         transport type without a properties field would pass every other \
         assertion in this file"
    );
}

#[test]
fn export_edges_refuses_an_id_the_triple_does_not_derive() {
    let dir = tempfile::tempdir().expect("tempdir");
    seeded_with_edges(dir.path());

    // `VelesQL` DML accepts an explicit edge id (`database/dml_executor.rs:147`),
    // so a store can hold an edge whose id is not `hash_edge_id` of its triple.
    // Reinserting it would rederive a DIFFERENT id, and the rebuild would ship a
    // store where one logical edge carries two identities.
    {
        let db = velesdb_core::Database::open(dir.path()).expect("open");
        let any = db
            .get_any_collection("_semantic_memory")
            .expect("collection");
        let forged = GraphEdge::new(999_999, LIVE, ALSO_LIVE, "forged").expect("edge");
        any.add_edge(forged).expect("add forged edge");
    }

    let (db, memory) = opened(dir.path(), DIM);
    let refused = super::super::export_edges(&memory, &db, "_semantic_memory", 1024)
        .expect_err("an underived id must stop the export");
    let message = refused.to_string();
    assert!(
        message.contains("999999") && message.contains("forged"),
        "the refusal must name the offending edge and its label so an operator \
         can find it; got {message}"
    );
}

#[test]
fn the_incoming_walk_yields_the_same_tuples_as_the_outgoing_one() {
    let dir = tempfile::tempdir().expect("tempdir");
    let expected = source_with_rich_edges(dir.path());

    let (db, memory) = opened(dir.path(), DIM);
    let exported =
        super::super::export_edges(&memory, &db, "_semantic_memory", 1024).expect("export");
    let crossed = super::super::cross_check_edges(&memory, &db, "_semantic_memory", 1024)
        .expect("cross check");

    assert_eq!(
        tuples(&crossed),
        tuples(&exported),
        "the incoming index must yield the SAME tuples, not merely the same \
         count — two physically distinct indexes agreeing is the evidence; equal \
         cardinalities would also hold if both had lost the same number"
    );
    assert_eq!(
        tuples(&exported),
        expected,
        "and both must equal the source"
    );
}

#[test]
fn the_verified_export_walks_both_directions_over_one_snapshot() {
    let dir = tempfile::tempdir().expect("tempdir");
    let expected = source_with_rich_edges(dir.path());

    let (db, memory) = opened(dir.path(), DIM);
    let verified = super::super::export_edges_verified(&memory, &db, "_semantic_memory", 1024)
        .expect("verified export");
    assert_eq!(
        tuples(&verified),
        expected,
        "the verified entry point must return what the outgoing walk returns, \
         having agreed with the incoming one"
    );
    drop(memory);
    drop(db);

    // Why this entry point exists: `export_edges` and `cross_check_edges` each
    // enumerate the store for themselves, and expiry is read against the wall
    // clock on every read. A fact expiring between two such calls leaves them
    // disagreeing about an edge neither of them lost. The verified walk takes
    // ONE snapshot of the live ids, so it cannot see a fact live in one
    // direction and dead in the other however long the two halves take.
    super::preservation::seed_raw(dir.path(), LATER_EXPIRED, "fact 3", Some(PAST));
    let (db, memory) = opened(dir.path(), DIM);
    let after = super::super::export_edges_verified(&memory, &db, "_semantic_memory", 1024)
        .expect("verified export after expiry");
    assert!(
        after.len() < verified.len(),
        "positive control: expiring an endpoint must actually change the export, \
         or this test compares a walk against itself"
    );
    assert!(
        after
            .iter()
            .all(|edge| edge.source() != LATER_EXPIRED && edge.target() != LATER_EXPIRED),
        "no edge touching the expired fact may survive, in either direction"
    );
}

#[test]
fn reinserted_edges_are_read_back_identical_at_the_destination() {
    let dir = tempfile::tempdir().expect("tempdir");
    let expected = source_with_rich_edges(dir.path());

    let exported = {
        let (db, memory) = opened(dir.path(), DIM);
        super::super::export_edges(&memory, &db, "_semantic_memory", 1024).expect("export")
    };
    let facts = super::preservation::read_out(dir.path(), "_semantic_memory");

    // Facts FIRST — `relate` requires both endpoints live, so an edge-before-fact
    // order does not merely misorder the work, it cannot complete.
    let dest = super::preservation::destination();
    {
        let db = velesdb_core::Database::open(dest.path()).expect("open destination");
        for fact in &facts {
            super::super::reinsert(
                &db,
                "_semantic_memory",
                fact,
                &super::preservation::NEW_EMBEDDING,
            )
            .expect("reinsert fact");
        }
    }

    let (db, memory) = opened(dest.path(), super::preservation::NEW_EMBEDDING.len());
    let outcome = super::super::reinsert_edges(&memory, "_semantic_memory", &exported)
        .expect("reinsert edges");
    assert_eq!(
        outcome.inserted,
        exported.len() as u64,
        "every exported edge must land"
    );

    // The verdict is the RE-READ, never the return code: `relate` is idempotent
    // on an id that already exists and ignores the properties it was handed, so
    // a destination that dropped every property would still answer success.
    let read_back = super::super::export_edges(&memory, &db, "_semantic_memory", 1024)
        .expect("re-read destination");
    assert_eq!(
        tuples(&read_back),
        expected,
        "the destination must hold the SAME tuples, properties included, read \
         back out through the same export the source went through"
    );
}
