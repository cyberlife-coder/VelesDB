//! GATE 10 — the switch (#1762, PR C3).
//!
//! Two renames stand between a validated destination and a live store, and a
//! crash can land between any two steps. What these tests pin is that every
//! landing spot either CONTINUES forward — a rename that already happened is
//! journalled late, never undone — or refuses with both stores intact. The
//! discriminator for the ambiguous spot is the provenance stamp validation
//! wrote: the activated store carries the TARGET's stamp, the old source does
//! not, so "who is sitting at the source's name" is readable from disk.

use super::execute::{root_with_source, run, NEW_DIM};
use super::*;

fn validated(root: &std::path::Path) -> super::super::ExecuteOutcome {
    let executed = run(root).expect("execute");
    super::super::validate_destination(
        &root.join("store"),
        &executed.destination,
        &TargetContract::automatic("hash", NEW_DIM),
        1024,
    )
    .expect("validate");
    executed
}

fn journal(workspace: &std::path::Path) -> MigrationState {
    MigrationState::read(workspace)
        .expect("read journal")
        .expect("journal exists")
}

/// Advance the journal by hand, as the crashed run would have.
fn journal_phase(workspace: &std::path::Path, phase: Phase) {
    let lock = MigrationLock::acquire(workspace, "switch-test").expect("lock");
    let mut state = journal(workspace);
    state.phase = phase;
    state.write(workspace, &lock).expect("journal write");
    lock.release().expect("release");
}

#[test]
fn a_full_switch_activates_the_destination_and_frees_the_archive() {
    let root = root_with_source();
    let executed = validated(root.path());
    let store = root.path().join("store");
    let archive = root.path().join("store.archive");

    let outcome = super::super::switch_over(&store, &executed.destination).expect("switch");

    assert_eq!(
        outcome.activated,
        store.canonicalize().expect("the activated store exists"),
        "the new store must sit at the source's name"
    );
    assert!(
        !archive.exists(),
        "committing must free the archive — the recovery table says advancing \
         frees it, and this is the advance"
    );
    assert!(
        !executed.destination.exists(),
        "the destination directory moved; a copy left behind would be a third \
         authority"
    );
    assert_eq!(journal(&executed.workspace).phase, Phase::Committed);

    // The store at the source's name IS the rebuilt one: it opens, carries the
    // TARGET's provenance, and holds the rebuilt facts.
    let stamped = crate::embedding_provenance::read(&store)
        .expect("read provenance")
        .expect("the activated store carries the stamp validation wrote");
    assert_eq!(
        (stamped.model.as_str(), stamped.dimension),
        ("hash", NEW_DIM)
    );
    let db = velesdb_core::Database::open(&store).expect("the activated store opens");
    let facts = super::super::enumerate_by_cursor(&db, "_semantic_memory", 1024).expect("walk");
    assert_eq!(
        facts.len() as u64,
        super::execute::SEEDED,
        "the facts are the rebuilt ones"
    );
}

#[test]
fn a_switch_before_validation_is_refused() {
    let root = root_with_source();
    let executed = run(root.path()).expect("execute");

    let refusal = super::super::switch_over(&root.path().join("store"), &executed.destination)
        .expect_err("switching an unvalidated destination");
    assert!(
        refusal.to_string().contains("validate"),
        "the refusal must point at the missing validation: {refusal}"
    );
    assert!(root.path().join("store").exists(), "nothing may have moved");
    assert!(executed.destination.exists(), "nothing may have moved");
}

#[test]
fn an_occupied_archive_slot_is_refused_with_everything_intact() {
    let root = root_with_source();
    let executed = validated(root.path());
    let archive = root.path().join("store.archive");
    std::fs::create_dir(&archive).expect("occupy the archive slot");
    std::fs::write(archive.join("evidence.txt"), b"someone else's data").expect("mark it");

    let refusal = super::super::switch_over(&root.path().join("store"), &executed.destination)
        .expect_err("renaming the source over an existing archive would eat it");
    assert!(
        refusal.to_string().contains("store.archive"),
        "the refusal must name the occupied slot: {refusal}"
    );
    assert!(
        root.path().join("store").exists(),
        "the source must not have moved"
    );
    assert!(
        executed.destination.exists(),
        "the destination must not have moved"
    );
    assert!(
        std::fs::read(archive.join("evidence.txt")).is_ok(),
        "whatever occupies the slot must be untouched"
    );
    assert_eq!(
        journal(&executed.workspace).phase,
        Phase::DestinationValidated,
        "a refused switch must leave the journal where it was"
    );
}

#[test]
fn a_crash_after_the_first_rename_is_continued_not_undone() {
    let root = root_with_source();
    let executed = validated(root.path());
    let store = root.path().join("store");
    let archive = root.path().join("store.archive");

    // The crash window: rename one happened, its journal entry did not.
    std::fs::rename(&store, &archive).expect("simulate the crashed rename");
    assert_eq!(
        journal(&executed.workspace).phase,
        Phase::DestinationValidated
    );

    let outcome = super::super::switch_over(&store, &executed.destination)
        .expect("the re-run must journal the rename late and continue forward");
    assert_eq!(outcome.activated, store.canonicalize().expect("exists"));
    assert_eq!(journal(&executed.workspace).phase, Phase::Committed);
    assert!(!archive.exists(), "the completed switch frees the archive");
}

#[test]
fn a_crash_after_the_second_rename_is_recognised_by_the_stamp() {
    let root = root_with_source();
    let executed = validated(root.path());
    let store = root.path().join("store");
    let archive = root.path().join("store.archive");

    // Both renames happened; the journal only knows about the first. What sits
    // at the source's name is the ACTIVATED destination — and the proof is the
    // provenance stamp validation wrote on it.
    std::fs::rename(&store, &archive).expect("first rename");
    journal_phase(&executed.workspace, Phase::SourceArchived);
    std::fs::rename(&executed.destination, &store).expect("second rename");

    let outcome = super::super::switch_over(&store, &executed.destination)
        .expect("the stamp identifies the activated store; the run continues");
    assert_eq!(outcome.activated, store.canonicalize().expect("exists"));
    assert_eq!(journal(&executed.workspace).phase, Phase::Committed);
    assert!(!archive.exists());
}

#[test]
fn an_unstamped_occupant_of_the_source_slot_is_refused() {
    let root = root_with_source();
    let executed = validated(root.path());
    let store = root.path().join("store");
    let archive = root.path().join("store.archive");

    // Same disk shape as the crash above — source-name occupied, archive
    // present — but what occupies the slot does NOT carry the target's stamp:
    // this is the two-authorities shape nothing in this migration produced.
    std::fs::rename(&store, &archive).expect("first rename");
    journal_phase(&executed.workspace, Phase::SourceArchived);
    // The destination moves aside too: the shape under test is (source-name
    // occupied, archive present, destination GONE) — with the destination
    // still in place the disk is the three-authorities shape, which refuses
    // earlier and for a different reason.
    let elsewhere = root.path().join("elsewhere");
    std::fs::rename(&executed.destination, &elsewhere).expect("destination aside");
    std::fs::create_dir(&store).expect("an impostor at the source's name");

    let refusal = super::super::switch_over(&store, &executed.destination)
        .expect_err("an unstamped occupant cannot be assumed to be the destination");
    let message = refusal.to_string();
    assert!(
        message.contains("stamp") || message.contains("provenance"),
        "the refusal must say WHY the occupant is not trusted: {message}"
    );
    assert!(
        archive.exists(),
        "the archive — the only authority — must be untouched"
    );
    assert!(
        elsewhere.exists(),
        "the set-aside destination must be untouched"
    );
}

#[test]
fn a_committed_migration_refuses_to_switch_again() {
    let root = root_with_source();
    let executed = validated(root.path());
    let store = root.path().join("store");
    super::super::switch_over(&store, &executed.destination).expect("switch");

    let refusal = super::super::switch_over(&store, &executed.destination)
        .expect_err("a committed migration has nothing left to switch");
    assert!(
        refusal.to_string().contains("complete"),
        "the refusal must say the migration is done: {refusal}"
    );
}
