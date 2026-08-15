use std::path::Path;

use super::LiveGenerationSlot;
use crate::embedder::Embedder;
use crate::migration::{
    commit_retained_switch, finalize_staged_live_switch, rollback_staged_live_switch,
    target_embedder_witness, ARCHIVE_SUFFIX,
};
use crate::mutation::controller::{ControllerPhase, ConvergenceController};
use crate::mutation::journal::{CutoverIdentity, DirtyJournal};
use crate::{MemoryError, MemoryService};

pub(crate) struct LiveRecovery<'a> {
    pub(crate) controller: &'a mut ConvergenceController,
    pub(crate) journal: &'a DirtyJournal,
    pub(crate) source: &'a Path,
    pub(crate) destination: &'a Path,
    pub(crate) source_model: &'a str,
    pub(crate) target_model: &'a str,
}

impl<E: Embedder> LiveGenerationSlot<E> {
    pub(crate) fn recover(
        mut recovery: LiveRecovery<'_>,
        source_embedder: E,
        target_embedder: E,
    ) -> Result<Self, MemoryError> {
        verify_identity(&recovery, &target_embedder)?;
        match recovery.controller.phase() {
            ControllerPhase::Quiescing { .. } => recover_source(&mut recovery, source_embedder),
            ControllerPhase::Activated => recover_target(&mut recovery, target_embedder),
            _ => Err(super::unavailable(
                "controller does not require live cutover recovery",
            )),
        }
    }
}

fn verify_identity<E: Embedder>(
    recovery: &LiveRecovery<'_>,
    target_embedder: &E,
) -> Result<(), MemoryError> {
    let witness = target_embedder_witness(target_embedder)?;
    recovery.journal.verify_cutover_identity(&CutoverIdentity {
        source: recovery.source,
        destination: recovery.destination,
        source_provenance: recovery.source_model,
        target_model: recovery.target_model,
        target_dimension: target_embedder.dimension(),
        target_witness: &witness,
        epoch_id: recovery.controller.epoch_id(),
    })
}

fn recover_source<E: Embedder>(
    recovery: &mut LiveRecovery<'_>,
    source_embedder: E,
) -> Result<LiveGenerationSlot<E>, MemoryError> {
    let archive = recovery.source.with_file_name(format!(
        "{}{}",
        recovery
            .source
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| super::unavailable("source has no usable directory name"))?,
        ARCHIVE_SUFFIX
    ));
    if archive.exists() || !recovery.destination.exists() {
        rollback_staged_live_switch(recovery.source, recovery.destination)?;
    }
    let service = MemoryService::open(recovery.source, source_embedder)?;
    recovery.controller.complete_source_recovery()?;
    Ok(LiveGenerationSlot::new(service, recovery.source_model))
}

fn recover_target<E: Embedder>(
    recovery: &mut LiveRecovery<'_>,
    target_embedder: E,
) -> Result<LiveGenerationSlot<E>, MemoryError> {
    let service = MemoryService::open(recovery.source, target_embedder)?;
    finalize_staged_live_switch(recovery.source, recovery.destination)?;
    commit_retained_switch(recovery.source, recovery.destination)?;
    recovery.controller.complete_target_recovery()?;
    Ok(LiveGenerationSlot::new(service, recovery.target_model))
}
