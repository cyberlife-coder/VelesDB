//! Constructor helpers for `VectorStore`.

use crate::{DistanceMetric, StorageMode, VectorStore};

/// Creates a new empty `VectorStore`.
pub fn create_store(
    dimension: usize,
    metric: DistanceMetric,
    storage_mode: StorageMode,
) -> VectorStore {
    VectorStore {
        ids: Vec::new(),
        data: Vec::new(),
        payloads: Vec::new(),
        dimension,
        metric,
        storage_mode,
        sparse_index: None,
    }
}

/// Creates a metadata-only store (dimension=0).
pub fn create_metadata_only() -> VectorStore {
    create_store(0, DistanceMetric::Cosine, StorageMode::Full)
}

/// Creates a store with capacity pre-allocation.
pub fn create_with_capacity(
    dimension: usize,
    metric: DistanceMetric,
    storage_mode: StorageMode,
    capacity: usize,
) -> VectorStore {
    let mut store = create_store(dimension, metric, storage_mode);
    store.ids.reserve(capacity);
    // A capacity hint; `saturating_mul` keeps a 32-bit-usize wasm32 build from
    // silently under-reserving on a huge `capacity` (a wrapping `*` would). A
    // genuinely oversized request still aborts on allocation.
    store.data.reserve(capacity.saturating_mul(dimension));
    store.payloads.reserve(capacity);
    store
}
