use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::{
    execute::journal_workspace, fingerprint, query_error, CollectionProgress, MigrationLock,
    MigrationState, Phase, AGENT_COLLECTIONS, STATE_FORMAT_VERSION,
};

pub(crate) fn prepare_live_switch(
    source: &Path,
    destination: &Path,
    target_model: &str,
    target_dimension: usize,
    target_witness: &str,
) -> Result<PathBuf, crate::MemoryError> {
    verify_target_provenance(destination, target_model, target_dimension)?;
    let source = source
        .canonicalize()
        .map_err(|err| query_error(format!("cannot resolve live source: {err}")))?;
    let source_fingerprint = fingerprint(&source)?;
    let workspace = journal_workspace(destination)?;
    let lock = MigrationLock::acquire(&workspace, "migrate-live-prepare").map_err(query_error)?;
    let result = prepare_locked(
        &workspace,
        &lock,
        source,
        source_fingerprint,
        target_model,
        target_dimension,
        target_witness,
    );
    super::execute::reconcile(result, lock.release())?;
    Ok(workspace)
}

fn prepare_locked(
    workspace: &Path,
    lock: &MigrationLock,
    source_path: PathBuf,
    source_fingerprint: String,
    target_model: &str,
    target_dimension: usize,
    target_witness: &str,
) -> Result<(), crate::MemoryError> {
    if let Some(existing) = MigrationState::read(workspace).map_err(query_error)? {
        existing
            .may_resume(
                &source_path,
                &source_fingerprint,
                target_model,
                target_dimension,
            )
            .map_err(query_error)?;
        if existing.embedder_witness.as_deref() != Some(target_witness) {
            return Err(query_error(
                "live switch target embedder witness changed; start a fresh migration",
            ));
        }
        return require_validated(existing.phase);
    }
    let mut state = new_state(
        source_path,
        source_fingerprint,
        target_model,
        target_dimension,
        target_witness,
    );
    state.write(workspace, lock).map_err(query_error)?;
    state.phase = Phase::DestinationValidated;
    state.write(workspace, lock).map_err(query_error)
}

fn new_state(
    source_path: PathBuf,
    source_fingerprint: String,
    target_model: &str,
    target_dimension: usize,
    target_witness: &str,
) -> MigrationState {
    let progress = AGENT_COLLECTIONS
        .iter()
        .map(|name| ((*name).to_owned(), CollectionProgress::Complete))
        .collect::<BTreeMap<_, _>>();
    MigrationState {
        format_version: STATE_FORMAT_VERSION,
        phase: Phase::Prepared,
        source_path,
        source_fingerprint,
        target_model: target_model.to_owned(),
        target_dimension,
        progress,
        embedder_witness: Some(target_witness.to_owned()),
    }
}

fn require_validated(phase: Phase) -> Result<(), crate::MemoryError> {
    if phase == Phase::DestinationValidated {
        return Ok(());
    }
    Err(query_error(format!(
        "live switch preparation found journal phase {phase:?}; recovery must finish first"
    )))
}

fn verify_target_provenance(
    destination: &Path,
    target_model: &str,
    target_dimension: usize,
) -> Result<(), crate::MemoryError> {
    let recorded = crate::embedding_provenance::read(destination).map_err(query_error)?;
    crate::embedding_provenance::check(recorded.as_ref(), target_model, target_dimension)
        .map_err(query_error)
}
