use crate::embedder::Embedder;
use crate::storage::NativeStore;
use crate::MemoryService;

#[path = "online_migration/cutover.rs"]
mod cutover;
pub(crate) use cutover::LiveCutover;
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

    #[cfg(test)]
    pub(super) fn replace_for_test(
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
#[path = "online_migration_tests.rs"]
mod tests;
