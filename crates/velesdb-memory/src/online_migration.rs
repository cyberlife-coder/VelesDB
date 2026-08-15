use crate::embedder::Embedder;
use crate::storage::NativeStore;
use crate::MemoryService;

#[path = "online_migration/cutover.rs"]
mod cutover;
pub(crate) use cutover::LiveCutover;
#[path = "online_migration/cleanup.rs"]
mod cleanup;
#[path = "online_migration/job_runner.rs"]
mod job_runner;
#[path = "online_migration/job_state.rs"]
mod job_state;
#[path = "online_migration/manager.rs"]
mod manager;
#[path = "online_migration/startup.rs"]
mod startup;
pub(crate) use cleanup::remove_cancelled_artifacts;
pub(crate) use job_runner::{run_job, JobRunOutcome, JobTarget};
pub(crate) use job_state::{JobPhase, JobRecord, JobSpec, JobStore};
pub(crate) use manager::{MigrationStartConfig, MigrationStatus, OnlineMigrationManager};
pub(crate) use startup::recover_startup;
#[path = "online_migration/recovery.rs"]
mod recovery;
pub(crate) use recovery::LiveRecovery;

pub(crate) struct LiveGenerationSlot<E: Embedder> {
    generation: parking_lot::RwLock<Option<ActiveGeneration<E>>>,
}

pub(crate) struct ActiveGeneration<E: Embedder> {
    service: MemoryService<E, NativeStore>,
    model: String,
}

impl<E: Embedder> ActiveGeneration<E> {
    pub(crate) fn model(&self) -> &str {
        &self.model
    }

    pub(crate) fn dimension(&self) -> usize {
        self.service.embedder.dimension()
    }
}

impl<E: Embedder> LiveGenerationSlot<E> {
    pub(crate) fn new(service: MemoryService<E, NativeStore>, model: impl Into<String>) -> Self {
        Self {
            generation: parking_lot::RwLock::new(Some(ActiveGeneration {
                service,
                model: model.into(),
            })),
        }
    }

    pub(crate) fn with_generation<T>(
        &self,
        run: impl FnOnce(&ActiveGeneration<E>) -> T,
    ) -> Result<T, crate::MemoryError> {
        let generation = self.generation.read();
        generation
            .as_ref()
            .map(run)
            .ok_or_else(|| unavailable("service generation is recovering"))
    }

    pub(crate) fn run<T>(
        &self,
        operation: impl FnOnce(&MemoryService<E, NativeStore>) -> Result<T, crate::MemoryError>,
    ) -> Result<T, crate::MemoryError> {
        let generation = self.generation.read();
        let active = generation
            .as_ref()
            .ok_or_else(|| unavailable("service generation is recovering"))?;
        operation(&active.service)
    }

    pub(crate) fn inspect<T>(
        &self,
        operation: impl FnOnce(&MemoryService<E, NativeStore>) -> T,
    ) -> Result<T, crate::MemoryError> {
        self.with_generation(|active| operation(&active.service))
    }

    pub(crate) fn inspect_active<T>(
        &self,
        operation: impl FnOnce(&str, usize, &MemoryService<E, NativeStore>) -> T,
    ) -> Result<T, crate::MemoryError> {
        self.with_generation(|active| {
            operation(active.model(), active.dimension(), &active.service)
        })
    }

    pub(crate) fn declare_model(&self, model: impl Into<String>) {
        if let Some(active) = self.generation.write().as_mut() {
            active.model = model.into();
        }
    }

    #[cfg(test)]
    pub(crate) fn replace_for_test(
        &self,
        service: MemoryService<E, NativeStore>,
        model: impl Into<String>,
    ) {
        *self.generation.write() = Some(ActiveGeneration {
            service,
            model: model.into(),
        });
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl<E> LiveGenerationSlot<E>
where
    E: Embedder + Send + Sync + 'static,
{
    pub(crate) fn spawn_autograph_worker(
        self: &std::sync::Arc<Self>,
        capacity: usize,
    ) -> Result<crate::service::AutographWorkerHandle, crate::MemoryError> {
        let (tx, rx) = std::sync::mpsc::sync_channel(capacity);
        self.install_autograph_sender(tx)?;
        let worker_slot = std::sync::Arc::clone(self);
        let join = std::thread::Builder::new()
            .name("velesdb-autograph".to_owned())
            .spawn(move || worker_slot.autograph_worker_loop(&rx))
            .map_err(worker_error)?;
        let closer_slot = std::sync::Arc::clone(self);
        Ok(crate::service::AutographWorkerHandle {
            close_queue: Some(Box::new(move || closer_slot.close_autograph_queue())),
            join: Some(join),
        })
    }

    fn install_autograph_sender(
        &self,
        tx: std::sync::mpsc::SyncSender<crate::service::AutographJob>,
    ) -> Result<(), crate::MemoryError> {
        self.run(|service| {
            let mut sender = service.autograph_queue.tx.lock();
            if sender.is_some() {
                return Err(worker_error("autograph worker already spawned"));
            }
            *sender = Some(tx);
            service
                .autograph_queue
                .closing
                .store(false, std::sync::atomic::Ordering::Release);
            Ok(())
        })
    }

    fn autograph_worker_loop(&self, rx: &std::sync::mpsc::Receiver<crate::service::AutographJob>) {
        let mut skipped = 0_u64;
        for job in rx {
            if self.autograph_closing() || self.process_autograph(&job).is_err() {
                skipped = skipped.saturating_add(1);
            }
        }
        self.record_skipped_autographs(skipped);
    }

    fn autograph_closing(&self) -> bool {
        self.inspect(|service| {
            service
                .autograph_queue
                .closing
                .load(std::sync::atomic::Ordering::Acquire)
        })
        .unwrap_or(true)
    }

    fn process_autograph(
        &self,
        job: &crate::service::AutographJob,
    ) -> Result<(), crate::MemoryError> {
        self.run(|service| {
            let _generation = service.enter_generation();
            service.autograph(job.fact_id, &job.fact);
            Ok(())
        })
    }

    fn record_skipped_autographs(&self, skipped: u64) {
        if skipped == 0 {
            return;
        }
        let _ = self.inspect(|service| {
            service
                .autograph_queue
                .dropped
                .fetch_add(skipped, std::sync::atomic::Ordering::Relaxed);
        });
        #[cfg(feature = "mcp")]
        tracing::warn!(
            skipped,
            "autograph worker closing: queued enrichments skipped — facts remain stored"
        );
    }

    fn close_autograph_queue(&self) {
        let _ = self.inspect(|service| {
            service
                .autograph_queue
                .closing
                .store(true, std::sync::atomic::Ordering::Release);
            service.autograph_queue.tx.lock().take();
        });
    }
}

fn worker_error(message: impl std::fmt::Display) -> crate::MemoryError {
    crate::MemoryError::Extract(crate::extract::ExtractError::Backend(format!(
        "autograph worker: {message}"
    )))
}

fn unavailable(message: impl Into<String>) -> crate::MemoryError {
    velesdb_core::Error::Query(message.into()).into()
}

impl<E: Embedder> MemoryService<E, NativeStore> {
    pub(crate) fn migration_store(&self) -> &NativeStore {
        &self.store
    }

    pub(crate) fn migration_exclusive<T>(
        &self,
        run: impl FnOnce() -> Result<T, crate::MemoryError>,
    ) -> Result<T, crate::MemoryError> {
        let _generation = self.generation_gate.write();
        run()
    }
}

#[cfg(test)]
#[path = "online_migration/cleanup_tests.rs"]
mod cleanup_tests;
#[cfg(test)]
#[path = "online_migration/job_runner_tests.rs"]
mod job_runner_tests;
#[cfg(test)]
#[path = "online_migration/job_state_tests.rs"]
mod job_state_tests;
#[cfg(test)]
#[path = "online_migration/manager_tests.rs"]
mod manager_tests;
#[cfg(test)]
#[path = "online_migration/startup_tests.rs"]
mod startup_tests;
#[cfg(test)]
#[path = "online_migration_tests.rs"]
mod tests;
