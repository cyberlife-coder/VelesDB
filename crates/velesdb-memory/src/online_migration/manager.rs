use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;

use crate::embedder::Embedder;
use crate::embedding_provenance::{self, EmbeddingProvenance};
use crate::migration::{journal_workspace, target_embedder_witness};
use crate::mutation::catchup::CatchUpConfig;
use crate::mutation::controller::{ControllerConfig, ConvergenceController};
use crate::mutation::journal::{DirtyJournal, EpochIdentity};
use crate::storage::NativeStore;
use crate::MemoryError;

use super::job_runner::{run_job, JobRunOutcome, JobTarget};
use super::job_state::{JobPhase, JobRecord, JobSpec, JobStore};
use super::{remove_cancelled_artifacts, LiveGenerationSlot};

type TargetFactory<E> = dyn Fn(&str) -> Result<JobTarget<E>, MemoryError> + Send + Sync;

#[derive(Debug, Clone, Copy)]
pub(crate) struct MigrationStartConfig {
    pub(crate) journal_max_bytes: u64,
    pub(crate) catch_up: CatchUpConfig,
    pub(crate) controller: ControllerConfig,
}

pub(crate) struct MigrationStatus {
    pub(crate) record: JobRecord,
    pub(crate) running: bool,
}

struct Worker {
    cancel: Arc<AtomicBool>,
    join: std::thread::JoinHandle<()>,
}

pub(crate) struct OnlineMigrationManager<E: Embedder> {
    slot: Arc<LiveGenerationSlot<E>>,
    source: PathBuf,
    control: PathBuf,
    target_factory: Arc<TargetFactory<E>>,
    store: Mutex<Option<JobStore>>,
    worker: Mutex<Option<Worker>>,
}

impl<E> OnlineMigrationManager<E>
where
    E: Embedder + Send + Sync + 'static,
{
    pub(crate) fn new(
        slot: Arc<LiveGenerationSlot<E>>,
        source: impl Into<PathBuf>,
        target_factory: Arc<TargetFactory<E>>,
    ) -> Result<Arc<Self>, MemoryError> {
        let source = source.into();
        let control = sibling(&source, "online-migration-control")?;
        std::fs::create_dir_all(&control)
            .map_err(|error| capture(format!("cannot create migration control state: {error}")))?;
        ensure_real_directory(&control)?;
        let store = JobStore::try_open(&control)?;
        Ok(Arc::new(Self {
            slot,
            source,
            control,
            target_factory,
            store: Mutex::new(store),
            worker: Mutex::new(None),
        }))
    }

    pub(crate) fn start(
        self: &Arc<Self>,
        target_backend: &str,
        config: MigrationStartConfig,
    ) -> Result<JobRecord, MemoryError> {
        self.reap_worker()?;
        self.ensure_idle()?;
        self.start_idle(target_backend, config)
    }

    fn start_idle(
        self: &Arc<Self>,
        target_backend: &str,
        config: MigrationStartConfig,
    ) -> Result<JobRecord, MemoryError> {
        let target = (self.target_factory)(target_backend)?;
        let record = self.prepare_record(target_backend, &target, config)?;
        let store = JobStore::create(&self.control, &record)?;
        *self.store.lock() = Some(store.clone());
        prepare_artifacts(&record, target.embedder.dimension())?;
        self.spawn(store, record.clone(), target)?;
        Ok(record)
    }

    pub(crate) fn status(&self) -> Result<Option<MigrationStatus>, MemoryError> {
        let Some(store) = self.store.lock().clone() else {
            return Ok(None);
        };
        let running = self
            .worker
            .lock()
            .as_ref()
            .is_some_and(|worker| !worker.join.is_finished());
        Ok(Some(MigrationStatus {
            record: store.load()?,
            running,
        }))
    }

    pub(crate) fn cancel(&self) -> Result<JobRecord, MemoryError> {
        self.reap_worker()?;
        self.cancel_current()
    }

    fn cancel_current(&self) -> Result<JobRecord, MemoryError> {
        let store = self.current_store()?;
        let mut record = store.load()?;
        if matches!(record.phase, JobPhase::Committed | JobPhase::Cancelled) {
            return Ok(record);
        }
        record.request_cancellation();
        store.save(&record)?;
        if let Some(worker) = self.worker.lock().as_ref() {
            worker.cancel.store(true, Ordering::Release);
            return Ok(record);
        }
        cancel_stopped(&self.slot, &store, &mut record)?;
        remove_cancelled_artifacts(&record)?;
        Ok(record)
    }

    pub(crate) fn recover(self: &Arc<Self>) -> Result<JobRecord, MemoryError> {
        self.reap_worker()?;
        self.ensure_idle()?;
        let (store, record, target) = self.recovery_inputs()?;
        self.spawn(store, record.clone(), target)?;
        Ok(record)
    }

    fn recovery_inputs(&self) -> Result<(JobStore, JobRecord, JobTarget<E>), MemoryError> {
        let store = self.current_store()?;
        let record = store.load()?;
        ensure_resumable(record.phase)?;
        let target = (self.target_factory)(&record.spec.target_backend)?;
        verify_target(&record, &target)?;
        Ok((store, record, target))
    }

    fn prepare_record(
        &self,
        target_backend: &str,
        target: &JobTarget<E>,
        config: MigrationStartConfig,
    ) -> Result<JobRecord, MemoryError> {
        let source = self
            .source
            .canonicalize()
            .map_err(|error| capture(format!("cannot resolve migration source: {error}")))?;
        let destination = sibling(&source, "online-migration-target")?;
        refuse_existing_destination(&destination)?;
        let workspace = journal_workspace(&destination)?;
        let source_model = self.slot.inspect_active(|model, _, _| model.to_owned())?;
        let witness = target_embedder_witness(&target.embedder)?;
        let identity = EpochIdentity::new(
            source,
            source_model,
            target.model.clone(),
            target.embedder.dimension(),
            witness,
            destination,
        )?;
        JobRecord::new(JobSpec {
            identity,
            target_backend: target_backend.to_owned(),
            journal_max_bytes: config.journal_max_bytes,
            catch_up: config.catch_up,
            controller: config.controller,
            workspace,
        })
    }

    fn spawn(
        self: &Arc<Self>,
        store: JobStore,
        record: JobRecord,
        target: JobTarget<E>,
    ) -> Result<(), MemoryError> {
        let cancel = Arc::new(AtomicBool::new(record.cancellation_requested));
        let worker_cancel = Arc::clone(&cancel);
        let slot = Arc::clone(&self.slot);
        let join = std::thread::Builder::new()
            .name("velesdb-online-migration".to_owned())
            .spawn(move || {
                let result = run_job(&slot, &store, record, target, || {
                    worker_cancel.load(Ordering::Acquire)
                });
                match result {
                    Ok(JobRunOutcome::Cancelled) => cleanup_after_worker(&store),
                    Err(error) => persist_worker_error(&store, &error),
                    _ => {}
                }
            })
            .map_err(|error| capture(format!("cannot spawn online migration: {error}")))?;
        *self.worker.lock() = Some(Worker { cancel, join });
        Ok(())
    }

    fn ensure_idle(&self) -> Result<(), MemoryError> {
        if self.worker.lock().is_some() {
            return Err(capture("an online migration worker is already active"));
        }
        Ok(())
    }

    fn current_store(&self) -> Result<JobStore, MemoryError> {
        self.store
            .lock()
            .clone()
            .ok_or_else(|| capture("no online migration job exists"))
    }

    fn reap_worker(&self) -> Result<(), MemoryError> {
        let finished = self
            .worker
            .lock()
            .as_ref()
            .is_some_and(|worker| worker.join.is_finished());
        if !finished {
            return Ok(());
        }
        let worker = self
            .worker
            .lock()
            .take()
            .ok_or_else(|| capture("finished migration worker disappeared"))?;
        worker
            .join
            .join()
            .map_err(|_| capture("online migration worker panicked"))
    }
}

fn prepare_artifacts(record: &JobRecord, dimension: usize) -> Result<(), MemoryError> {
    drop(DirtyJournal::open(
        &record.spec.workspace,
        &record.spec.identity,
        record.spec.journal_max_bytes,
    )?);
    prepare_destination(record, dimension)
}

fn cancel_stopped<E: Embedder>(
    slot: &LiveGenerationSlot<E>,
    store: &JobStore,
    record: &mut JobRecord,
) -> Result<(), MemoryError> {
    if matches!(
        record.phase,
        JobPhase::BaseCopied
            | JobPhase::CatchingUp
            | JobPhase::NonConverging
            | JobPhase::CutoverReady
    ) {
        let mut controller = ConvergenceController::open(
            &record.spec.workspace,
            record.spec.identity.epoch_id(),
            record.spec.controller,
        )?;
        let authoritative =
            slot.inspect_active(|model, _, _| model == record.spec.identity.source_provenance())?;
        controller.cancel(authoritative, record.spec.identity.epoch_id())?;
    }
    slot.run(|source| source.install_mutation_observer(None))?;
    record.transition(JobPhase::Cancelled)?;
    store.save(record)
}

fn cleanup_after_worker(store: &JobStore) {
    let result = store
        .load()
        .and_then(|record| remove_cancelled_artifacts(&record));
    if let Err(error) = result {
        persist_worker_error(store, &error);
    }
}

fn prepare_destination(record: &JobRecord, dimension: usize) -> Result<(), MemoryError> {
    let destination = record.spec.identity.destination_path();
    refuse_existing_destination(destination)?;
    drop(NativeStore::open(destination, dimension)?);
    embedding_provenance::write(
        destination,
        &EmbeddingProvenance::new(record.spec.identity.target_model(), dimension),
    )
    .map_err(capture)
}

fn verify_target<E: Embedder>(
    record: &JobRecord,
    target: &JobTarget<E>,
) -> Result<(), MemoryError> {
    let identity = &record.spec.identity;
    let witness = target_embedder_witness(&target.embedder)?;
    if identity.target_model() != target.model
        || identity.target_dimension() != target.embedder.dimension()
        || identity.target_witness() != witness
    {
        return Err(capture("migration target identity changed before recovery"));
    }
    Ok(())
}

fn ensure_resumable(phase: JobPhase) -> Result<(), MemoryError> {
    match phase {
        JobPhase::Prepared
        | JobPhase::Capturing
        | JobPhase::BaseCopied
        | JobPhase::CatchingUp
        | JobPhase::NonConverging
        | JobPhase::CutoverReady => Ok(()),
        JobPhase::Quiescing | JobPhase::Activated => Err(capture(
            "cutover recovery is required before the migration can resume",
        )),
        JobPhase::Committed | JobPhase::Cancelled => {
            Err(capture("the online migration job is already terminal"))
        }
    }
}

fn persist_worker_error(store: &JobStore, error: &MemoryError) {
    if let Ok(mut record) = store.load() {
        record.fail(error.to_string());
        let _ = store.save(&record);
    }
}

fn refuse_existing_destination(path: &Path) -> Result<(), MemoryError> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(capture(format!(
            "online migration destination already exists: {}",
            path.display()
        ))),
        Err(error) => Err(capture(format!(
            "cannot inspect online migration destination: {error}"
        ))),
    }
}

fn ensure_real_directory(path: &Path) -> Result<(), MemoryError> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| capture(format!("cannot inspect migration directory: {error}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(capture(
            "online migration control path must be a real directory",
        ));
    }
    Ok(())
}

fn sibling(source: &Path, suffix: &str) -> Result<PathBuf, MemoryError> {
    let name = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| capture("online migration source has no usable directory name"))?;
    Ok(source.with_file_name(format!("{name}.{suffix}")))
}

fn capture(message: impl Into<String>) -> MemoryError {
    MemoryError::MigrationCapture(message.into())
}
