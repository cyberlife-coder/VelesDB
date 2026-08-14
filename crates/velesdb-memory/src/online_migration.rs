use crate::embedder::Embedder;
use crate::storage::NativeStore;
use crate::MemoryService;

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
