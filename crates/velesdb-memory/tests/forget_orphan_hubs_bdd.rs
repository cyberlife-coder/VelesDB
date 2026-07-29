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

use common::SharedTopicExtractor;

#[test]
fn forget_collects_a_hub_no_surviving_fact_mentions() {
    let (_dir, svc) = service();
    let ids = svc
        .remember_extracted("seed", &SharedTopicExtractor, None)
        .expect("remember_extracted")
        .ids;

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
        .expect("remember_extracted")
        .ids;

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
        .expect("remember_extracted")
        .ids;

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
fn forget_keeps_a_hub_a_manual_relate_still_points_at() {
    // A manual `relate` only writes the edge the caller asked for — unlike
    // `remember_extracted`'s `wire_entity`, it never adds the `mentions` edge
    // back from the hub. `collect_orphan_hubs` must still see it, or `forget`
    // deletes a hub a surviving fact's edge still targets, leaving that edge
    // pointing at nothing with no signal to the caller (issue #1662).
    let (_dir, svc) = service();
    let ids = svc
        .remember_extracted("seed", &SharedTopicExtractor, None)
        .expect("remember_extracted")
        .ids;
    let parser_hub = svc
        .entity_profile("parser")
        .expect("entity_profile")
        .expect("precondition: the parser hub must exist")
        .id;

    let manual_fact = svc
        .remember("a fact wired to parser by hand", &[], None)
        .expect("remember");
    svc.relate(manual_fact, parser_hub, "concerne")
        .expect("manual relate fact -> hub");

    // Forgetting every fact `remember_extracted` wired to the hub leaves only
    // the manual, one-directional edge pointing at it.
    for id in ids {
        svc.forget(id).expect("forget");
    }

    assert!(
        svc.entity_profile("parser")
            .expect("entity_profile after forgetting the extracted facts")
            .is_some(),
        "a hub a surviving fact still points at via a manual relate must not \
         be collected, even with no mentions edge back from the hub"
    );

    svc.forget(manual_fact)
        .expect("forget the last fact pointing at the hub");

    assert!(
        svc.entity_profile("parser")
            .expect("entity_profile after forgetting the manual fact")
            .is_none(),
        "once the manual fact is gone too, the hub must finally be collected"
    );
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
