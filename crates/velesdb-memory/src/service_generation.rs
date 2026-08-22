//! The store-generation gate and its guard — `service.rs`'s concurrency
//! seam, split out under the file-budget rule (#1974) the same way
//! `service_graph.rs` was. A `#[path]` child of `service`, so the private
//! field types stay private to the service subtree.

/// The store-generation gate: mutation paths hold it shared, generation
/// writers (observer install, online migration's swap) take it exclusive.
/// Under `persistence` it is a real lock; without the feature (wasm,
/// single-threaded by construction) it is a ZST with the same read-side
/// API, so the service keeps one shape and the no-op compiles out instead
/// of the *field* silently disappearing (#2017).
#[cfg(feature = "persistence")]
pub(super) struct GenerationGate(parking_lot::RwLock<()>);
#[cfg(feature = "persistence")]
impl GenerationGate {
    pub(super) fn new() -> Self {
        Self(parking_lot::RwLock::new(()))
    }
    pub(super) fn read(&self) -> GenerationReadGuard<'_> {
        self.0.read()
    }
    pub(super) fn write(&self) -> parking_lot::RwLockWriteGuard<'_, ()> {
        self.0.write()
    }
}
#[cfg(not(feature = "persistence"))]
pub(super) struct GenerationGate;
#[cfg(not(feature = "persistence"))]
impl GenerationGate {
    pub(super) fn new() -> Self {
        Self
    }
    pub(super) fn read(&self) -> GenerationReadGuard<'_> {
        let Self = *self;
        std::marker::PhantomData
    }
}

#[cfg(feature = "persistence")]
pub(super) type GenerationReadGuard<'a> = parking_lot::RwLockReadGuard<'a, ()>;
#[cfg(not(feature = "persistence"))]
pub(super) type GenerationReadGuard<'a> = std::marker::PhantomData<&'a ()>;

pub(super) struct GenerationGuard<'a> {
    pub(super) _guard: GenerationReadGuard<'a>,
}
