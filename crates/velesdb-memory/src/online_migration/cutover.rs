use std::path::Path;
use std::time::Duration;

use super::{ActiveGeneration, LiveGenerationSlot};
use crate::embedder::Embedder;
use crate::migration::{
    commit_retained_switch, finalize_staged_live_switch, prepare_live_switch,
    rollback_staged_live_switch, stage_live_switch,
};
use crate::mutation::controller::ConvergenceController;
use crate::mutation::journal::{CutoverIdentity, DirtyJournal};
use crate::storage::NativeStore;
use crate::{MemoryError, MemoryService};

pub(crate) struct LiveCutover<'a> {
    pub(crate) controller: &'a mut ConvergenceController,
    pub(crate) journal: &'a DirtyJournal,
    pub(crate) source: &'a Path,
    pub(crate) destination: &'a Path,
    pub(crate) target_model: &'a str,
    pub(crate) started_at: Duration,
    pub(crate) completed_at: Duration,
}

struct RetiredGeneration<E: Embedder> {
    embedder: E,
    model: String,
    runtime: RuntimeState,
}

struct RuntimeState {
    autograph: Option<crate::extract::DynExtractor>,
    autograph_queue: crate::service::AutographQueue,
}

impl<E: Embedder> LiveGenerationSlot<E> {
    pub(crate) fn cut_over(
        &self,
        mut cutover: LiveCutover<'_>,
        target_embedder: E,
        seal: impl FnOnce(&MemoryService<E, NativeStore>) -> Result<(), MemoryError>,
    ) -> Result<(), MemoryError> {
        let target_witness = crate::migration::target_embedder_witness(&target_embedder)?;
        let mut generation = self.generation.write();
        let retired = take_retired(
            &mut generation,
            &mut cutover,
            &target_embedder,
            &target_witness,
            seal,
        )?;
        let (target, retired) =
            stage_or_restore(&mut generation, &cutover, retired, target_embedder)?;
        let (target, retired) =
            activate_or_restore(&mut generation, &mut cutover, retired, target)?;
        *generation = Some(transplant(target, retired.runtime, cutover.target_model));
        drop(generation);
        finish_switch(&cutover)
    }
}

fn take_retired<E: Embedder>(
    generation: &mut Option<ActiveGeneration<E>>,
    cutover: &mut LiveCutover<'_>,
    target_embedder: &E,
    target_witness: &str,
    seal: impl FnOnce(&MemoryService<E, NativeStore>) -> Result<(), MemoryError>,
) -> Result<RetiredGeneration<E>, MemoryError> {
    let active = generation
        .as_ref()
        .ok_or_else(|| super::unavailable("service generation is recovering"))?;
    preflight(active, cutover, target_embedder, target_witness, seal)?;
    let retired = retire(
        generation
            .take()
            .ok_or_else(|| super::unavailable("service generation disappeared"))?,
    );
    if let Err(error) = prove_source_handle_closed(cutover, retired.embedder.dimension()) {
        return restore(generation, cutover, retired, error);
    }
    Ok(retired)
}

fn stage_or_restore<E: Embedder>(
    generation: &mut Option<ActiveGeneration<E>>,
    cutover: &LiveCutover<'_>,
    retired: RetiredGeneration<E>,
    target_embedder: E,
) -> Result<(MemoryService<E, NativeStore>, RetiredGeneration<E>), MemoryError> {
    match stage_and_open(cutover, target_embedder) {
        Ok(target) => Ok((target, retired)),
        Err(error) => restore(generation, cutover, retired, error),
    }
}

fn activate_or_restore<E: Embedder>(
    generation: &mut Option<ActiveGeneration<E>>,
    cutover: &mut LiveCutover<'_>,
    retired: RetiredGeneration<E>,
    target: MemoryService<E, NativeStore>,
) -> Result<(MemoryService<E, NativeStore>, RetiredGeneration<E>), MemoryError> {
    if let Err(error) = cutover.controller.activate(cutover.completed_at) {
        drop(target);
        return restore(generation, cutover, retired, error);
    }
    Ok((target, retired))
}

fn finish_switch(cutover: &LiveCutover<'_>) -> Result<(), MemoryError> {
    finalize_staged_live_switch(cutover.source, cutover.destination)?;
    commit_retained_switch(cutover.source, cutover.destination)?;
    Ok(())
}

fn prove_source_handle_closed(
    cutover: &LiveCutover<'_>,
    source_dimension: usize,
) -> Result<(), MemoryError> {
    let probe = NativeStore::open(cutover.source, source_dimension).map_err(|error| {
        super::unavailable(format!("source handle survived retirement: {error}"))
    })?;
    drop(probe);
    Ok(())
}

fn preflight<E: Embedder>(
    active: &ActiveGeneration<E>,
    cutover: &mut LiveCutover<'_>,
    target_embedder: &E,
    target_witness: &str,
    seal: impl FnOnce(&MemoryService<E, NativeStore>) -> Result<(), MemoryError>,
) -> Result<(), MemoryError> {
    cutover
        .controller
        .ensure_cutover_start(cutover.started_at)?;
    seal(&active.service)?;
    ensure_journal_drained(cutover.journal)?;
    cutover.journal.verify_cutover_identity(&CutoverIdentity {
        source: cutover.source,
        destination: cutover.destination,
        source_provenance: active.model(),
        target_model: cutover.target_model,
        target_dimension: target_embedder.dimension(),
        target_witness,
        epoch_id: cutover.controller.epoch_id(),
    })?;
    let workspace = prepare_live_switch(
        cutover.source,
        cutover.destination,
        cutover.target_model,
        target_embedder.dimension(),
        target_witness,
    )?;
    if workspace != cutover.journal.workspace() {
        return Err(super::unavailable(
            "cutover workspace disagrees with dirty journal",
        ));
    }
    Ok(())
}

fn ensure_journal_drained(journal: &DirtyJournal) -> Result<(), MemoryError> {
    if journal.last_sequence() == journal.compacted_through() {
        return Ok(());
    }
    Err(super::unavailable(
        "dirty journal is not drained at the final watermark",
    ))
}

fn stage_and_open<E: Embedder>(
    cutover: &LiveCutover<'_>,
    target_embedder: E,
) -> Result<MemoryService<E, NativeStore>, MemoryError> {
    stage_live_switch(cutover.source, cutover.destination)?;
    MemoryService::open(cutover.source, target_embedder)
        .map_err(|error| super::unavailable(format!("cannot open activated target: {error}")))
}

fn retire<E: Embedder>(generation: ActiveGeneration<E>) -> RetiredGeneration<E> {
    let MemoryService {
        store,
        embedder,
        autograph,
        autograph_queue,
        generation_gate: _,
    } = generation.service;
    drop(store);
    RetiredGeneration {
        embedder,
        model: generation.model,
        runtime: RuntimeState {
            autograph,
            autograph_queue,
        },
    }
}

fn restore<T, E: Embedder>(
    slot: &mut Option<ActiveGeneration<E>>,
    cutover: &LiveCutover<'_>,
    retired: RetiredGeneration<E>,
    original: MemoryError,
) -> Result<T, MemoryError> {
    rollback_staged_live_switch(cutover.source, cutover.destination)
        .map_err(|recovery| combined(&original, &recovery))?;
    let store = NativeStore::open(cutover.source, retired.embedder.dimension())
        .map_err(|recovery| combined(&original, &recovery))?;
    *slot = Some(assemble(
        store,
        retired.embedder,
        retired.runtime,
        retired.model,
    ));
    Err(original)
}

fn transplant<E: Embedder>(
    target: MemoryService<E, NativeStore>,
    runtime: RuntimeState,
    model: &str,
) -> ActiveGeneration<E> {
    let MemoryService {
        store, embedder, ..
    } = target;
    assemble(store, embedder, runtime, model.to_owned())
}

fn assemble<E: Embedder>(
    store: NativeStore,
    embedder: E,
    runtime: RuntimeState,
    model: String,
) -> ActiveGeneration<E> {
    ActiveGeneration {
        service: MemoryService {
            store,
            embedder,
            autograph: runtime.autograph,
            autograph_queue: runtime.autograph_queue,
            generation_gate: parking_lot::RwLock::new(()),
        },
        model,
    }
}

fn combined(original: &MemoryError, recovery: &MemoryError) -> MemoryError {
    super::unavailable(format!(
        "cutover failed: {original}; source recovery also failed: {recovery}"
    ))
}
