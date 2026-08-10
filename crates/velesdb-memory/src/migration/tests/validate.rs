//! GATE 9 — validation of the rebuilt destination (#1762, PR C3).
//!
//! The rebuild's own re-reads run WHILE it writes, collection by collection.
//! This is the other kind of proof: one pass over the finished destination,
//! against the source as it stands, before anything is allowed to move. It is
//! also where the destination earns its provenance stamp — the daemon reads
//! `embedding-provenance.json` before it opens a store, so an unstamped
//! destination would degrade into the unrecorded-model warning the moment the
//! switch put it live.
//!
//! Divergences have exactly one tolerated explanation: a fact whose ABSOLUTE
//! expiry passed between the two walks — the clock window Fable's C2b review
//! named, discriminated mechanically here as promised there. Everything else
//! is loss, and it is named.

use super::execute::{root_with_source, run, NEW_DIM, SEEDED};
use super::*;

#[test]
fn a_validated_destination_advances_the_journal_and_carries_provenance() {
    let root = root_with_source();
    let executed = run(root.path()).expect("execute");

    let outcome = super::super::validate_destination(
        &root.path().join("store"),
        &executed.destination,
        &TargetContract::automatic("hash", NEW_DIM),
        1024,
    )
    .expect("validate");

    assert_eq!(outcome.facts, SEEDED, "every fact must have been compared");
    assert_eq!(outcome.edges, 1, "every edge must have been compared");
    assert_eq!(
        outcome.explained_by_expiry, 0,
        "nothing expired mid-walk in this fixture; a nonzero count here means \
         the discriminator explained something it should not have"
    );

    let journalled = MigrationState::read(&executed.workspace)
        .expect("read journal")
        .expect("journal exists");
    assert_eq!(
        journalled.phase,
        Phase::DestinationValidated,
        "the validation is worthless unless the journal records it"
    );

    let stamped = crate::embedding_provenance::read(&executed.destination)
        .expect("read provenance")
        .expect("the validated destination must carry a provenance stamp");
    assert_eq!(
        (stamped.model.as_str(), stamped.dimension),
        ("hash", NEW_DIM),
        "the stamp must name the TARGET embedder — the daemon reads it before \
         opening, and a wrong stamp would be trusted forever"
    );

    // Idempotence: validating a validated destination succeeds and changes
    // nothing — the re-run an operator performs after any doubt.
    super::super::validate_destination(
        &root.path().join("store"),
        &executed.destination,
        &TargetContract::automatic("hash", NEW_DIM),
        1024,
    )
    .expect("re-validation is idempotent");
}

#[test]
fn a_fact_the_source_does_not_hold_is_refused_by_name() {
    let root = root_with_source();
    let executed = run(root.path()).expect("execute");

    // A live intruder written straight into the destination — the shape of a
    // corruption, an operator mistake, or a rebuild bug. It carries no expiry,
    // so the discriminator must NOT explain it away.
    {
        let db = velesdb_core::Database::open(&executed.destination).expect("open destination");
        let any = db
            .get_any_collection("_semantic_memory")
            .expect("collection");
        any.upsert(vec![velesdb_core::Point::new(
            999,
            vec![0.0; NEW_DIM],
            Some(serde_json::json!({ "content": "an intruder the source never held" })),
        )])
        .expect("plant intruder");
    }

    let refusal = super::super::validate_destination(
        &root.path().join("store"),
        &executed.destination,
        &TargetContract::automatic("hash", NEW_DIM),
        1024,
    )
    .expect_err("a destination holding a fact the source does not is not valid");
    let message = refusal.to_string();
    assert!(
        message.contains("_semantic_memory") && message.contains("999"),
        "the refusal must name the collection and the fact: {message}"
    );

    // ...and the journal must NOT have advanced past a failed validation.
    let journalled = MigrationState::read(&executed.workspace)
        .expect("read journal")
        .expect("journal exists");
    assert_eq!(
        journalled.phase,
        Phase::Prepared,
        "a failed validation must leave the journal exactly where it was"
    );
}

#[test]
fn an_unfinished_rebuild_cannot_be_validated() {
    let root = root_with_source();
    // A destination and a journal staged by hand, with one collection still
    // mid-facts — the state a killed rebuild leaves.
    let destination = root.path().join("rebuilt");
    {
        let _store =
            crate::storage::NativeStore::open(&destination, NEW_DIM).expect("create destination");
    }
    let workspace = root.path().join("rebuilt.migration-journal");
    std::fs::create_dir_all(&workspace).expect("workspace");
    let lock = MigrationLock::acquire(&workspace, "unfinished-test").expect("lock");
    let mut progress = std::collections::BTreeMap::new();
    for name in AGENT_COLLECTIONS {
        progress.insert((*name).to_owned(), CollectionProgress::Complete);
    }
    progress.insert(
        "_episodic_memory".to_owned(),
        CollectionProgress::Facts { cursor: Some(3) },
    );
    let state = MigrationState {
        format_version: STATE_FORMAT_VERSION,
        phase: Phase::Prepared,
        source_path: root.path().join("store"),
        source_fingerprint: super::super::fingerprint(&root.path().join("store"))
            .expect("fingerprint"),
        target_model: "hash".to_owned(),
        target_dimension: NEW_DIM,
        progress,
        embedder_witness: None,
    };
    state.write(&workspace, &lock).expect("journal");
    lock.release().expect("release");

    let refusal = super::super::validate_destination(
        &root.path().join("store"),
        &destination,
        &TargetContract::automatic("hash", NEW_DIM),
        1024,
    )
    .expect_err("validating a rebuild that is not finished");
    assert!(
        refusal.to_string().contains("_episodic_memory"),
        "the refusal must name the unfinished collection: {refusal}"
    );
}

#[test]
fn a_source_that_changed_since_the_rebuild_is_refused() {
    let root = root_with_source();
    let executed = run(root.path()).expect("execute");

    // The daemon (or anyone) writes one more fact to the SOURCE after the
    // rebuild. Validating against it would compare the destination to a store
    // it was never built from, and every difference would read as loss.
    {
        let store =
            crate::storage::NativeStore::open(root.path().join("store"), DIM).expect("open");
        store
            .store(100, "written after the rebuild", &EMBEDDING)
            .expect("late write");
    }

    let refusal = super::super::validate_destination(
        &root.path().join("store"),
        &executed.destination,
        &TargetContract::automatic("hash", NEW_DIM),
        1024,
    )
    .expect_err("a moved source invalidates the comparison, not the destination");
    let message = refusal.to_string();
    assert!(
        message.contains("fingerprint") || message.contains("changed"),
        "the refusal must say the SOURCE moved, not accuse the destination: {message}"
    );
}

#[test]
fn the_expiry_discriminator_tells_vanished_from_live() {
    // The only tolerated divergence is a fact whose absolute expiry passed
    // between the two walks. That race cannot be staged deterministically, and
    // the expiry itself cannot be read back — an expired point is invisible on
    // EVERY public read surface, `get` answering `None` for it exactly as for
    // a deleted one. What the discriminator actually tests is that narrower,
    // sufficient fact: under the validation's held lock, an id a walk returned
    // that now reads back absent can only have expired. So: an expired point
    // explains (it reads back absent), a live one does not. An id the store
    // never held ALSO reads back absent — the function cannot tell, which is
    // why its contract restricts callers to ids this session's walks returned;
    // that restriction is upheld by construction in `compare_facts` and
    // `compare_edges`, where every probed id comes out of a walk's diff.
    let root = root_with_source();
    let store = root.path().join("store");
    super::preservation::seed_raw(&store, 50, "expired long ago", Some(1_000_000));

    let db = velesdb_core::Database::open(&store).expect("open");
    assert!(
        super::super::divergence_explained_by_expiry(&db, "_semantic_memory", 50),
        "a point whose absolute expiry passed must explain its own absence"
    );
    assert!(
        !super::super::divergence_explained_by_expiry(&db, "_semantic_memory", 1),
        "a live point explains nothing — its divergence is loss"
    );
}
