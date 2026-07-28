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
