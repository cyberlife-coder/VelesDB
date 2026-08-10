use super::*;

pub(super) const VALID_FINGERPRINT: &str =
    "sha256-tree-v2:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn state(phase: Phase) -> MigrationState {
    MigrationState {
        format_version: STATE_FORMAT_VERSION,
        phase,
        source_path: std::path::PathBuf::from("/store"),
        source_fingerprint: VALID_FINGERPRINT.to_owned(),
        target_model: diagnosis::TARGET_MODEL.to_owned(),
        target_dimension: diagnosis::TARGET_DIM,
        embedder_witness: None,
        // Complete rather than fresh, because this file's tests advance the
        // PHASE — and a phase past Prepared with an unfinished rebuild is now
        // itself a semantics refusal, which would shadow what each test is
        // actually about.
        progress: AGENT_COLLECTIONS
            .iter()
            .map(|name| ((*name).to_owned(), CollectionProgress::Complete))
            .collect(),
    }
}

fn lock(workspace: &std::path::Path) -> MigrationLock {
    MigrationLock::acquire(workspace, "state-persistence-test").expect("lock")
}

fn semantically_invalid_states() -> Vec<(&'static str, MigrationState)> {
    let baseline = state(Phase::Prepared);
    let mut empty_path = baseline.clone();
    empty_path.source_path = std::path::PathBuf::new();
    let mut relative_path = baseline.clone();
    relative_path.source_path = std::path::PathBuf::from("relative/store");
    let mut parent_path = baseline.clone();
    parent_path.source_path = std::path::PathBuf::from("/stores/../other-store");
    let mut wrong_fingerprint_prefix = baseline.clone();
    wrong_fingerprint_prefix.source_fingerprint =
        "sha256-tree-v1:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            .to_owned();
    let mut short_fingerprint = baseline.clone();
    short_fingerprint.source_fingerprint = "sha256-tree-v2:0123".to_owned();
    let mut uppercase_fingerprint = baseline.clone();
    uppercase_fingerprint.source_fingerprint =
        "sha256-tree-v2:ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
            .to_owned();
    let mut empty_model = baseline.clone();
    empty_model.target_model = "   ".to_owned();
    let mut zero_dimension = baseline;
    zero_dimension.target_dimension = 0;

    vec![
        ("source_path", empty_path),
        ("source_path", relative_path),
        ("source_path", parent_path),
        ("source_fingerprint", wrong_fingerprint_prefix),
        ("source_fingerprint", short_fingerprint),
        ("source_fingerprint", uppercase_fingerprint),
        ("target_model", empty_model),
        ("target_dimension", zero_dimension),
    ]
}

#[test]
fn state_write_round_trips_replaces_and_leaves_no_staging_file() {
    let workspace = tempfile::tempdir().expect("workspace");
    let lock = lock(workspace.path());

    state(Phase::Prepared)
        .write(workspace.path(), &lock)
        .expect("first write");
    assert_eq!(
        MigrationState::read(workspace.path())
            .expect("read first")
            .expect("first exists")
            .phase,
        Phase::Prepared
    );
    assert!(!workspace.path().join(STATE_TEMP_FILE).exists());

    state(Phase::DestinationValidated)
        .write(workspace.path(), &lock)
        .expect("replace valid state");
    assert_eq!(
        MigrationState::read(workspace.path())
            .expect("read replacement")
            .expect("replacement exists")
            .phase,
        Phase::DestinationValidated
    );
    assert!(!workspace.path().join(STATE_TEMP_FILE).exists());
    lock.release().expect("release");
}

#[test]
fn writing_without_the_exact_workspace_lock_is_refused() {
    let workspace = tempfile::tempdir().expect("workspace");
    let other = tempfile::tempdir().expect("other workspace");
    let wrong_lock = lock(other.path());

    let error = state(Phase::Prepared)
        .write(workspace.path(), &wrong_lock)
        .expect_err("wrong lock must refuse");
    assert!(error.contains("exact workspace"), "{error}");
    assert!(!workspace.path().join(STATE_FILE).exists());
    assert!(!workspace.path().join(STATE_TEMP_FILE).exists());
    wrong_lock.release().expect("release");
}

#[test]
fn live_os_guard_blocks_reacquisition_after_canonical_lock_deletion() {
    let workspace = tempfile::tempdir().expect("workspace");
    let active = MigrationLock::acquire(workspace.path(), "run-A").expect("first lock");
    let first_record: serde_json::Value = serde_json::from_slice(
        &std::fs::read(workspace.path().join(LOCK_FILE)).expect("A lock bytes"),
    )
    .expect("A lock record");
    let first_token = first_record["token"].as_str().expect("persisted A token");
    assert!(!first_token.is_empty(), "the lock token must be persisted");

    // Delete the human-readable record while A remains live. The persistent
    // sibling's OS lock must still close the delete/recreate ABA window.
    std::fs::remove_file(workspace.path().join(LOCK_FILE)).expect("remove A lock");
    let blocked = MigrationLock::acquire(workspace.path(), "run-B")
        .expect_err("B must not pass A's still-live OS guard");
    assert!(blocked.contains("workspace guard"), "{blocked}");
    assert!(!workspace.path().join(LOCK_FILE).exists());

    let write_error = state(Phase::Prepared)
        .write(workspace.path(), &active)
        .expect_err("A must stop when its canonical identity disappears");
    assert!(write_error.contains("lock identity"), "{write_error}");
    assert!(!workspace.path().join(STATE_FILE).exists());
    assert!(!workspace.path().join(STATE_TEMP_FILE).exists());

    let release_error = active
        .release()
        .expect_err("release must refuse an absent canonical identity");
    assert!(
        release_error.contains("later acquisition"),
        "{release_error}"
    );

    // The failed consuming release drops A's OS handle. Since the canonical
    // record is already absent, B can now create a fresh identity.
    let current = MigrationLock::acquire(workspace.path(), "run-B").expect("fresh lock B");
    let replacement: serde_json::Value = serde_json::from_slice(
        &std::fs::read(workspace.path().join(LOCK_FILE)).expect("B lock bytes"),
    )
    .expect("B lock record");
    assert_ne!(
        replacement["token"].as_str().expect("persisted B token"),
        first_token,
        "every acquisition must carry a distinct persisted identity"
    );
    state(Phase::Prepared)
        .write(workspace.path(), &current)
        .expect("B remains the usable owner");
    current.release().expect("release B");
}

#[test]
fn erroneous_release_leaves_canonical_evidence_for_manual_cleanup() {
    let workspace = tempfile::tempdir().expect("workspace");
    let lock = MigrationLock::acquire(workspace.path(), "run-A").expect("first lock");
    let tampered = b"operator evidence with no valid token";
    std::fs::write(workspace.path().join(LOCK_FILE), tampered).expect("tamper canonical lock");

    let error = lock
        .release()
        .expect_err("release must refuse a changed canonical identity");
    assert!(error.contains("invalid"), "{error}");
    assert_eq!(
        std::fs::read(workspace.path().join(LOCK_FILE)).expect("evidence remains"),
        tampered,
        "an erroneous release must fail closed"
    );

    let stale = MigrationLock::acquire(workspace.path(), "run-B")
        .expect_err("the stale canonical record must still block after A drops");
    assert!(stale.contains("lock record remains"), "{stale}");

    std::fs::remove_file(workspace.path().join(LOCK_FILE)).expect("manual cleanup");
    let current = MigrationLock::acquire(workspace.path(), "run-B").expect("lock after cleanup");
    state(Phase::Prepared)
        .write(workspace.path(), &current)
        .expect("B owns the clean workspace");
    current.release().expect("release B");
}

#[test]
fn ordinary_drop_and_panic_leave_the_canonical_lock_fail_closed() {
    let dropped_workspace = tempfile::tempdir().expect("drop workspace");
    let dropped = MigrationLock::acquire(dropped_workspace.path(), "drop-run").expect("lock");
    drop(dropped);
    assert!(
        dropped_workspace.path().join(LOCK_FILE).exists(),
        "Drop must never remove canonical evidence"
    );
    let refusal = MigrationLock::acquire(dropped_workspace.path(), "after-drop")
        .expect_err("a dropped handle must leave a stale canonical lock");
    assert!(refusal.contains("lock record remains"), "{refusal}");
    std::fs::remove_file(dropped_workspace.path().join(LOCK_FILE)).expect("manual drop cleanup");
    MigrationLock::acquire(dropped_workspace.path(), "after-cleanup")
        .expect("manual cleanup restores acquisition")
        .release()
        .expect("release after cleanup");

    let panic_workspace = tempfile::tempdir().expect("panic workspace");
    let panic_result = std::panic::catch_unwind(|| {
        let _lock = MigrationLock::acquire(panic_workspace.path(), "panic-run").expect("lock");
        panic!("injected migration panic");
    });
    assert!(
        panic_result.is_err(),
        "positive control: the closure panicked"
    );
    assert!(
        panic_workspace.path().join(LOCK_FILE).exists(),
        "unwinding must leave canonical evidence"
    );
    let refusal = MigrationLock::acquire(panic_workspace.path(), "after-panic")
        .expect_err("panic evidence must block a later acquisition");
    assert!(refusal.contains("lock record remains"), "{refusal}");
}

#[test]
fn state_write_refuses_non_current_versions_before_creating_files() {
    for format_version in [STATE_FORMAT_VERSION - 1, STATE_FORMAT_VERSION + 1] {
        let workspace = tempfile::tempdir().expect("workspace");
        let lock = lock(workspace.path());
        let mut candidate = state(Phase::Prepared);
        candidate.format_version = format_version;

        let error = candidate
            .write(workspace.path(), &lock)
            .expect_err("only the current state version may be persisted");
        assert!(
            error.contains(&format!("version {format_version}")),
            "{error}"
        );
        assert!(!workspace.path().join(STATE_FILE).exists());
        assert!(!workspace.path().join(STATE_TEMP_FILE).exists());
        lock.release().expect("release");
    }
}

#[test]
fn read_refuses_semantically_invalid_state_without_mutation() {
    for (field, candidate) in semantically_invalid_states() {
        let workspace = tempfile::tempdir().expect("workspace");
        let bytes = serde_json::to_vec_pretty(&candidate).expect("invalid state JSON");
        std::fs::write(workspace.path().join(STATE_FILE), &bytes).expect("plant invalid state");

        let error = MigrationState::read(workspace.path())
            .expect_err("read must reject semantically invalid current-format state");
        assert!(error.contains(field), "{field} refusal was: {error}");
        assert_eq!(
            std::fs::read(workspace.path().join(STATE_FILE)).expect("state evidence"),
            bytes,
            "read must not rewrite invalid {field} evidence"
        );
        assert!(!workspace.path().join(STATE_TEMP_FILE).exists());
    }
}

#[test]
fn write_refuses_semantically_invalid_state_before_staging() {
    for (field, candidate) in semantically_invalid_states() {
        let workspace = tempfile::tempdir().expect("workspace");
        let lock = lock(workspace.path());

        let error = candidate
            .write(workspace.path(), &lock)
            .expect_err("write must reject semantically invalid state");
        assert!(error.contains(field), "{field} refusal was: {error}");
        assert!(!workspace.path().join(STATE_FILE).exists());
        assert!(!workspace.path().join(STATE_TEMP_FILE).exists());
        lock.release().expect("release");
    }
}

#[test]
fn may_resume_refuses_semantically_invalid_in_memory_state() {
    for (field, candidate) in semantically_invalid_states() {
        let error = candidate
            .may_resume(
                std::path::Path::new("/store"),
                VALID_FINGERPRINT,
                diagnosis::TARGET_MODEL,
                diagnosis::TARGET_DIM,
            )
            .expect_err("may_resume must reject semantically invalid state");
        assert!(error.contains(field), "{field} refusal was: {error}");
    }
}

#[test]
fn state_write_keeps_the_migration_identity_immutable() {
    let workspace = tempfile::tempdir().expect("workspace");
    let lock = lock(workspace.path());
    let baseline = state(Phase::Prepared);
    baseline
        .write(workspace.path(), &lock)
        .expect("baseline state");
    let baseline_bytes = std::fs::read(workspace.path().join(STATE_FILE)).expect("baseline bytes");

    let mut changed_path = baseline.clone();
    changed_path.source_path = "/different-store".into();
    let mut changed_fingerprint = baseline.clone();
    changed_fingerprint.source_fingerprint =
        "sha256-tree-v2:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
            .to_owned();
    let mut changed_model = baseline.clone();
    changed_model.target_model = "different-model".to_owned();
    let mut changed_dimension = baseline.clone();
    changed_dimension.target_dimension += 1;

    for (field, candidate) in [
        ("source_path", changed_path),
        ("source_fingerprint", changed_fingerprint),
        ("target_model", changed_model),
        ("target_dimension", changed_dimension),
    ] {
        let error = candidate
            .write(workspace.path(), &lock)
            .expect_err("migration identity drift must be refused");
        assert!(error.contains(field), "{field} refusal was: {error}");
        assert_eq!(
            std::fs::read(workspace.path().join(STATE_FILE)).expect("state retained"),
            baseline_bytes,
            "{field} drift must be refused before the durable state changes"
        );
        assert!(!workspace.path().join(STATE_TEMP_FILE).exists());
    }
    lock.release().expect("release");
}

#[test]
fn state_write_allows_only_idempotence_or_one_phase_forward() {
    for (from_index, from) in PHASES.iter().copied().enumerate() {
        for (to_index, to) in PHASES.iter().copied().enumerate() {
            let workspace = tempfile::tempdir().expect("workspace");
            let lock = lock(workspace.path());
            let baseline = state(from);
            std::fs::write(
                workspace.path().join(STATE_FILE),
                serde_json::to_vec_pretty(&baseline).expect("baseline JSON"),
            )
            .expect("baseline state");
            let baseline_bytes =
                std::fs::read(workspace.path().join(STATE_FILE)).expect("baseline bytes");

            let result = state(to).write(workspace.path(), &lock);
            let allowed = to_index == from_index || to_index == from_index + 1;
            if allowed {
                result.unwrap_or_else(|error| {
                    panic!("{from:?} -> {to:?} should be permitted: {error}")
                });
            } else {
                let error = result.expect_err("phase regression or skip must be refused");
                assert!(error.contains("phase transition"), "{error}");
                assert_eq!(
                    std::fs::read(workspace.path().join(STATE_FILE)).expect("state retained"),
                    baseline_bytes,
                    "{from:?} -> {to:?} must be refused before mutation"
                );
                assert!(!workspace.path().join(STATE_TEMP_FILE).exists());
            }
            lock.release().expect("release");
        }
    }
}

#[test]
fn a_new_journal_must_start_prepared() {
    let workspace = tempfile::tempdir().expect("workspace");
    let lock = lock(workspace.path());

    let error = state(Phase::DestinationValidated)
        .write(workspace.path(), &lock)
        .expect_err("a fresh journal must not skip its prepared phase");
    assert!(error.contains("must start"), "{error}");
    assert!(!workspace.path().join(STATE_FILE).exists());
    assert!(!workspace.path().join(STATE_TEMP_FILE).exists());
    lock.release().expect("release");
}

#[test]
fn preexisting_staging_is_refused_and_preserved_byte_for_byte() {
    let workspace = tempfile::tempdir().expect("workspace");
    let lock = lock(workspace.path());
    state(Phase::Prepared)
        .write(workspace.path(), &lock)
        .expect("baseline");
    let final_before = std::fs::read(workspace.path().join(STATE_FILE)).expect("final before");
    let staged_before = b"evidence from interrupted writer";
    std::fs::write(workspace.path().join(STATE_TEMP_FILE), staged_before).expect("stale staging");

    let error = state(Phase::DestinationValidated)
        .write(workspace.path(), &lock)
        .expect_err("ambiguous staging must refuse");
    assert!(error.contains("interrupted state write"), "{error}");
    assert_eq!(
        std::fs::read(workspace.path().join(STATE_FILE)).expect("final after"),
        final_before
    );
    assert_eq!(
        std::fs::read(workspace.path().join(STATE_TEMP_FILE)).expect("staging after"),
        staged_before
    );

    std::fs::remove_file(workspace.path().join(STATE_TEMP_FILE)).expect("manual cleanup");
    state(Phase::DestinationValidated)
        .write(workspace.path(), &lock)
        .expect("positive control after explicit cleanup");
    lock.release().expect("release");
}

#[test]
fn a_corrupt_existing_state_is_refused_without_overwriting_evidence() {
    let workspace = tempfile::tempdir().expect("workspace");
    let lock = lock(workspace.path());
    let corrupt = b"{not valid migration state";
    std::fs::write(workspace.path().join(STATE_FILE), corrupt).expect("corrupt state");

    let error = state(Phase::Prepared)
        .write(workspace.path(), &lock)
        .expect_err("corrupt state must refuse");
    assert!(error.contains("not readable JSON"), "{error}");
    assert_eq!(
        std::fs::read(workspace.path().join(STATE_FILE)).expect("corrupt retained"),
        corrupt
    );
    assert!(!workspace.path().join(STATE_TEMP_FILE).exists());

    std::fs::remove_file(workspace.path().join(STATE_FILE)).expect("manual evidence cleanup");
    state(Phase::Prepared)
        .write(workspace.path(), &lock)
        .expect("positive control after explicit cleanup");
    lock.release().expect("release");
}

#[cfg(unix)]
#[test]
fn symlink_and_directory_destinations_are_refused_without_touching_them() {
    use std::os::unix::fs::symlink;

    let workspace = tempfile::tempdir().expect("workspace");
    let outside = tempfile::NamedTempFile::new().expect("outside");
    std::fs::write(outside.path(), b"outside sentinel").expect("outside content");
    let final_path = workspace.path().join(STATE_FILE);
    symlink(outside.path(), &final_path).expect("state symlink");
    let lock = lock(workspace.path());

    let error = state(Phase::Prepared)
        .write(workspace.path(), &lock)
        .expect_err("symlink destination must refuse");
    assert!(error.contains("symlink"), "{error}");
    assert_eq!(
        std::fs::read(outside.path()).expect("outside intact"),
        b"outside sentinel"
    );
    std::fs::remove_file(&final_path).expect("remove symlink");
    std::fs::create_dir(&final_path).expect("directory obstacle");

    let error = state(Phase::Prepared)
        .write(workspace.path(), &lock)
        .expect_err("directory destination must refuse");
    assert!(error.contains("directory"), "{error}");
    assert!(final_path.is_dir(), "directory obstacle must remain");
    lock.release().expect("release");
}

#[test]
fn promotion_failure_keeps_old_state_and_cleans_owned_staging() {
    let workspace = tempfile::tempdir().expect("workspace");
    let old = b"old durable state";
    std::fs::write(workspace.path().join(STATE_FILE), old).expect("old state");
    let fail_promote = |_temporary: &std::path::Path, _final_path: &std::path::Path| {
        Err(std::io::Error::other("injected promotion failure"))
    };
    let barrier = |_workspace: &std::path::Path, _final_path: &std::path::Path| Ok(());

    let error = super::super::state::commit_state_with(
        workspace.path(),
        b"new state",
        fail_promote,
        barrier,
    )
    .expect_err("promotion failure must surface");
    assert!(error.contains("injected promotion failure"), "{error}");
    assert_eq!(
        std::fs::read(workspace.path().join(STATE_FILE)).expect("old intact"),
        old
    );
    assert!(!workspace.path().join(STATE_TEMP_FILE).exists());
}

#[test]
fn barrier_failure_reports_visible_new_state_without_rollback() {
    let workspace = tempfile::tempdir().expect("workspace");
    std::fs::write(workspace.path().join(STATE_FILE), b"old state").expect("old state");
    let promote = |temporary: &std::path::Path, final_path: &std::path::Path| {
        std::fs::rename(temporary, final_path)
    };
    let fail_barrier = |_workspace: &std::path::Path, _final_path: &std::path::Path| {
        Err(std::io::Error::other("injected barrier failure"))
    };

    let error = super::super::state::commit_state_with(
        workspace.path(),
        b"new visible state",
        promote,
        fail_barrier,
    )
    .expect_err("barrier failure must surface");
    assert!(error.contains("visible"), "{error}");
    assert!(error.contains("Do not retry blindly"), "{error}");
    assert_eq!(
        std::fs::read(workspace.path().join(STATE_FILE)).expect("new visible"),
        b"new visible state"
    );
    assert!(!workspace.path().join(STATE_TEMP_FILE).exists());
}
