use super::diagnosis::{drift, tree, TARGET_DIM, TARGET_MODEL};
use super::*;

const VALID_FINGERPRINT: &str =
    "sha256-tree-v2:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

// ---------------------------------------------------------------------------
// GATE 5 — the lock and the phase journal
//
// A rebuild can stop anywhere. What has to hold is not that it never stops, but
// that every place it CAN stop has one defined action, and that a stop whose
// meaning the disk does not determine changes nothing at all.
// ---------------------------------------------------------------------------

/// A state that would resume cleanly, so each test can change exactly one thing
/// and attribute the refusal to it.
fn resumable_state() -> MigrationState {
    MigrationState {
        format_version: super::STATE_FORMAT_VERSION,
        phase: Phase::Prepared,
        source_path: std::path::PathBuf::from("/store"),
        source_fingerprint: VALID_FINGERPRINT.to_owned(),
        target_model: TARGET_MODEL.to_owned(),
        target_dimension: TARGET_DIM,
        progress: AGENT_COLLECTIONS
            .iter()
            .map(|name| {
                (
                    (*name).to_owned(),
                    CollectionProgress::Facts { cursor: None },
                )
            })
            .collect(),
    }
}

/// The explanation a recovery carries, whichever action it names.
fn stated(recovery: &Recovery) -> &str {
    match recovery {
        Recovery::Continue { rationale, .. } => rationale,
        Recovery::Restore { action } => action,
        Recovery::Refuse { reason } => reason,
    }
}

/// Every switch layout that does NOT determine what happened.
const AMBIGUOUS: &[(bool, bool, bool)] = &[
    (true, true, true),
    (true, true, false),
    (false, false, true),
    (false, false, false),
];

/// Every switch layout that DOES.
const DECIDABLE: &[(bool, bool, bool)] = &[
    (false, true, true),
    (false, true, false),
    (true, false, true),
    (true, false, false),
];

fn switch(triple: (bool, bool, bool)) -> SwitchState {
    SwitchState {
        source: triple.0,
        archive: triple.1,
        destination: triple.2,
    }
}

#[test]
fn two_migrations_cannot_hold_the_lock() {
    let workspace = tempfile::tempdir().expect("tempdir");

    // (1) a free lock is taken.
    let first = MigrationLock::acquire(workspace.path(), "run-A").expect("the first must succeed");

    // (2) a second acquisition fails while the first is held...
    let refusal = MigrationLock::acquire(workspace.path(), "run-B")
        .expect_err("two migrations must not hold one workspace");
    // (3) ...and the refusal names who has it.
    assert!(
        refusal.contains("run-A"),
        "the refusal must name the holder, or an operator cannot tell a stale \
         lock from a live one: {refusal}"
    );
    assert!(
        refusal.contains("wait") && refusal.contains("NOT stolen"),
        "the active-guard refusal must say what an operator can safely do: {refusal}"
    );

    // (6) and it is REFUSED, not stolen: no pid, no port, no liveness check.
    assert!(
        !refusal.to_lowercase().contains("pid")
            || refusal.contains("no process id or port is consulted"),
        "the lock must not be broken on a liveness check: {refusal}"
    );
    assert!(
        workspace.path().join(super::LOCK_FILE).exists(),
        "a refused acquisition must leave the existing lock exactly where it was"
    );

    // (4) releasing frees it, and (5) it can then be taken again.
    first.release().expect("release");
    assert!(
        !workspace.path().join(super::LOCK_FILE).exists(),
        "release must remove the lock file"
    );
    let second =
        MigrationLock::acquire(workspace.path(), "run-B").expect("a released lock is free again");

    // The positive control for the refusal above: without this, an `acquire`
    // that always failed would satisfy every assertion so far.
    assert_eq!(
        MigrationLock::holder(workspace.path()).as_deref(),
        Some("held_by=run-B"),
        "the lock must record its new holder"
    );
    second.release().expect("release");

    // (6) again, deliberately: a lock left behind by a process that is gone is
    // still refused. Nothing here is alive, and that changes nothing.
    std::fs::write(
        workspace.path().join(super::LOCK_FILE),
        "held_by=a-dead-run\n",
    )
    .expect("plant a stale lock");
    let stale = MigrationLock::acquire(workspace.path(), "run-C")
        .expect_err("a stale lock must be refused, never stolen");
    assert!(
        stale.contains("a-dead-run"),
        "the refusal must name the holder recorded in the stale lock: {stale}"
    );
}

#[test]
fn the_lock_never_lives_in_the_source() {
    // Property (7). The diagnosis contract is that the source is not written
    // to; a lock file placed there would make the act of asking a write.
    let (source, _ttl) = seeded();
    let before = tree(source.path());
    let workspace = tempfile::tempdir().expect("tempdir");

    let lock = MigrationLock::acquire(workspace.path(), "run-A").expect("acquire");

    assert!(
        workspace.path().join(super::LOCK_FILE).exists(),
        "positive control: the lock must actually have been created somewhere"
    );
    assert!(
        !source.path().join(super::LOCK_FILE).exists(),
        "the migration lock must never be placed in the source"
    );
    assert!(
        !source
            .path()
            .join(super::super::state::LOCK_GUARD_FILE)
            .exists(),
        "the persistent OS guard must never be placed in the source"
    );
    assert!(
        drift(&before, &tree(source.path())).is_empty(),
        "taking the lock must not touch the source at all"
    );
    lock.release().expect("release");
}

#[test]
fn a_newer_state_version_is_refused() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let lock = MigrationLock::acquire(workspace.path(), "state-version-test").expect("lock");

    // The positive control first: a state at THIS version reads back.
    resumable_state()
        .write(workspace.path(), &lock)
        .expect("write");
    let read = MigrationState::read(workspace.path())
        .expect("a state at the current version must read")
        .expect("it exists");
    assert_eq!(read, resumable_state(), "a state must round-trip verbatim");

    // Now a state from the future, carrying a field this build knows nothing
    // about — the shape a newer version would genuinely have.
    let newer = serde_json::json!({
        "format_version": super::STATE_FORMAT_VERSION + 1,
        "phase": "prepared",
        "source_path": "/store",
        "source_fingerprint": VALID_FINGERPRINT,
        "target_model": TARGET_MODEL,
        "target_dimension": TARGET_DIM,
        "a_field_from_the_future": { "that": "this build cannot interpret" },
    });
    std::fs::write(
        workspace.path().join(super::STATE_FILE),
        serde_json::to_string_pretty(&newer).expect("json"),
    )
    .expect("write newer state");

    let refusal = MigrationState::read(workspace.path())
        .expect_err("a state from a newer version must be refused");
    assert!(
        refusal.contains(&format!("version {}", super::STATE_FORMAT_VERSION + 1)),
        "the refusal must name the version it found: {refusal}"
    );
    assert!(
        !refusal.contains("does not parse"),
        "the refusal must be about the VERSION, not about a parse failure — a \
         newer state is expected to carry fields this build cannot read, and \
         reporting that as corruption would send the operator after the wrong \
         problem: {refusal}"
    );

    // ...and the same refusal is reachable without going through the file, so
    // an in-memory state cannot bypass it.
    let mut from_future = resumable_state();
    from_future.format_version = super::STATE_FORMAT_VERSION + 1;
    assert!(
        from_future
            .may_resume(
                &from_future.source_path,
                &from_future.source_fingerprint,
                TARGET_MODEL,
                TARGET_DIM,
            )
            .is_err(),
        "may_resume must refuse a newer version too"
    );
    lock.release().expect("release");
}

#[test]
fn an_older_weak_fingerprint_state_requires_a_fresh_diagnosis() {
    let workspace = tempfile::tempdir().expect("tempdir");
    // Pinned to the LITERAL version 1, not `STATE_FORMAT_VERSION - 1`: this
    // fixture is a real v1 file with the weak length-only fingerprint v1
    // actually carried. Version-relative arithmetic made the fixture drift —
    // at v3 it described a "v2" file with a fingerprint no v2 build ever
    // wrote. (The v2 boundary has its own test in `rebuild_state`.)
    let old = serde_json::json!({
        "format_version": 1,
        "phase": "prepared",
        "source_path": "/store",
        "source_fingerprint": "fnv1a64:0123456789abcdef",
        "target_model": TARGET_MODEL,
        "target_dimension": TARGET_DIM,
    });
    std::fs::write(
        workspace.path().join(super::STATE_FILE),
        serde_json::to_string_pretty(&old).expect("json"),
    )
    .expect("write old state");

    let refusal = MigrationState::read(workspace.path())
        .expect_err("a weak length-only fingerprint must never resume");
    assert!(
        refusal.contains("older") && refusal.contains("fresh diagnosis"),
        "the refusal must name the state as older and say how to recover: {refusal}"
    );
    assert!(
        refusal.contains("version 1") && refusal.contains(&super::STATE_FORMAT_VERSION.to_string()),
        "the refusal must name both versions: {refusal}"
    );

    let mut in_memory = resumable_state();
    in_memory.format_version -= 1;
    in_memory.source_fingerprint = "fnv1a64:0123456789abcdef".to_owned();
    let refusal = in_memory
        .may_resume(
            &in_memory.source_path,
            &in_memory.source_fingerprint,
            TARGET_MODEL,
            TARGET_DIM,
        )
        .expect_err("an in-memory old state must not bypass the version gate");
    assert!(refusal.contains("fresh diagnosis"), "{refusal}");
}

#[test]
fn a_changed_source_fingerprint_refuses_resume() {
    let state = resumable_state();

    // Positive control: the unchanged fingerprint resumes.
    state
        .may_resume(
            &state.source_path,
            &state.source_fingerprint,
            TARGET_MODEL,
            TARGET_DIM,
        )
        .expect("an unchanged source must resume");

    let refusal = state
        .may_resume(
            &state.source_path,
            "sha256-tree-v2:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            TARGET_MODEL,
            TARGET_DIM,
        )
        .expect_err("a source that changed under a prepared migration must refuse");
    assert!(
        refusal.contains(&state.source_fingerprint) && refusal.contains("ffffffffffffffff"),
        "the refusal must name BOTH fingerprints — one of them alone leaves the \
         operator guessing which side moved: {refusal}"
    );

    // And the fingerprint really is sensitive to a changed store, or the check
    // above guards nothing.
    let (dir, _ttl) = seeded();
    let before = super::fingerprint(dir.path()).expect("fingerprint");
    std::fs::write(dir.path().join("a-new-file"), b"something").expect("write");
    let after = super::fingerprint(dir.path()).expect("fingerprint");
    assert_ne!(
        before, after,
        "the fingerprint must move when the store does; a constant would make \
         every resume look safe"
    );
    assert_eq!(
        super::fingerprint(dir.path()).expect("fingerprint"),
        after,
        "and it must be stable when the store is not — a fingerprint that \
         changed on its own would refuse every legitimate resume"
    );
}

#[test]
fn a_same_size_content_change_changes_the_fingerprint() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("payload.bin");
    std::fs::write(&file, b"AAAA").expect("write initial content");
    let before = super::fingerprint(dir.path()).expect("fingerprint before");

    std::fs::write(&file, b"BBBB").expect("replace with same-size content");
    let after = super::fingerprint(dir.path()).expect("fingerprint after");

    assert_ne!(
        before, after,
        "a content edit that preserves file length must invalidate a prepared migration"
    );
}

#[test]
fn a_changed_target_model_refuses_resume() {
    let state = resumable_state();

    // Positive control.
    state
        .may_resume(
            &state.source_path,
            &state.source_fingerprint,
            TARGET_MODEL,
            TARGET_DIM,
        )
        .expect("the prepared model must resume");

    let refusal = state
        .may_resume(
            &state.source_path,
            &state.source_fingerprint,
            "some-other-model",
            TARGET_DIM,
        )
        .expect_err("a migration prepared for one model must not resume against another");
    assert!(
        refusal.contains(TARGET_MODEL) && refusal.contains("some-other-model"),
        "the refusal must name both models: {refusal}"
    );
    assert!(
        refusal.contains("not searchable") || refusal.contains("Half"),
        "the refusal must say WHY — half a store embedded by one model and half \
         by another is the failure, and it is invisible at read time: {refusal}"
    );
}

#[test]
fn changed_source_path_and_target_dimension_refuse_resume() {
    let state = resumable_state();

    let path_refusal = state
        .may_resume(
            std::path::Path::new("/another-store"),
            &state.source_fingerprint,
            TARGET_MODEL,
            TARGET_DIM,
        )
        .expect_err("a journal must not transfer to another source path");
    assert!(path_refusal.contains("/store"), "{path_refusal}");
    assert!(path_refusal.contains("/another-store"), "{path_refusal}");

    let dimension_refusal = state
        .may_resume(
            &state.source_path,
            &state.source_fingerprint,
            TARGET_MODEL,
            TARGET_DIM + 1,
        )
        .expect_err("a journal must not change vector width");
    assert!(
        dimension_refusal.contains(&TARGET_DIM.to_string()),
        "{dimension_refusal}"
    );
    assert!(
        dimension_refusal.contains(&(TARGET_DIM + 1).to_string()),
        "{dimension_refusal}"
    );
}

#[test]
fn every_phase_has_an_explicit_recovery_action() {
    assert_eq!(
        PHASES.len(),
        5,
        "the five phases are Prepared, DestinationValidated, SourceArchived, \
         DestinationActivated, Committed"
    );

    for phase in PHASES {
        let recovery = phase.recovery();
        // Each action must carry a stated reason, not just a verdict: a bare
        // 'Continue' is something an operator has to trust rather than check.
        let stated = stated(&recovery);
        assert!(
            stated.len() > 40,
            "{phase:?} has an action with no usable explanation: {stated:?}"
        );
    }

    // The actions are not all the same — a `recovery()` that returned one
    // constant would satisfy the loop above while deciding nothing.
    let distinct: BTreeSet<String> = PHASES
        .iter()
        .map(|p| format!("{:?}", p.recovery()))
        .collect();
    assert_eq!(
        distinct.len(),
        PHASES.len(),
        "each phase must have its OWN action; identical ones mean the phase was \
         not actually considered"
    );

    // The two that matter most, named rather than inferred: the phase where the
    // source has been moved aside and nothing replaced it must RESTORE, and the
    // finished migration must refuse to run again.
    assert!(
        matches!(Phase::SourceArchived.recovery(), Recovery::Restore { .. }),
        "with the source archived and the destination not yet activated, the \
         source is the only authority and must go back"
    );
    assert!(
        matches!(Phase::Committed.recovery(), Recovery::Refuse { .. }),
        "a finished migration has nothing to resume"
    );
    assert!(
        matches!(
            Phase::DestinationActivated.recovery(),
            Recovery::Continue {
                next: Phase::Committed,
                ..
            }
        ),
        "once the destination is live, going BACK would discard the store the \
         caller is already reading from"
    );
}

#[test]
fn an_ambiguous_switch_state_changes_nothing() {
    let layouts = SwitchState::all();
    assert_eq!(
        layouts.len(),
        8,
        "three directories, present or not, is eight states — an enumeration \
         that missed one would leave a disk layout with no defined action"
    );
    assert_eq!(
        layouts
            .iter()
            .map(|l| (l.source, l.archive, l.destination))
            .collect::<BTreeSet<_>>()
            .len(),
        8,
        "the eight must be distinct"
    );
    for layout in &layouts {
        let recovery = layout.recovery();
        let stated = stated(&recovery);
        assert!(
            stated.len() > 40,
            "{layout:?} has no usable explanation: {stated:?}"
        );
    }

    // The layouts no sequence of this migration produces — or that two
    // different histories both produce — must REFUSE.
    for triple in AMBIGUOUS {
        let layout = switch(*triple);
        assert!(
            matches!(layout.recovery(), Recovery::Refuse { .. }),
            "{layout:?} does not determine what happened and must change nothing"
        );
    }

    // The positive control: "refuse everything" would satisfy the loop above
    // while making every interrupted migration unrecoverable.
    for triple in DECIDABLE {
        let layout = switch(*triple);
        assert!(
            !matches!(layout.recovery(), Recovery::Refuse { .. }),
            "{layout:?} DOES determine what happened; refusing it would strand a \
             recoverable migration"
        );
    }
}

#[test]
fn deciding_what_to_do_does_not_already_do_it() {
    // Separating the decision from the action is what keeps a WRONG decision
    // from having destroyed the evidence before anyone reads it. Every layout
    // is asked against a real directory holding all three names.
    let workspace = tempfile::tempdir().expect("tempdir");
    for name in ["store", "store.archive", "store.rebuilt"] {
        std::fs::create_dir(workspace.path().join(name)).expect("create");
        std::fs::write(workspace.path().join(name).join("data"), name).expect("write");
    }
    let before = tree(workspace.path());
    assert_eq!(before.len(), 3, "positive control: three files must exist");

    for layout in &SwitchState::all() {
        let _ = layout.recovery();
    }

    assert!(
        drift(&before, &tree(workspace.path())).is_empty(),
        "deciding must not delete, rename or create anything"
    );
}
