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
        data_sq8: Vec::new(),
        data_binary: Vec::new(),
        sq8_mins: Vec::new(),
        sq8_scales: Vec::new(),
        payloads: Vec::new(),
        dimension,
        metric,
        storage_mode,
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
    match storage_mode {
        StorageMode::Full => store.data.reserve(capacity * dimension),
        StorageMode::SQ8 => {
            store.data_sq8.reserve(capacity * dimension);
            store.sq8_mins.reserve(capacity);
            store.sq8_scales.reserve(capacity);
        }
        StorageMode::Binary => {
            let bytes_per = dimension.div_ceil(8);
            store.data_binary.reserve(capacity * bytes_per);
        }
        // ProductQuantization falls back to SQ8 in WASM context
        StorageMode::ProductQuantization => {
            store.data_sq8.reserve(capacity * dimension);
            store.sq8_mins.reserve(capacity);
            store.sq8_scales.reserve(capacity);
        }
    }
    store.payloads.reserve(capacity);
    store
}
