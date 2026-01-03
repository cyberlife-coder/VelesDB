//! Collection types and configuration.

use crate::distance::DistanceMetric;
use crate::index::{Bm25Index, HnswIndex};
use crate::quantization::{BinaryQuantizedVector, QuantizedVector, StorageMode};
use crate::storage::{LogPayloadStorage, MmapStorage};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Metadata for a collection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionConfig {
    /// Name of the collection.
    pub name: String,

    /// Vector dimension.
    pub dimension: usize,

    /// Distance metric.
    pub metric: DistanceMetric,

    /// Number of points in the collection.
    pub point_count: usize,

    /// Storage mode for vectors (Full, SQ8, Binary).
    #[serde(default)]
    pub storage_mode: StorageMode,
}

/// A collection of vectors with associated metadata.
#[derive(Clone)]
pub struct Collection {
    /// Path to the collection data.
    pub(super) path: PathBuf,

    /// Collection configuration.
    pub(super) config: Arc<RwLock<CollectionConfig>>,

    /// Vector storage (on-disk, memory-mapped).
    pub(super) vector_storage: Arc<RwLock<MmapStorage>>,

    /// Payload storage (on-disk, log-structured).
    pub(super) payload_storage: Arc<RwLock<LogPayloadStorage>>,

    /// HNSW index for fast approximate nearest neighbor search.
    pub(super) index: Arc<HnswIndex>,

    /// BM25 index for full-text search.
    pub(super) text_index: Arc<Bm25Index>,

    /// SQ8 quantized vectors cache (for SQ8 storage mode).
    pub(super) sq8_cache: Arc<RwLock<HashMap<u64, QuantizedVector>>>,

    /// Binary quantized vectors cache (for Binary storage mode).
    pub(super) binary_cache: Arc<RwLock<HashMap<u64, BinaryQuantizedVector>>>,
}

impl Collection {
    /// Extracts all string values from a JSON payload for text indexing.
    pub(crate) fn extract_text_from_payload(payload: &serde_json::Value) -> String {
        let mut texts = Vec::new();
        Self::collect_strings(payload, &mut texts);
        texts.join(" ")
    }

    /// Recursively collects all string values from a JSON value.
    fn collect_strings(value: &serde_json::Value, texts: &mut Vec<String>) {
        match value {
            serde_json::Value::String(s) => texts.push(s.clone()),
            serde_json::Value::Array(arr) => {
                for item in arr {
                    Self::collect_strings(item, texts);
                }
            }
            serde_json::Value::Object(obj) => {
                for v in obj.values() {
                    Self::collect_strings(v, texts);
                }
            }
            _ => {}
        }
    }
}
