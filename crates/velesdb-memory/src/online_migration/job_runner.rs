use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::embedder::Embedder;
use crate::mutation::catchup::OnlineCatchUp;
use crate::mutation::controller::{
    CancellationPermit, ControllerPhase, ConvergenceController, ConvergenceSample,
    ConvergenceVerdict,
};
use crate::mutation::journal::DirtyJournal;
use crate::storage::NativeStore;
use crate::MemoryError;

use super::job_state::{JobPhase, JobRecord, JobStore};
use super::{LiveCutover, LiveGenerationSlot};

pub(crate) struct JobTarget<E: Embedder> {
    pub(crate) embedder: E,
    pub(crate) model: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JobRunOutcome {
    Committed,
    NonConverging,
    Cancelled,
}

pub(crate) fn run_job<E, C>(
    slot: &LiveGenerationSlot<E>,
    store: &JobStore,
    mut record: JobRecord,
    target: JobTarget<E>,
    cancelled: C,
) -> Result<JobRunOutcome, MemoryError>
where
    E: Embedder,
    C: Fn() -> bool,
{
    let journal = Arc::new(DirtyJournal::open(
        &record.spec.workspace,
        &record.spec.identity,
        record.spec.journal_max_bytes,
    )?);
    prepare_base(slot, store, &mut record, &target, Arc::clone(&journal))?;
    let mut controller = ConvergenceController::open(
        &record.spec.workspace,
        record.spec.identity.epoch_id(),
        record.spec.controller,
    )?;
    if record.cancellation_requested || cancelled() {
        return cancel(slot, store, &mut record, &mut controller, journal.as_ref());
    }
    catch_up(
        slot,
        store,
        &mut record,
        target,
        &journal,
        &mut controller,
        cancelled,
    )
}

fn prepare_base<E: Embedder>(
    slot: &LiveGenerationSlot<E>,
    store: &JobStore,
    record: &mut JobRecord,
    target: &JobTarget<E>,
    journal: Arc<DirtyJournal>,
) -> Result<(), MemoryError> {
    if !matches!(record.phase, JobPhase::Prepared | JobPhase::Capturing) {
        return Ok(());
    }
    if record.phase == JobPhase::Prepared {
        transition(store, record, JobPhase::Capturing)?;
    }
    let destination = open_destination(record, target.embedder.dimension())?;
    let progress = slot.run(|active| {
        let session = session(active, &destination, &target.embedder, journal, record)?;
        session.copy_base()
    })?;
    record.record_base_copy(progress);
    transition(store, record, JobPhase::BaseCopied)?;
    transition(store, record, JobPhase::CatchingUp)
}

fn catch_up<E, C>(
    slot: &LiveGenerationSlot<E>,
    store: &JobStore,
    record: &mut JobRecord,
    target: JobTarget<E>,
    journal: &Arc<DirtyJournal>,
    controller: &mut ConvergenceController,
    cancelled: C,
) -> Result<JobRunOutcome, MemoryError>
where
    E: Embedder,
    C: Fn() -> bool,
{
    resume_catching_up(store, record)?;
    let origin = Instant::now();
    let mut observed_at = Duration::ZERO;
    loop {
        if cancellation_requested(record, &cancelled) {
            return cancel(slot, store, record, controller, journal.as_ref());
        }
        let progress = replay_once(slot, record, &target, Arc::clone(journal))?;
        observed_at = next_observation(origin.elapsed(), observed_at);
        let observation =
            controller.observe(ConvergenceSample::from_replay(observed_at, progress))?;
        record.record_observation(observation);
        let phase = phase_for(observation.verdict);
        transition(store, record, phase)?;
        match observation.verdict {
            ConvergenceVerdict::CatchingUp => {}
            ConvergenceVerdict::NonConverging => return Ok(JobRunOutcome::NonConverging),
            ConvergenceVerdict::CutoverReady => break,
        }
    }
    cut_over(slot, store, record, target, journal, controller, origin)
}

fn cancellation_requested(record: &JobRecord, cancelled: &impl Fn() -> bool) -> bool {
    record.cancellation_requested || cancelled()
}

fn replay_once<E: Embedder>(
    slot: &LiveGenerationSlot<E>,
    record: &JobRecord,
    target: &JobTarget<E>,
    journal: Arc<DirtyJournal>,
) -> Result<crate::mutation::catchup::ReplayProgress, MemoryError> {
    let destination = open_destination(record, target.embedder.dimension())?;
    slot.run(|active| {
        let session = session(active, &destination, &target.embedder, journal, record)?;
        session.catch_up_batch()
    })
}

fn cut_over<E: Embedder>(
    slot: &LiveGenerationSlot<E>,
    store: &JobStore,
    record: &mut JobRecord,
    target: JobTarget<E>,
    journal: &Arc<DirtyJournal>,
    controller: &mut ConvergenceController,
    origin: Instant,
) -> Result<JobRunOutcome, MemoryError> {
    let started_at = origin.elapsed();
    controller.begin_quiescing(started_at)?;
    transition(store, record, JobPhase::Quiescing)?;
    let now = || origin.elapsed();
    let JobTarget { embedder, model } = target;
    let cutover = LiveCutover {
        controller,
        journal: journal.as_ref(),
        source: record.spec.identity.source_path(),
        destination: record.spec.identity.destination_path(),
        target_model: &model,
        started_at,
        now: &now,
    };
    let result = slot.cut_over(cutover, embedder, |source, embedder| {
        final_drain(source, record, embedder, Arc::clone(journal))
    });
    if let Err(error) = result {
        record.fail(error.to_string());
        reconcile_cutover_failure(store, record, controller)?;
        return Err(error);
    }
    record.progress.measured_cutover = controller.measured_cutover();
    transition(store, record, JobPhase::Activated)?;
    transition(store, record, JobPhase::Committed)?;
    Ok(JobRunOutcome::Committed)
}

fn final_drain<E: Embedder>(
    source: &crate::MemoryService<E, NativeStore>,
    record: &JobRecord,
    target_embedder: &E,
    journal: Arc<DirtyJournal>,
) -> Result<(), MemoryError> {
    let destination = open_destination(record, target_embedder.dimension())?;
    let session = OnlineCatchUp::resume(
        source,
        &destination,
        target_embedder,
        journal,
        record.spec.catch_up,
    )?;
    while session.catch_up_batch()?.backlog > 0 {}
    session.verify()
}

fn session<'a, E: Embedder>(
    source: &'a crate::MemoryService<E, NativeStore>,
    destination: &'a NativeStore,
    target_embedder: &'a E,
    journal: Arc<DirtyJournal>,
    record: &JobRecord,
) -> Result<OnlineCatchUp<'a, E>, MemoryError> {
    if source.migration_capture_active() {
        OnlineCatchUp::resume(
            source,
            destination,
            target_embedder,
            journal,
            record.spec.catch_up,
        )
    } else {
        OnlineCatchUp::start(
            source,
            destination,
            target_embedder,
            journal,
            record.spec.catch_up,
        )
    }
}

fn cancel<E: Embedder>(
    slot: &LiveGenerationSlot<E>,
    store: &JobStore,
    record: &mut JobRecord,
    controller: &mut ConvergenceController,
    journal: &DirtyJournal,
) -> Result<JobRunOutcome, MemoryError> {
    record.request_cancellation();
    let permit = cancel_permit(slot, record, controller)?;
    verify_cancel_identity(record, journal, &permit)?;
    slot.run(|source| source.install_mutation_observer(None))?;
    transition(store, record, JobPhase::Cancelled)?;
    Ok(JobRunOutcome::Cancelled)
}

fn cancel_permit<E: Embedder>(
    slot: &LiveGenerationSlot<E>,
    record: &JobRecord,
    controller: &mut ConvergenceController,
) -> Result<CancellationPermit, MemoryError> {
    let authoritative =
        slot.inspect_active(|model, _, _| model == record.spec.identity.source_provenance())?;
    controller.cancel(authoritative, record.spec.identity.epoch_id())
}

fn verify_cancel_identity(
    record: &JobRecord,
    journal: &DirtyJournal,
    permit: &CancellationPermit,
) -> Result<(), MemoryError> {
    if permit.epoch_id() != record.spec.identity.epoch_id() {
        return Err(crate::MemoryError::MigrationCapture(
            "cancellation permit epoch mismatch".to_owned(),
        ));
    }
    let identity = &record.spec.identity;
    journal.verify_cutover_identity(&crate::mutation::journal::CutoverIdentity {
        source: identity.source_path(),
        destination: identity.destination_path(),
        source_provenance: identity.source_provenance(),
        target_model: identity.target_model(),
        target_dimension: identity.target_dimension(),
        target_witness: identity.target_witness(),
        epoch_id: identity.epoch_id(),
    })
}

fn resume_catching_up(store: &JobStore, record: &mut JobRecord) -> Result<(), MemoryError> {
    if matches!(
        record.phase,
        JobPhase::BaseCopied | JobPhase::NonConverging | JobPhase::CutoverReady
    ) {
        transition(store, record, JobPhase::CatchingUp)?;
    }
    Ok(())
}

fn reconcile_cutover_failure(
    store: &JobStore,
    record: &mut JobRecord,
    controller: &ConvergenceController,
) -> Result<(), MemoryError> {
    if controller.phase() == ControllerPhase::CatchingUp {
        record.transition(JobPhase::CatchingUp)?;
    } else {
        record.recovery_action = Some(
            controller
                .recovery_action()
                .unwrap_or("complete or recover cutover before serving traffic")
                .to_owned(),
        );
    }
    store.save(record)
}

fn open_destination(record: &JobRecord, dimension: usize) -> Result<NativeStore, MemoryError> {
    NativeStore::open(record.spec.identity.destination_path(), dimension)
}

fn transition(
    store: &JobStore,
    record: &mut JobRecord,
    phase: JobPhase,
) -> Result<(), MemoryError> {
    record.transition(phase)?;
    store.save(record)
}

fn next_observation(now: Duration, previous: Duration) -> Duration {
    if now > previous {
        now
    } else {
        previous.saturating_add(Duration::from_nanos(1))
    }
}

fn phase_for(verdict: ConvergenceVerdict) -> JobPhase {
    match verdict {
        ConvergenceVerdict::CatchingUp => JobPhase::CatchingUp,
        ConvergenceVerdict::CutoverReady => JobPhase::CutoverReady,
        ConvergenceVerdict::NonConverging => JobPhase::NonConverging,
    }
}
