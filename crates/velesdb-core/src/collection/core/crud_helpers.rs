//! Internal helpers for CRUD operations: quantization caching and secondary index updates.

use crate::collection::types::Collection;
use crate::index::{JsonValue, SecondaryIndex};
use crate::point::Point;
use crate::quantization::{
    BinaryQuantizedVector, PQVector, ProductQuantizer, QuantizedVector, StorageMode,
};

const PQ_TRAINING_SAMPLES: usize = 128;

fn auto_num_subspaces(dimension: usize) -> usize {
    let mut num_subspaces = 8usize;
    while num_subspaces > 1 && dimension % num_subspaces != 0 {
        num_subspaces /= 2;
    }
    num_subspaces.max(1)
}

impl Collection {
    /// Caches a quantized representation of `point`'s vector according to `storage_mode`.
    pub(crate) fn cache_quantized_vector(
        &self,
        point: &Point,
        storage_mode: StorageMode,
        sq8_cache: Option<&mut std::collections::HashMap<u64, QuantizedVector>>,
        binary_cache: Option<&mut std::collections::HashMap<u64, BinaryQuantizedVector>>,
        pq_cache: Option<&mut std::collections::HashMap<u64, PQVector>>,
    ) {
        match storage_mode {
            StorageMode::SQ8 => {
                if let Some(cache) = sq8_cache {
                    let quantized = QuantizedVector::from_f32(&point.vector);
                    cache.insert(point.id, quantized);
                }
            }
            StorageMode::Binary => {
                if let Some(cache) = binary_cache {
                    let quantized = BinaryQuantizedVector::from_f32(&point.vector);
                    cache.insert(point.id, quantized);
                }
            }
            StorageMode::ProductQuantization => {
                let mut quantizer_guard = self.pq_quantizer.write();
                let mut backfill_samples: Vec<(u64, Vec<f32>)> = Vec::new();

                if quantizer_guard.is_none() {
                    let mut buffer = self.pq_training_buffer.write();
                    buffer.push_back((point.id, point.vector.clone()));
                    if buffer.len() >= PQ_TRAINING_SAMPLES {
                        let training: Vec<Vec<f32>> =
                            buffer.iter().map(|(_, vector)| vector.clone()).collect();
                        let num_centroids = 256usize.min(training.len().max(2));
                        *quantizer_guard = ProductQuantizer::train(
                            &training,
                            auto_num_subspaces(point.vector.len()),
                            num_centroids,
                        )
                        .ok();
                        backfill_samples = buffer.drain(..).collect();
                    }
                }

                if let (Some(cache), Some(quantizer)) = (pq_cache, quantizer_guard.as_ref()) {
                    for (id, vector) in backfill_samples {
                        if let Ok(code) = quantizer.quantize(&vector) {
                            cache.insert(id, code);
                        }
                    }

                    if let Ok(code) = quantizer.quantize(&point.vector) {
                        cache.insert(point.id, code);
                    }
                }
            }
            StorageMode::Full | StorageMode::RaBitQ => {}
        }
    }

    pub(crate) fn update_secondary_indexes_on_upsert(
        &self,
        id: u64,
        old_payload: Option<&serde_json::Value>,
        new_payload: Option<&serde_json::Value>,
    ) {
        let indexes = self.secondary_indexes.read();
        for (field, index) in indexes.iter() {
            if let Some(old_value) = old_payload
                .and_then(|p| p.get(field))
                .and_then(JsonValue::from_json)
            {
                self.remove_from_secondary_index(index, &old_value, id);
            }
            if let Some(new_value) = new_payload
                .and_then(|p| p.get(field))
                .and_then(JsonValue::from_json)
            {
                self.insert_into_secondary_index(index, new_value, id);
            }
        }
    }

    pub(crate) fn update_secondary_indexes_on_delete(
        &self,
        id: u64,
        old_payload: Option<&serde_json::Value>,
    ) {
        let Some(payload) = old_payload else {
            return;
        };
        let indexes = self.secondary_indexes.read();
        for (field, index) in indexes.iter() {
            if let Some(old_value) = payload.get(field).and_then(JsonValue::from_json) {
                self.remove_from_secondary_index(index, &old_value, id);
            }
        }
    }

    // These methods take `&self` for consistency with the impl block calling convention,
    // but the operations are logically index-directed and do not need instance state.
    #[allow(clippy::unused_self)]
    fn insert_into_secondary_index(&self, index: &SecondaryIndex, key: JsonValue, id: u64) {
        match index {
            SecondaryIndex::BTree(tree) => {
                let mut tree = tree.write();
                let ids = tree.entry(key).or_default();
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
        }
    }

    #[allow(clippy::unused_self)]
    fn remove_from_secondary_index(&self, index: &SecondaryIndex, key: &JsonValue, id: u64) {
        match index {
            SecondaryIndex::BTree(tree) => {
                let mut tree = tree.write();
                if let Some(ids) = tree.get_mut(key) {
                    ids.retain(|existing| *existing != id);
                    if ids.is_empty() {
                        tree.remove(key);
                    }
                }
            }
        }
    }
}
