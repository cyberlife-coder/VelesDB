//! HNSW insert operations.

use super::super::distance::DistanceEngine;
use super::super::layer::{Layer, NodeId};
use super::NativeHnsw;
use std::sync::atomic::Ordering;

impl<D: DistanceEngine> NativeHnsw<D> {
    /// Normalizes the vector in-place if the distance engine uses cosine metric
    /// with pre-normalization.
    #[inline]
    fn normalize_if_cosine(&self, vector: &mut [f32]) {
        if self.distance.is_pre_normalized()
            && self.distance.metric() == crate::DistanceMetric::Cosine
        {
            crate::simd_native::normalize_inplace_native(vector);
        }
    }

    /// Allocates vector storage if needed and pushes the vector, returning its node ID.
    ///
    /// # Errors
    ///
    /// Returns an error if storage allocation or push fails.
    fn allocate_and_store_vector(&self, vector: &[f32]) -> crate::error::Result<NodeId> {
        let mut guard = self.vectors.write();
        if guard.is_none() {
            *guard = Some(crate::perf_optimizations::ContiguousVectors::new(
                vector.len(),
                16,
            )?);
        }
        let storage = guard.as_mut().ok_or_else(|| {
            crate::error::Error::Internal("Vector storage missing after init".to_string())
        })?;
        let id = storage.len();
        storage.push(vector)?;
        Ok(id)
    }

    /// Ensures all layers up to `node_layer` exist and have capacity for `node_id`.
    fn expand_layers(&self, node_id: NodeId, node_layer: usize) {
        let mut layers = self.layers.write();
        while layers.len() <= node_layer {
            layers.push(Layer::new(node_id + 1));
        }
        for layer in layers.iter_mut() {
            layer.ensure_capacity(node_id);
        }
    }

    /// Inserts a vector into the index.
    ///
    /// # Errors
    ///
    /// Returns an error if vector storage allocation or insertion fails.
    pub fn insert(&self, mut vector: Vec<f32>) -> crate::error::Result<NodeId> {
        self.normalize_if_cosine(&mut vector);

        let node_id = self.allocate_and_store_vector(&vector)?;
        // F-14: Use the original owned vector as the query — no clone needed
        // because `push` copies from a slice reference.
        let query = vector;
        let node_layer = self.random_layer();
        self.expand_layers(node_id, node_layer);

        let entry_point = *self.entry_point.read();
        if let Some(ep) = entry_point {
            self.insert_with_entry_point(node_id, &query, node_layer, ep);
        }

        // Update entry point: set if empty, or promote when node reaches a higher layer.
        // Single write-lock acquisition avoids redundant contention on `entry_point`.
        let current_max = self.max_layer.load(Ordering::Relaxed);
        if entry_point.is_none() || node_layer > current_max {
            *self.entry_point.write() = Some(node_id);
            if node_layer > current_max {
                self.max_layer.store(node_layer, Ordering::Relaxed);
            }
        }
        self.count.fetch_add(1, Ordering::Relaxed);
        Ok(node_id)
    }

    /// Performs the two-phase HNSW insertion when an entry point exists:
    /// 1. Greedy descent through upper layers above `node_layer`
    /// 2. Neighbor selection and bidirectional connection at layers 0..=`node_layer`
    #[inline]
    fn insert_with_entry_point(
        &self,
        node_id: NodeId,
        query: &[f32],
        node_layer: usize,
        ep: NodeId,
    ) {
        let mut current_ep = ep;
        let max_layer = self.max_layer.load(Ordering::Relaxed);

        // Phase 1: greedy descent through upper layers
        for layer_idx in (node_layer + 1..=max_layer).rev() {
            current_ep = self.search_layer_single(query, current_ep, layer_idx);
        }

        // Phase 2: search + connect at each layer from node_layer down to 0
        for layer_idx in (0..=node_layer).rev() {
            let max_conn = if layer_idx == 0 {
                self.max_connections_0
            } else {
                self.max_connections
            };
            let neighbors =
                // Stagnation disabled during construction (0) to ensure
                // optimal neighbor selection — see Devin review PR #336.
                self.search_layer(query, vec![current_ep], self.ef_construction, layer_idx, 0);
            let selected = self.select_neighbors(&neighbors, max_conn);
            self.connect_neighbors_batch(node_id, &selected, layer_idx, max_conn);
            if !neighbors.is_empty() {
                current_ep = neighbors[0].0;
            }
        }
    }
}
