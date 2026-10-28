//! Who can fix a refused journal step is decided where it is refused.
//!
//! The state layer used to return a bare message and let each caller guess a
//! class for it, so a journal an operator had only to resume with the right
//! model was reported to every client as a server fault. These tests pin the
//! class of each cause at its source, and that `MemoryError::category` carries
//! it to the adapters unchanged.

use super::*;
use crate::{ErrorCategory, MemoryError};

const FINGERPRINT: &str =
    "sha256-tree-v2:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn prepared() -> MigrationState {
    MigrationState {
        format_version: STATE_FORMAT_VERSION,
        phase: Phase::Prepared,
        source_path: std::path::PathBuf::from("/store"),
        source_fingerprint: FINGERPRINT.to_owned(),
        target_model: diagnosis::TARGET_MODEL.to_owned(),
        target_dimension: diagnosis::TARGET_DIM,
        embedder_witness: None,
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

fn category_of(error: MigrationError) -> ErrorCategory {
    MemoryError::from(error).category()
}

#[test]
fn a_held_lock_is_the_callers_to_wait_out() {
    let workspace = tempfile::tempdir().expect("workspace");
    let _held = MigrationLock::acquire(workspace.path(), "run-A").expect("first lock");

    let error = MigrationLock::acquire(workspace.path(), "run-B").expect_err("second lock");

    assert!(matches!(error, MigrationError::Precondition(_)), "{error}");
    assert_eq!(category_of(error), ErrorCategory::InvalidInput);
}

#[test]
fn a_state_from_another_version_is_refused_as_a_precondition() {
    let workspace = tempfile::tempdir().expect("workspace");
    let mut newer = serde_json::to_value(prepared()).expect("json");
    newer["format_version"] = serde_json::json!(STATE_FORMAT_VERSION + 1);
    std::fs::write(workspace.path().join(STATE_FILE), newer.to_string()).expect("write");

    let error = MigrationState::read(workspace.path()).expect_err("newer version");

    assert!(matches!(error, MigrationError::Precondition(_)), "{error}");
}

#[test]
fn a_journal_that_is_not_json_contradicts_itself() {
    let workspace = tempfile::tempdir().expect("workspace");
    std::fs::write(workspace.path().join(STATE_FILE), "{ not json").expect("write");

    let error = MigrationState::read(workspace.path()).expect_err("corrupt journal");

    assert!(matches!(error, MigrationError::Integrity(_)), "{error}");
    assert_eq!(category_of(error), ErrorCategory::Internal);
}

#[test]
fn a_journal_that_lost_a_collection_is_an_integrity_fault() {
    let mut state = prepared();
    state.progress.remove(AGENT_COLLECTIONS[0]);

    let error = state
        .may_resume(
            &state.source_path,
            FINGERPRINT,
            &state.target_model,
            state.target_dimension,
        )
        .expect_err("progress missing a collection");

    assert!(matches!(error, MigrationError::Integrity(_)), "{error}");
}

#[test]
fn resuming_with_another_model_is_a_request_the_caller_can_correct() {
    let state = prepared();

    let error = state
        .may_resume(
            &state.source_path,
            FINGERPRINT,
            "another-model",
            state.target_dimension,
        )
        .expect_err("model drift");

    assert!(matches!(error, MigrationError::Precondition(_)), "{error}");
    assert_eq!(category_of(error), ErrorCategory::InvalidInput);
}

#[test]
fn a_source_that_changed_since_the_journal_is_an_integrity_fault() {
    let state = prepared();
    let changed = format!("sha256-tree-v2:{}", "f".repeat(64));

    let error = state
        .may_resume(
            &state.source_path,
            &changed,
            &state.target_model,
            state.target_dimension,
        )
        .expect_err("fingerprint drift");

    assert!(matches!(error, MigrationError::Integrity(_)), "{error}");
}

#[test]
fn an_invalid_requested_identity_is_the_callers_error_not_the_journals() {
    let state = prepared();

    let error = state
        .may_resume(
            &state.source_path,
            "weak-fingerprint",
            &state.target_model,
            state.target_dimension,
        )
        .expect_err("malformed request");

    assert!(matches!(error, MigrationError::Precondition(_)), "{error}");
}

#[test]
fn writing_without_the_matching_lock_is_a_precondition() {
    let workspace = tempfile::tempdir().expect("workspace");
    let other = tempfile::tempdir().expect("other workspace");
    let lock = MigrationLock::acquire(other.path(), "elsewhere").expect("lock");

    let error = prepared()
        .write(workspace.path(), &lock)
        .expect_err("wrong lock");

    assert!(matches!(error, MigrationError::Precondition(_)), "{error}");
}

#[test]
fn a_refused_capture_is_a_server_fault_that_names_its_cause() {
    let error = crate::migration::capture("a mutation observer is already active");

    assert_eq!(error.category(), ErrorCategory::Internal);
    assert!(
        matches!(error, MemoryError::Migration(MigrationError::Capture(_))),
        "{error:?}"
    );
    assert_eq!(
        error.to_string(),
        "migration error: a mutation observer is already active"
    );
}
