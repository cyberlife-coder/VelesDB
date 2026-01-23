//! Constructor helpers for `VectorStore`.

use crate::{DistanceMetric, StorageMode, VectorStore};
use wasm_bindgen::JsValue;

/// Parses a metric string to `DistanceMetric`.
pub fn parse_metric(metric: &str) -> Result<DistanceMetric, JsValue> {
    match metric.to_lowercase().as_str() {
        "cosine" => Ok(DistanceMetric::Cosine),
        "euclidean" | "l2" => Ok(DistanceMetric::Euclidean),
        "dot" | "dotproduct" | "inner" => Ok(DistanceMetric::DotProduct),
        "hamming" => Ok(DistanceMetric::Hamming),
        "jaccard" => Ok(DistanceMetric::Jaccard),
        _ => Err(JsValue::from_str(
            "Unknown metric. Use: cosine, euclidean, dot, hamming, jaccard",
        )),
    }
}

/// Parses a storage mode string to `StorageMode`.
pub fn parse_storage_mode(mode: &str) -> Result<StorageMode, JsValue> {
    match mode.to_lowercase().as_str() {
        "full" => Ok(StorageMode::Full),
        "sq8" => Ok(StorageMode::SQ8),
        "binary" => Ok(StorageMode::Binary),
        _ => Err(JsValue::from_str(
            "Unknown storage mode. Use: full, sq8, binary",
        )),
    }
}

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
    }
    store.payloads.reserve(capacity);
    store
}
