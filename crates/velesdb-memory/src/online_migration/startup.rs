use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::migration::{
    commit_retained_switch, finalize_staged_live_switch, rollback_staged_live_switch,
    target_embedder_witness, MigrationState, OnlineMigrationStartup, ARCHIVE_SUFFIX,
};
use crate::mutation::controller::{ControllerPhase, ConvergenceController};
use crate::mutation::journal::{CutoverIdentity, DirtyJournal};
use crate::MemoryError;

use super::job_state::{JobPhase, JobRecord, JobStore};

pub(crate) fn recover_startup<F>(
    source: &Path,
    target_factory: F,
) -> Result<OnlineMigrationStartup, MemoryError>
where
    F: Fn(&str) -> Result<(crate::DynEmbedder, String), MemoryError>,
{
    let Some((store, record)) = load_recovery_job(source)? else {
        return Ok(OnlineMigrationStartup::None);
    };
    recover_loaded(&store, record, target_factory)
}

fn load_recovery_job(source: &Path) -> Result<Option<(JobStore, JobRecord)>, MemoryError> {
    let control = sibling(source, "online-migration-control")?;
    if !path_exists(&control)? {
        return Ok(None);
    }
    let Some(store) = JobStore::try_open(&control)? else {
        return Ok(None);
    };
    let record = store.load()?;
    if !requires_startup_recovery(record.phase) {
        return Ok(None);
    }
    Ok(Some((store, record)))
}

fn recover_loaded<F>(
    store: &JobStore,
    mut record: JobRecord,
    target_factory: F,
) -> Result<OnlineMigrationStartup, MemoryError>
where
    F: Fn(&str) -> Result<(crate::DynEmbedder, String), MemoryError>,
{
    let journal = Arc::new(DirtyJournal::open(
        &record.spec.workspace,
        &record.spec.identity,
        record.spec.journal_max_bytes,
    )?);
    let mut controller = ConvergenceController::open(
        &record.spec.workspace,
        record.spec.identity.epoch_id(),
        record.spec.controller,
    )?;
    verify_durable_identity(&record, &journal)?;
    recover_phase(store, &mut record, &mut controller, target_factory)
}

fn recover_phase<F>(
    store: &JobStore,
    record: &mut JobRecord,
    controller: &mut ConvergenceController,
    target_factory: F,
) -> Result<OnlineMigrationStartup, MemoryError>
where
    F: Fn(&str) -> Result<(crate::DynEmbedder, String), MemoryError>,
{
    match controller.phase() {
        ControllerPhase::Quiescing { .. } => {
            recover_source(store, record, controller)?;
            Ok(OnlineMigrationStartup::SourceRestored {
                source_model: record.spec.identity.source_provenance().to_owned(),
            })
        }
        ControllerPhase::Activated => {
            let (embedder, model) = target_factory(&record.spec.target_backend)?;
            verify_target(record, embedder.as_ref(), &model)?;
            recover_target(store, record, controller)?;
            Ok(OnlineMigrationStartup::TargetActivated { embedder, model })
        }
        ControllerPhase::CatchingUp => {
            reconcile_source(store, record, controller)?;
            Ok(OnlineMigrationStartup::SourceRestored {
                source_model: record.spec.identity.source_provenance().to_owned(),
            })
        }
        _ => Err(capture(
            "cutover job and controller phases disagree at startup",
        )),
    }
}

fn requires_startup_recovery(phase: JobPhase) -> bool {
    matches!(
        phase,
        JobPhase::CutoverReady | JobPhase::Quiescing | JobPhase::Activated
    )
}

fn recover_source(
    store: &JobStore,
    record: &mut JobRecord,
    controller: &mut ConvergenceController,
) -> Result<(), MemoryError> {
    let identity = &record.spec.identity;
    let state = MigrationState::read(&record.spec.workspace).map_err(capture)?;
    if state.is_some() {
        rollback_staged_live_switch(identity.source_path(), identity.destination_path())?;
    } else {
        require_unmoved(identity.source_path(), identity.destination_path())?;
    }
    controller.complete_source_recovery()?;
    record.transition(JobPhase::CatchingUp)?;
    record.recovery_action = controller.recovery_action().map(str::to_owned);
    store.save(record)
}

fn recover_target(
    store: &JobStore,
    record: &mut JobRecord,
    controller: &mut ConvergenceController,
) -> Result<(), MemoryError> {
    let identity = &record.spec.identity;
    finalize_staged_live_switch(identity.source_path(), identity.destination_path())?;
    commit_retained_switch(identity.source_path(), identity.destination_path())?;
    controller.complete_target_recovery()?;
    reconcile_activated_record(record)?;
    record.progress.measured_cutover = controller.measured_cutover();
    record.transition(JobPhase::Committed)?;
    record.recovery_action = None;
    store.save(record)
}

fn reconcile_activated_record(record: &mut JobRecord) -> Result<(), MemoryError> {
    if record.phase == JobPhase::CutoverReady {
        record.transition(JobPhase::Quiescing)?;
    }
    if record.phase == JobPhase::Quiescing {
        record.transition(JobPhase::Activated)?;
    }
    Ok(())
}

fn reconcile_source(
    store: &JobStore,
    record: &mut JobRecord,
    controller: &ConvergenceController,
) -> Result<(), MemoryError> {
    if record.phase != JobPhase::CatchingUp {
        record.transition(JobPhase::CatchingUp)?;
    }
    record.recovery_action = controller.recovery_action().map(str::to_owned);
    store.save(record)
}

fn verify_durable_identity(record: &JobRecord, journal: &DirtyJournal) -> Result<(), MemoryError> {
    let identity = &record.spec.identity;
    journal.verify_cutover_identity(&CutoverIdentity {
        source: identity.source_path(),
        destination: identity.destination_path(),
        source_provenance: identity.source_provenance(),
        target_model: identity.target_model(),
        target_dimension: identity.target_dimension(),
        target_witness: identity.target_witness(),
        epoch_id: identity.epoch_id(),
    })
}

fn verify_target(
    record: &JobRecord,
    embedder: &dyn crate::Embedder,
    model: &str,
) -> Result<(), MemoryError> {
    let identity = &record.spec.identity;
    let witness = target_embedder_witness(embedder)?;
    if model != identity.target_model()
        || embedder.dimension() != identity.target_dimension()
        || witness != identity.target_witness()
    {
        return Err(capture(
            "migration target identity changed before startup recovery",
        ));
    }
    Ok(())
}

fn require_unmoved(source: &Path, destination: &Path) -> Result<(), MemoryError> {
    let archive = archive_path(source)?;
    if path_exists(source)? && path_exists(destination)? && !path_exists(&archive)? {
        return Ok(());
    }
    Err(capture(
        "cutover has no switch journal but its filesystem layout was moved",
    ))
}

fn archive_path(source: &Path) -> Result<PathBuf, MemoryError> {
    let name = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| capture("online migration source has no usable directory name"))?;
    Ok(source.with_file_name(format!("{name}{ARCHIVE_SUFFIX}")))
}

fn sibling(source: &Path, suffix: &str) -> Result<PathBuf, MemoryError> {
    let name = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| capture("online migration source has no usable directory name"))?;
    Ok(source.with_file_name(format!("{name}.{suffix}")))
}

fn path_exists(path: &Path) -> Result<bool, MemoryError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(capture(format!(
            "cannot inspect {}: {error}",
            path.display()
        ))),
    }
}

fn capture(message: impl Into<String>) -> MemoryError {
    MemoryError::MigrationCapture(message.into())
}
