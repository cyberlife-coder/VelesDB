//! BDD integration tests for `MemoryService::unrelate` (issue #1661).
//!
//! `relate` was the only write with no undo: a mistaken edge could only be
//! removed by destroying the facts at its endpoints. `unrelate` is its exact
//! symmetric — same refusals, idempotent on an absent edge (a cleanup must be
//! replayable), and it never touches the facts or entity hubs themselves.
//!
//! Categories: Nominal, Edge, Negative.

mod common;

use common::service;
use velesdb_memory::extract::{ExtractError, ExtractedFact, Extractor};
use velesdb_memory::MemoryError;

/// One fact tagged with one topic, so an autograph `about` edge exists.
struct OneTopicExtractor;

impl Extractor for OneTopicExtractor {
    fn extract(&self, _text: &str) -> Result<Vec<ExtractedFact>, ExtractError> {
        Ok(vec![ExtractedFact {
            text: "The parser handles VelesQL.".to_string(),
            entities: vec!["parser".to_string()],
        }])
    }
}

#[test]
fn unrelate_removes_the_exact_edge_and_reports_it() {
    let (_dir, svc) = service();
    let from = svc
        .remember("decision: split the module", &[], None)
        .expect("remember");
    let to = svc
        .remember("cause: the file went past the gate", &[], None)
        .expect("remember");
    svc.relate(from, to, "caused_by").expect("relate");

    let outcome = svc.unrelate(from, to, "caused_by").expect("unrelate");

    assert!(
        outcome.found,
        "the edge existed and must be reported as found"
    );
    assert_eq!(
        outcome.removed, 1,
        "exactly the one matching edge is removed"
    );
    let explanation = svc
        .why("decision: split the module", 2, None)
        .expect("why after unrelate");
    assert!(
        explanation.edges.is_empty(),
        "the removed edge must no longer be traversable, got {:?}",
        explanation.edges
    );
}

#[test]
fn unrelate_leaves_both_endpoints_alive() {
    let (_dir, svc) = service();
    let from = svc.remember("fact alpha", &[], None).expect("remember");
    let to = svc.remember("fact beta", &[], None).expect("remember");
    svc.relate(from, to, "supports").expect("relate");

    svc.unrelate(from, to, "supports").expect("unrelate");

    // `relate` validates that BOTH endpoints still exist — recreating the
    // edge proves unrelate removed only the edge, not the facts.
    svc.relate(from, to, "supports")
        .expect("both endpoints must have survived the unrelate");
}

#[test]
fn unrelate_on_an_absent_edge_is_an_idempotent_no_op() {
    let (_dir, svc) = service();
    let from = svc.remember("fact alpha", &[], None).expect("remember");
    let to = svc.remember("fact beta", &[], None).expect("remember");
    svc.relate(from, to, "supports").expect("relate");

    svc.unrelate(from, to, "supports").expect("first unrelate");
    let outcome = svc
        .unrelate(from, to, "supports")
        .expect("second unrelate must not error");

    assert!(
        !outcome.found,
        "an already-removed edge is reported not-found"
    );
    assert_eq!(outcome.removed, 0);
}

#[test]
fn unrelate_only_removes_the_named_relation() {
    let (_dir, svc) = service();
    let from = svc.remember("fact alpha", &[], None).expect("remember");
    let to = svc.remember("fact beta", &[], None).expect("remember");
    svc.relate(from, to, "supports").expect("relate supports");
    svc.relate(from, to, "contradicts")
        .expect("relate contradicts");

    let outcome = svc.unrelate(from, to, "supports").expect("unrelate");

    assert!(outcome.found);
    assert_eq!(outcome.removed, 1, "the other label must be untouched");
    let explanation = svc.why("fact alpha", 1, None).expect("why");
    assert_eq!(
        explanation.edges.len(),
        1,
        "the `contradicts` edge must survive, got {:?}",
        explanation.edges
    );
    assert_eq!(explanation.edges[0].relation, "contradicts");
}

#[test]
fn unrelate_refuses_the_same_inputs_relate_refuses() {
    let (_dir, svc) = service();
    let id = svc.remember("fact alpha", &[], None).expect("remember");

    let err = svc.unrelate(id, id, "supports").expect_err("self-loop");
    assert!(
        matches!(err, MemoryError::SelfRelation(_)),
        "a self-loop is refused exactly like relate's, got {err:?}"
    );

    let other = svc.remember("fact beta", &[], None).expect("remember");
    let err = svc.unrelate(id, other, "").expect_err("empty label");
    assert!(
        matches!(err, MemoryError::InvalidRelation(_)),
        "an empty label is refused exactly like relate's, got {err:?}"
    );
}

/// The store does not distinguish an explicit edge from an autograph one, so
/// `unrelate` removes both alike. Correcting an autograph edge should instead
/// go through forget + remember of the source fact — a later `remember` of
/// the same passage can rebuild the edge removed here.
#[test]
fn unrelate_also_removes_an_autograph_edge() {
    let (_dir, svc) = service();
    let ids = svc
        .remember_extracted("seed", &OneTopicExtractor, None)
        .expect("remember_extracted");
    let hub = svc
        .entity_profile("parser")
        .expect("entity_profile")
        .expect("hub exists")
        .id;

    let outcome = svc.unrelate(ids[0], hub, "about").expect("unrelate");

    assert!(outcome.found, "the autograph `about` edge is removable too");
    assert_eq!(outcome.removed, 1);
}
