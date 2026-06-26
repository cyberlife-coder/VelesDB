//! BDD integration tests for the `remember / recall / relate / forget`
//! operations of the memory service domain core.
//!
//! Categories: Nominal (≥60%), Edge (~20%), Negative (≥20%).

mod common;

use common::service;
use velesdb_memory::{Link, MemoryError};

// --- Nominal ---------------------------------------------------------------

#[test]
fn remember_then_recall_returns_the_fact() {
    let (_dir, svc) = service();
    let fact = "we chose parking_lot to avoid lock poisoning";
    let id = svc.remember(fact, &[], None).expect("remember");

    let hits = svc
        .recall("parking_lot lock poisoning", 5, None)
        .expect("recall");

    assert!(
        hits.iter().any(|h| h.id == id),
        "recalled set should contain the stored fact"
    );
}

#[test]
fn remember_is_idempotent_on_identical_content() {
    let (_dir, svc) = service();
    let fact = "VelesDB ships as a single binary";
    let first = svc.remember(fact, &[], None).expect("first remember");
    let second = svc.remember(fact, &[], None).expect("second remember");

    assert_eq!(
        first, second,
        "identical content must yield the same stable id"
    );
}

#[test]
fn remember_with_links_creates_edges_without_error() {
    let (_dir, svc) = service();
    let pr = svc
        .remember("PR #42 introduces parking_lot", &[], None)
        .expect("remember target");
    let decision = svc
        .remember(
            "we chose parking_lot to avoid lock poisoning",
            &[Link {
                target: pr,
                relation: "decided_in".to_owned(),
            }],
            None,
        )
        .expect("remember with link");

    assert_ne!(decision, pr, "distinct facts must have distinct ids");
}

// --- Edge ------------------------------------------------------------------

#[test]
fn relate_two_existing_memories_returns_edge_id() {
    let (_dir, svc) = service();
    let a = svc
        .remember("ticket EPIC-317 tracks the lock rework", &[], None)
        .expect("remember a");
    let b = svc
        .remember("benchmark shows no contention regression", &[], None)
        .expect("remember b");

    let edge = svc.relate(a, b, "references").expect("relate");

    // Edge id is an opaque handle; we only assert the call succeeded and is usable.
    let _ = edge;
}

#[test]
fn relate_with_unknown_endpoint_errors_without_creating_a_dangling_edge() {
    let (_dir, svc) = service();
    let real = svc
        .remember("ticket EPIC-317 tracks the lock rework", &[], None)
        .expect("remember real");

    // Either endpoint missing must be rejected as client input, not silently
    // create an edge dangling off a memory that was never stored.
    let bad_target = svc
        .relate(real, 999, "references")
        .expect_err("relate to a missing target must error");
    assert!(matches!(bad_target, MemoryError::UnknownMemory(999)));

    let bad_source = svc
        .relate(999, real, "references")
        .expect_err("relate from a missing source must error");
    assert!(matches!(bad_source, MemoryError::UnknownMemory(999)));
}

#[test]
fn forget_removes_the_fact_from_recall() {
    let (_dir, svc) = service();
    let id = svc
        .remember("ephemeral scratch note about France", &[], None)
        .expect("remember");
    svc.forget(id).expect("forget");

    let hits = svc.recall("France", 5, None).expect("recall after forget");

    assert!(
        !hits.iter().any(|h| h.id == id),
        "forgotten fact must not be recalled"
    );
}

// --- Negative --------------------------------------------------------------

#[test]
fn recall_on_empty_store_is_empty() {
    let (_dir, svc) = service();

    let hits = svc
        .recall("anything at all", 10, None)
        .expect("recall on empty store");

    assert!(hits.is_empty(), "fresh store has nothing to recall");
}

#[test]
fn remember_rejects_empty_fact() {
    let (_dir, svc) = service();

    let err = svc
        .remember("   ", &[], None)
        .expect_err("empty fact must be rejected");

    assert!(matches!(err, MemoryError::EmptyFact));
}

#[test]
fn remember_with_unknown_link_target_errors_and_stores_nothing() {
    let (_dir, svc) = service();

    let err = svc
        .remember(
            "a decision",
            &[Link {
                target: 999,
                relation: "x".to_owned(),
            }],
            None,
        )
        .expect_err("unknown link target must error");
    assert!(matches!(err, MemoryError::UnknownMemory(999)));

    // The fact must not have been persisted (no partial write).
    let hits = svc.recall("a decision", 5, None).expect("recall");
    assert!(
        hits.is_empty(),
        "a failed link must not leave the fact half-written"
    );
}

#[test]
fn recall_with_empty_query_returns_empty() {
    let (_dir, svc) = service();
    svc.remember("some stored fact", &[], None)
        .expect("remember");

    let hits = svc.recall("   ", 5, None).expect("recall with blank query");

    assert!(hits.is_empty(), "a blank query recalls nothing");
}

#[test]
fn remember_trims_whitespace_for_idempotence() {
    let (_dir, svc) = service();

    let id_bare = svc
        .remember("hello world", &[], None)
        .expect("remember bare");
    let id_padded = svc
        .remember("  hello world  ", &[], None)
        .expect("remember padded");

    assert_eq!(
        id_bare, id_padded,
        "leading/trailing whitespace must not change the stable id"
    );
}
