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
pub(super) fn journal_phase(workspace: &std::path::Path, phase: Phase) {
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
fn a_live_switch_retains_the_archive_until_the_new_generation_is_installed() {
    let root = root_with_source();
    let executed = validated(root.path());
    let store = root.path().join("store");
    let archive = root.path().join("store.archive");

    super::super::stage_live_switch(&store, &executed.destination).expect("stage");
    super::super::finalize_staged_live_switch(&store, &executed.destination).expect("finalize");
    assert!(
        archive.exists(),
        "the old generation still has a recovery copy"
    );
    assert_eq!(
        journal(&executed.workspace).phase,
        Phase::DestinationActivated
    );

    super::super::commit_retained_switch(&store, &executed.destination)
        .expect("commit after installing the new generation");
    assert!(!archive.exists(), "commit may now release the archive");
    assert_eq!(journal(&executed.workspace).phase, Phase::Committed);
}

#[test]
fn a_staged_live_switch_can_roll_back_before_activation() {
    let root = root_with_source();
    let executed = validated(root.path());
    let store = root.path().join("store");
    let archive = root.path().join("store.archive");

    super::super::stage_live_switch(&store, &executed.destination).expect("stage");
    assert!(store.exists() && archive.exists());
    assert!(!executed.destination.exists());
    assert_eq!(
        journal(&executed.workspace).phase,
        Phase::DestinationValidated,
        "physical staging must remain pre-activation in durable state"
    );

    super::super::rollback_staged_live_switch(&store, &executed.destination).expect("rollback");
    assert!(store.exists());
    assert!(!archive.exists());
    assert!(executed.destination.exists());
    assert_eq!(
        journal(&executed.workspace).phase,
        Phase::DestinationValidated
    );
}

#[test]
fn a_live_crash_after_the_first_rename_rolls_back_to_source_authority() {
    let root = root_with_source();
    let executed = validated(root.path());
    let store = root.path().join("store");
    let archive = root.path().join("store.archive");
    std::fs::rename(&store, &archive).expect("first rename");

    super::super::rollback_staged_live_switch(&store, &executed.destination)
        .expect("rollback first rename");

    assert!(store.exists());
    assert!(executed.destination.exists());
    assert!(!archive.exists());
    assert_eq!(
        journal(&executed.workspace).phase,
        Phase::DestinationValidated
    );
}

#[test]
fn a_staged_live_switch_is_journalled_only_after_activation() {
    let root = root_with_source();
    let executed = validated(root.path());
    let store = root.path().join("store");

    super::super::stage_live_switch(&store, &executed.destination).expect("stage");
    super::super::finalize_staged_live_switch(&store, &executed.destination).expect("finalize");
    assert_eq!(
        journal(&executed.workspace).phase,
        Phase::DestinationActivated
    );
    assert!(root.path().join("store.archive").exists());
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
fn a_source_that_moved_on_cannot_be_archived_by_a_stale_journal() {
    // The stamp alone cannot tell two migrations toward the same target
    // apart, and a crashed migration's journal stays at DestinationValidated
    // forever. If a LATER write lands in the source — a daemon, or a whole
    // second migration — this journal describes a store that no longer
    // exists, and archiving the live one on its say-so would end with commit
    // DESTROYING post-journal writes. The fingerprint is what notices.
    let root = root_with_source();
    let executed = validated(root.path());
    let store = root.path().join("store");
    {
        let native = crate::storage::NativeStore::open(&store, DIM).expect("open");
        native
            .store(77, "written after validation", &EMBEDDING)
            .expect("late write");
    }

    let refusal = super::super::switch_over(&store, &executed.destination)
        .expect_err("a stale journal must not move a store that moved on");
    let message = refusal.to_string();
    assert!(
        message.contains("fingerprint") || message.contains("changed"),
        "the refusal must say the source no longer matches the journal: {message}"
    );
    assert!(store.exists(), "nothing may have moved");
    assert!(executed.destination.exists(), "nothing may have moved");
}

#[test]
fn commit_refuses_to_free_an_archive_that_received_writes() {
    // The archive is the ONLY copy of the old data, and (measured in review)
    // neither a rename nor remove_dir_all is stopped by a daemon's flock on a
    // file inside it: a daemon that opened the source in the validate→switch
    // window keeps writing into the ARCHIVE after the first rename, silently.
    // Freeing that archive would destroy its writes. Before destruction, the
    // archive must still fingerprint as the settled source the journal knows.
    let root = root_with_source();
    let executed = validated(root.path());
    let store = root.path().join("store");
    let archive = root.path().join("store.archive");

    // Both renames done, journal caught up to DestinationActivated — and then
    // a foreign write lands in the archive.
    std::fs::rename(&store, &archive).expect("first rename");
    journal_phase(&executed.workspace, Phase::SourceArchived);
    std::fs::rename(&executed.destination, &store).expect("second rename");
    journal_phase(&executed.workspace, Phase::DestinationActivated);
    std::fs::write(archive.join("daemon-was-here.wal"), b"unsynced writes").expect("foreign write");

    let refusal = super::super::switch_over(&store, &executed.destination)
        .expect_err("an archive that changed since the journal must not be destroyed");
    let message = refusal.to_string();
    assert!(
        message.contains("archive"),
        "the refusal must name the archive: {message}"
    );
    assert!(
        std::fs::read(archive.join("daemon-was-here.wal")).is_ok(),
        "the foreign write — possibly the only copy of someone's data — must survive"
    );
    assert_eq!(
        journal(&executed.workspace).phase,
        Phase::DestinationActivated,
        "a refused commit must leave the journal where it was"
    );
}

#[test]
fn a_manual_restore_is_re_archived_and_the_switch_completes() {
    // The recovery table's advice for SourceArchived is "move the archive
    // back to the source's name". An operator who follows it and then re-runs
    // the switch presents: source at its name, no archive, destination
    // intact, journal at SourceArchived — the journal AHEAD of the disk. The
    // switch redoes the first rename (fingerprint-checked) and completes,
    // instead of stranding the migration in a shape everything refuses.
    let root = root_with_source();
    let executed = validated(root.path());
    let store = root.path().join("store");
    let archive = root.path().join("store.archive");

    std::fs::rename(&store, &archive).expect("first rename");
    journal_phase(&executed.workspace, Phase::SourceArchived);
    std::fs::rename(&archive, &store).expect("the operator's manual restore");

    let outcome = super::super::switch_over(&store, &executed.destination)
        .expect("a restored source under a SourceArchived journal must re-archive and continue");
    assert_eq!(outcome.activated, store.canonicalize().expect("exists"));
    assert_eq!(journal(&executed.workspace).phase, Phase::Committed);
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
