use std::path::Path;

use crate::embedding_provenance::{self, EmbeddingProvenance};
use crate::mutation::journal::{CutoverIdentity, DirtyJournal};
use crate::MemoryError;

use super::{JobPhase, JobRecord};

pub(crate) fn remove_cancelled_artifacts(record: &JobRecord) -> Result<(), MemoryError> {
    require_cancelled(record)?;
    let identity = &record.spec.identity;
    verify_workspace_path(identity.destination_path(), &record.spec.workspace)?;
    let destination_exists = real_directory_if_present(identity.destination_path())?;
    let workspace_exists = real_directory_if_present(&record.spec.workspace)?;
    cleanup_present(record, destination_exists, workspace_exists)
}

fn require_cancelled(record: &JobRecord) -> Result<(), MemoryError> {
    if record.phase == JobPhase::Cancelled {
        return Ok(());
    }
    Err(capture(
        "only a cancelled migration may remove its artifacts",
    ))
}

fn cleanup_present(
    record: &JobRecord,
    destination_exists: bool,
    workspace_exists: bool,
) -> Result<(), MemoryError> {
    match (destination_exists, workspace_exists) {
        (false, false) => Ok(()),
        (true, false) => Err(capture("cancelled destination has no epoch journal")),
        (destination_exists, true) => cleanup_verified(record, destination_exists),
    }
}

fn cleanup_verified(record: &JobRecord, destination_exists: bool) -> Result<(), MemoryError> {
    let identity = &record.spec.identity;
    let journal = DirtyJournal::open(
        &record.spec.workspace,
        identity,
        record.spec.journal_max_bytes,
    )?;
    journal.verify_cutover_identity(&cutover_identity(record))?;
    drop(journal);
    if destination_exists {
        verify_target_provenance(record)?;
        remove_real_directory(identity.destination_path())?;
    }
    remove_real_directory(&record.spec.workspace)
}

fn cutover_identity(record: &JobRecord) -> CutoverIdentity<'_> {
    let identity = &record.spec.identity;
    CutoverIdentity {
        source: identity.source_path(),
        destination: identity.destination_path(),
        source_provenance: identity.source_provenance(),
        target_model: identity.target_model(),
        target_dimension: identity.target_dimension(),
        target_witness: identity.target_witness(),
        epoch_id: identity.epoch_id(),
    }
}

fn verify_workspace_path(destination: &Path, workspace: &Path) -> Result<(), MemoryError> {
    let name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| capture("cancelled destination has no usable directory name"))?;
    let expected = destination.with_file_name(format!("{name}.migration-journal"));
    if workspace == expected {
        return Ok(());
    }
    Err(capture(
        "cancelled migration workspace path is not derived from its destination",
    ))
}

fn verify_target_provenance(record: &JobRecord) -> Result<(), MemoryError> {
    let identity = &record.spec.identity;
    let expected = EmbeddingProvenance::new(identity.target_model(), identity.target_dimension());
    let actual = embedding_provenance::read(identity.destination_path()).map_err(capture)?;
    if actual.as_ref() == Some(&expected) {
        return Ok(());
    }
    Err(capture(
        "cancelled destination provenance does not match its epoch",
    ))
}

fn real_directory_if_present(path: &Path) -> Result<bool, MemoryError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => Ok(true),
        Ok(_) => Err(capture(format!(
            "migration artifact is not a real directory: {}",
            path.display()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(capture(format!(
            "cannot inspect migration artifact: {error}"
        ))),
    }
}

fn remove_real_directory(path: &Path) -> Result<(), MemoryError> {
    std::fs::remove_dir_all(path)
        .map_err(|error| capture(format!("cannot remove {}: {error}", path.display())))?;
    sync_parent(path)
}

fn sync_parent(path: &Path) -> Result<(), MemoryError> {
    let parent = path
        .parent()
        .ok_or_else(|| capture("migration artifact has no parent directory"))?;
    sync_directory(parent).map_err(|error| capture(format!("cannot sync artifact parent: {error}")))
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> std::io::Result<()> {
    std::fs::File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

fn capture(message: impl Into<String>) -> MemoryError {
    MemoryError::MigrationCapture(message.into())
}
