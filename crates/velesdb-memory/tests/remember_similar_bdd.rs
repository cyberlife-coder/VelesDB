//! Feature: `remember` reports the pre-existing memories a new fact is
//! closest to, and — only when the operator enables it — wires a `related_to`
//! edge to each hit at or above the configured similarity floor.
//!
//! An agent that re-derives a lesson it already stored should see that lesson
//! in `similar` before deciding which explicit `links` to pass; it should
//! never see the fact it just wrote, nor the internal scaffolding (entity
//! hubs) that autograph wires around facts.

#![cfg(feature = "persistence")]

mod common;

use common::{service, SharedTopicExtractor};
use tempfile::TempDir;
use velesdb_memory::service::SIMILAR_LIMIT;
use velesdb_memory::{HashEmbedder, Link, MemoryService};

const LESSON: &str = "the parser reports errors through ParseError, never by panicking";

/// A service with similarity auto-linking at `threshold`.
fn auto_linked(threshold: f32) -> (TempDir, MemoryService<HashEmbedder>) {
    let (dir, svc) = service();
    (dir, svc.with_auto_link(threshold))
}

fn related_to_targets(svc: &MemoryService<HashEmbedder>, fact: &str) -> Vec<u64> {
    svc.why(fact, 1, None)
        .expect("why")
        .edges
        .into_iter()
        .filter(|edge| edge.relation == "related_to")
        .map(|edge| edge.to)
        .collect()
}

/// Given two near-identical facts, when the second is remembered, then the
/// first is returned as similar — and the just-stored fact never lists
/// itself.
#[test]
fn remember_returns_existing_similars_but_never_itself() {
    let (_dir, svc) = service();
    svc.remember(LESSON, &[], None).expect("seed");
    let outcome = svc
        .remember_detailed(
            "the parser reports errors through ParseError and never panics",
            &[],
            None,
            None,
        )
        .expect("detailed remember");

    assert!(!outcome.similar.is_empty(), "the near-duplicate is similar");
    let best = &outcome.similar[0];
    assert!(best.score > 0.0, "similarity is reported raw: {best:?}");
    assert!(
        outcome.similar.iter().all(|hit| hit.id != outcome.fact_id),
        "the new fact must not list itself: {:?}",
        outcome.similar
    );
}

/// Given a fact remembered twice, when the second write runs the candidate
/// search, then the earlier copy — which IS the same id — is still excluded.
#[test]
fn re_remembered_fact_is_not_its_own_similar() {
    let (_dir, svc) = service();
    let first = svc.remember(LESSON, &[], None).expect("first");
    let outcome = svc
        .remember_detailed(LESSON, &[], None, None)
        .expect("re-remember");
    assert_eq!(outcome.fact_id, first, "content-addressed id is stable");
    assert!(
        !outcome.similar.iter().any(|hit| hit.id == first),
        "the stored copy must not come back as similar"
    );
}

/// Given more pre-existing memories than the page size, when a similar fact
/// lands, then the result is capped at `SIMILAR_LIMIT` entries.
#[test]
fn similar_page_is_capped() {
    let (_dir, svc) = service();
    for index in 0..SIMILAR_LIMIT + 3 {
        svc.remember(&format!("{LESSON} — variant {index}"), &[], None)
            .expect("seed");
    }
    let outcome = svc
        .remember_detailed(LESSON, &[], None, None)
        .expect("detailed remember");
    assert!(
        outcome.similar.len() <= SIMILAR_LIMIT,
        "similar is capped at {SIMILAR_LIMIT}: got {}",
        outcome.similar.len()
    );
}

/// Given extraction-built entity hubs around the seeds, when a fact is
/// remembered, then hubs — internal scaffolding, not caller memories — are
/// absent from `similar`.
#[test]
fn similar_excludes_entity_hubs() {
    let (_dir, svc) = service();
    let stored = svc
        .remember_extracted(
            "Alice ships the parser in Rust.",
            &SharedTopicExtractor,
            None,
        )
        .expect("extracted");
    assert!(!stored.ids.is_empty(), "fixture stored facts");
    let hub = svc
        .entity_profile("rust")
        .expect("entity lookup")
        .expect("rust hub exists");

    let outcome = svc
        .remember_detailed("Rust tooling notes for the parser team", &[], None, None)
        .expect("detailed remember");
    assert!(
        !outcome.similar.iter().any(|hit| hit.id == hub.id),
        "entity hub must not appear in similar: {:?}",
        outcome.similar
    );
}

/// Given auto-link left off (the default), when a near-duplicate is
/// remembered, then no `related_to` edge is written — the feature is opt-in.
#[test]
fn similar_edges_are_off_by_default() {
    let (_dir, svc) = service();
    svc.remember(LESSON, &[], None).expect("seed");
    let outcome = svc
        .remember_detailed(
            "the parser reports errors through ParseError and never panics",
            &[],
            None,
            None,
        )
        .expect("detailed remember");
    assert!(!outcome.similar.is_empty(), "fixture: similar exists");
    assert!(
        related_to_targets(
            &svc,
            "the parser reports errors through ParseError and never panics"
        )
        .is_empty(),
        "no automatic edges without opt-in"
    );
}

/// Given auto-link with a permissive floor, when a near-duplicate is
/// remembered, then every similar hit gains a `related_to` edge — and an
/// explicit `links` entry is preserved alongside, not replaced.
#[test]
fn enabled_auto_link_wires_related_to_without_touching_caller_links() {
    let (_dir, svc) = auto_linked(0.0);
    let seed = svc.remember(LESSON, &[], None).expect("seed");
    let anchor = svc
        .remember("an unrelated decision note", &[], None)
        .expect("link target");
    let new_fact = "the parser reports errors through ParseError and never panics";
    let outcome = svc
        .remember_detailed(
            new_fact,
            &[Link {
                target: anchor,
                relation: "about".to_string(),
            }],
            None,
            None,
        )
        .expect("detailed remember");

    let edges = svc.why(new_fact, 1, None).expect("why").edges;
    assert!(
        outcome.similar.iter().all(|hit| edges
            .iter()
            .any(|edge| edge.relation == "related_to" && edge.to == hit.id)),
        "every similar hit is wired: similar={:?} edges={:?}",
        outcome.similar,
        edges
    );
    assert!(
        edges
            .iter()
            .any(|edge| edge.relation == "about" && edge.to == anchor),
        "the caller's own link is preserved: {edges:?}"
    );
    assert!(
        !related_to_targets(&svc, new_fact).contains(&outcome.fact_id),
        "no self-edge: {new_fact}"
    );
    let _ = seed;
}

/// Given auto-link with a floor just above the observed score, when the same
/// scenario runs, then no edge is written; with a floor at the score, it is.
#[test]
fn auto_link_honors_its_threshold() {
    let (_dir, svc) = service();
    svc.remember(LESSON, &[], None).expect("seed");
    let outcome = svc
        .remember_detailed(
            "the parser reports errors through ParseError and never panics",
            &[],
            None,
            None,
        )
        .expect("detailed remember");
    let score = outcome.similar[0].score;
    assert!((0.0..=1.0).contains(&score), "raw similarity: {score}");

    let new_fact = "the parser reports errors through ParseError and never panics";
    let (_dir_b, above) = auto_linked(score + 0.05);
    above.remember(LESSON, &[], None).expect("seed");
    above
        .remember_detailed(new_fact, &[], None, None)
        .expect("detailed remember");
    assert!(
        related_to_targets(&above, new_fact).is_empty(),
        "a hit below the floor is not wired"
    );

    let (_dir_c, at) = auto_linked(score);
    at.remember(LESSON, &[], None).expect("seed");
    at.remember_detailed(new_fact, &[], None, None)
        .expect("detailed remember");
    assert!(
        !related_to_targets(&at, new_fact).is_empty(),
        "a hit at the floor is wired (inclusive bound)"
    );
}

/// Given auto-link enabled, when the same fact is remembered twice, then the
/// `related_to` edge count does not grow — the triplet-derived edge id makes
/// the second write a no-op.
#[test]
fn re_remembered_fact_does_not_duplicate_its_auto_edges() {
    let (_dir, svc) = auto_linked(0.0);
    svc.remember(LESSON, &[], None).expect("seed");
    let new_fact = "the parser reports errors through ParseError and never panics";
    svc.remember_detailed(new_fact, &[], None, None)
        .expect("first detailed remember");
    let before = related_to_targets(&svc, new_fact);
    assert!(!before.is_empty(), "fixture: edges were written");

    svc.remember_detailed(new_fact, &[], None, None)
        .expect("re-remember");
    assert_eq!(
        related_to_targets(&svc, new_fact),
        before,
        "re-remembering derives the same edge ids"
    );
}
