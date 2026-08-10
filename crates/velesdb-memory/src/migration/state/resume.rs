//! The resume-identity validators of [`MigrationState`]: a journal may only
//! be resumed by the request it was prepared for — same source, same
//! fingerprint, same target model, same width. Each refusal names what
//! diverged and what to do about it. Split from `state.rs` as its own
//! concern (and to keep that file under the size gate).

use std::path::Path;

use super::MigrationState;

pub(super) fn validate_resume_source(
    state: &MigrationState,
    requested: &Path,
) -> Result<(), String> {
    if state.source_path == requested {
        return Ok(());
    }
    Err(format!(
        "this migration was prepared for source '{}' and the request names '{}'. A journal cannot be transferred between stores. Start a fresh diagnosis.",
        state.source_path.display(),
        requested.display()
    ))
}

pub(super) fn validate_resume_fingerprint(
    state: &MigrationState,
    requested: &str,
) -> Result<(), String> {
    if state.source_fingerprint == requested {
        return Ok(());
    }
    Err(format!(
        "the source changed since this migration was prepared: it was fingerprinted '{}' and is now '{}'. Resuming would rebuild from an inventory that no longer describes the store. Start a fresh diagnosis.",
        state.source_fingerprint, requested
    ))
}

pub(super) fn validate_resume_model(state: &MigrationState, requested: &str) -> Result<(), String> {
    if state.target_model == requested {
        return Ok(());
    }
    Err(format!(
        "this migration was prepared for the model '{}' and the request names '{}'. Half a store embedded by one model and half by another is not searchable. Either point the request back at '{}', or start a fresh migration.",
        state.target_model, requested, state.target_model
    ))
}

pub(super) fn validate_resume_dimension(
    state: &MigrationState,
    requested: usize,
) -> Result<(), String> {
    if state.target_dimension == requested {
        return Ok(());
    }
    Err(format!(
        "this migration was prepared for target dimension {} and the request names {}. Changing vector width mid-migration would make the rebuilt store unreadable. Start a fresh migration.",
        state.target_dimension, requested
    ))
}
