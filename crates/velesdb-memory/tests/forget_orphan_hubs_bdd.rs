//! BDD integration tests for the entity-hub garbage collection `forget` runs.
//!
//! An entity hub is scaffolding the memory builds on its own. Before this, it
//! outlived every fact that created it: retracting a fact left its entities
//! behind for good, so the graph accumulated nodes nothing could reach. These
//! tests pin both halves of the contract — the orphan goes, the entity another
//! fact still needs stays.
//!
//! Categories: Nominal, Edge, Negative.

mod common;

use common::service;
use velesdb_memory::extract::{ExtractError, ExtractedFact, Extractor};

/// Two facts sharing the topic `rust`; only the first mentions `parser`.
/// Forgetting the first must therefore collect `parser` and keep `rust`.
struct SharedTopicExtractor;

impl Extractor for SharedTopicExtractor {
    fn extract(&self, _text: &str) -> Result<Vec<ExtractedFact>, ExtractError> {
        Ok(vec![
            ExtractedFact {
                text: "Alice ships the parser in Rust.".to_string(),
                entities: vec!["rust".to_string(), "parser".to_string()],
            },
            ExtractedFact {
                text: "Bob maintains the Rust toolchain.".to_string(),
                entities: vec!["rust".to_string()],
            },
        ])
    }
}

#[test]
fn forget_collects_a_hub_no_surviving_fact_mentions() {
    let (_dir, svc) = service();
    let ids = svc
        .remember_extracted("seed", &SharedTopicExtractor, None)
        .expect("remember_extracted");

    assert!(
        svc.entity_profile("parser")
            .expect("entity_profile")
            .is_some(),
        "precondition: the hub must exist before the fact is forgotten"
    );

    svc.forget(ids[0])
        .expect("forget the only fact citing parser");

    assert!(
        svc.entity_profile("parser")
            .expect("entity_profile after forget")
            .is_none(),
        "a hub no surviving fact mentions must not outlive it"
    );
}

#[test]
fn forget_keeps_a_hub_another_fact_still_mentions() {
    let (_dir, svc) = service();
    let ids = svc
        .remember_extracted("seed", &SharedTopicExtractor, None)
        .expect("remember_extracted");

    svc.forget(ids[0]).expect("forget the first fact");

    assert!(
        svc.entity_profile("rust")
            .expect("entity_profile after forget")
            .is_some(),
        "an entity a surviving fact still refers to must be kept — forgetting \
         one fact about it is not forgetting the entity"
    );
}

#[test]
fn forgetting_every_fact_leaves_no_hub_behind() {
    let (_dir, svc) = service();
    let ids = svc
        .remember_extracted("seed", &SharedTopicExtractor, None)
        .expect("remember_extracted");

    for id in ids {
        svc.forget(id).expect("forget");
    }

    for entity in ["rust", "parser"] {
        assert!(
            svc.entity_profile(entity)
                .expect("entity_profile after forgetting everything")
                .is_none(),
            "no hub may survive once every fact behind it is retracted ({entity})"
        );
    }
}

#[test]
fn forget_on_a_plain_fact_without_hubs_still_reports_found() {
    let (_dir, svc) = service();
    let id = svc
        .remember("a fact that created no entity", &[], None)
        .expect("remember");

    let found = svc.forget(id).expect("forget");

    assert!(
        found,
        "hub collection must not change the found contract for a fact that \
         never created a hub"
    );
}

/// Issue #1662. The collector decided a hub was orphaned by reading only its
/// OUTGOING `mentions` edges — the pairs `remember_extracted` writes in both
/// directions. A `relate` posed by hand writes one direction only, and
/// relating a fact TO a hub is reachable: `entity()` hands out the hub id and
/// `relate` accepts any live target.
///
/// So a live fact could point at a hub the collector could not see, and the
/// hub was swept from under it — the caller's own edge silently lost, with
/// no error anywhere.
#[test]
fn forget_keeps_a_hub_a_live_fact_still_points_at() {
    let (_dir, svc) = service();
    let ids = svc
        .remember_extracted("seed", &SharedTopicExtractor, None)
        .expect("remember_extracted");
    let hub = svc
        .entity_profile("parser")
        .expect("entity_profile")
        .expect("precondition: the hub exists")
        .id;

    // A caller's own edge, one direction only — exactly what `relate` writes.
    let anchor = svc
        .remember(
            "An unrelated note that cites the parser hub by hand.",
            &[],
            None,
        )
        .expect("remember the anchor fact");
    svc.relate(anchor, hub, "cites").expect("relate by hand");

    // The only fact whose extraction created `parser` goes away.
    svc.forget(ids[0]).expect("forget the extracted fact");

    assert!(
        svc.entity_profile("parser")
            .expect("entity_profile after forget")
            .is_some(),
        "a hub a live fact still points at must survive the loss of its last \
         `mentions` edge — the caller's edge is the reference the collector missed"
    );

    // And once that anchor goes too, nothing references the hub any more.
    svc.forget(anchor).expect("forget the anchor");
    assert!(
        svc.entity_profile("parser")
            .expect("entity_profile after the anchor is gone")
            .is_none(),
        "with its last referent gone the hub is collected as before"
    );
}
