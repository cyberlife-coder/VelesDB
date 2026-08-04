use super::*;

fn state(phase: Phase) -> MigrationState {
    MigrationState {
        format_version: STATE_FORMAT_VERSION,
        phase,
        source_path: std::path::PathBuf::from("/store"),
        source_fingerprint: "sha256-tree-v2:0123456789abcdef".to_owned(),
        target_model: diagnosis::TARGET_MODEL.to_owned(),
        target_dimension: diagnosis::TARGET_DIM,
    }
}

fn lock(workspace: &std::path::Path) -> MigrationLock {
    MigrationLock::acquire(workspace, "state-persistence-test").expect("lock")
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
