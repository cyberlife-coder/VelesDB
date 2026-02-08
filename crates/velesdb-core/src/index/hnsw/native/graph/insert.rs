//! HNSW insert operations.

use super::super::distance::DistanceEngine;
use super::super::layer::{Layer, NodeId};
use super::NativeHnsw;
use std::sync::atomic::Ordering;

impl<D: DistanceEngine> NativeHnsw<D> {
    /// Inserts a vector into the index.
    pub fn insert(&self, vector: Vec<f32>) -> NodeId {
        let node_id = {
            let mut vectors = self.vectors.write();
            let id = vectors.len();
            vectors.push(vector);
            id
        };
        let node_layer = self.random_layer();
        {
            let mut layers = self.layers.write();
            while layers.len() <= node_layer {
                layers.push(Layer::new(node_id + 1));
            }
            for layer in layers.iter_mut() {
                layer.ensure_capacity(node_id);
            }
        }
        let entry_point = *self.entry_point.read();
        if let Some(ep) = entry_point {
            let mut current_ep = ep;
            let max_layer = self.max_layer.load(Ordering::Relaxed);
            for layer_idx in (node_layer + 1..=max_layer).rev() {
                current_ep =
                    self.search_layer_single(&self.get_vector(node_id), current_ep, layer_idx);
            }
            for layer_idx in (0..=node_layer).rev() {
                let neighbors = self.search_layer(
                    &self.get_vector(node_id),
                    vec![current_ep],
                    self.ef_construction,
                    layer_idx,
                );
                let max_conn = if layer_idx == 0 {
                    self.max_connections_0
                } else {
                    self.max_connections
                };
                let selected =
                    self.select_neighbors(&self.get_vector(node_id), &neighbors, max_conn);
                self.layers.read()[layer_idx].set_neighbors(node_id, selected.clone());
                for &neighbor in &selected {
                    self.add_bidirectional_connection(node_id, neighbor, layer_idx, max_conn);
                }
                if !neighbors.is_empty() {
                    current_ep = neighbors[0].0;
                }
            }
        } else {
            *self.entry_point.write() = Some(node_id);
        }
        if node_layer > self.max_layer.load(Ordering::Relaxed) {
            self.max_layer.store(node_layer, Ordering::Relaxed);
            *self.entry_point.write() = Some(node_id);
        }
        self.count.fetch_add(1, Ordering::Relaxed);
        node_id
    }
}
