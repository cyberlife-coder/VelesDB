//! HNSW insert operations.

use super::super::distance::DistanceEngine;
use super::super::layer::{Layer, NodeId};
use super::{NativeHnsw, NO_ENTRY_POINT};
use crate::perf_optimizations::ContiguousVectors;
use std::borrow::Cow;
use std::sync::atomic::Ordering;

/// Result of [`NativeHnsw::allocate_batch`]: `(NodeId, layer)` pairs and
/// the pre-processed query vectors (normalized for cosine, borrowed otherwise).
type BatchAllocation<'a> = (Vec<(NodeId, usize)>, Vec<Cow<'a, [f32]>>);

impl<D: DistanceEngine> NativeHnsw<D> {
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

    /// Pre-creates layers and allocates node capacity for an upcoming batch.
    ///
    /// Uses a statistical upper bound for the max expected layer:
    /// `ceil(log_M(total_nodes)) + 2`, capped at 15.
    // Reason: cast_precision_loss acceptable for statistical bound calculation
    // Reason: cast_possible_truncation result is capped at 15
    // Reason: cast_sign_loss log of positive numbers is positive
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    pub(in crate::index::hnsw::native) fn pre_expand_layers(&self, total_nodes: usize) {
        let max_layer = if self.max_connections > 1 && total_nodes > 1 {
            let log_m = (total_nodes as f64).ln() / (self.max_connections as f64).ln();
            (log_m.ceil() as usize + 2).min(15)
        } else {
            15
        };

        let mut layers = self.layers.write();
        while layers.len() <= max_layer {
            layers.push(Layer::new(total_nodes));
        }
        for layer in layers.iter_mut() {
            layer.ensure_capacity(total_nodes.saturating_sub(1));
        }
        self.pre_allocated_capacity
            .store(total_nodes, Ordering::Relaxed);
    }

    /// Ensures all layers up to `node_layer` exist and have capacity for `node_id`.
    fn expand_layers(&self, node_id: NodeId, node_layer: usize) {
        // Fast path: skip write lock if pre-allocation covers this insert
        if node_id < self.pre_allocated_capacity.load(Ordering::Relaxed)
            && node_layer < self.layers.read().len()
        {
            return;
        }
        // Slow path: acquire write lock (rare after pre-allocation)
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
    /// Accepts a borrowed slice to avoid forcing callers to clone. For cosine
    /// metric with pre-normalization, a temporary copy is made internally;
    /// for all other metrics the slice is used directly (zero-copy).
    ///
    /// # Errors
    ///
    /// Returns an error if vector storage allocation or insertion fails.
    pub fn insert(&self, vector: &[f32]) -> crate::error::Result<NodeId> {
        let query = self.prepare_query(vector);

        let node_id = self.allocate_and_store_vector(&query)?;
        let node_layer = self.random_layer();
        self.expand_layers(node_id, node_layer);

        let ep = self.entry_point.load(Ordering::Acquire);
        if ep != NO_ENTRY_POINT {
            self.insert_with_entry_point(node_id, &query, node_layer, ep);
        }

        self.promote_entry_point(node_id, node_layer);
        self.count.fetch_add(1, Ordering::Relaxed);
        Ok(node_id)
    }

    /// Atomically updates the entry point if the index is empty or the node
    /// reaches a higher layer than the current maximum.
    ///
    /// Uses lock-free CAS loops instead of a mutex. Entry-point promotion is
    /// extremely rare (O(log_M(N)) times per index lifetime), so the CAS loop
    /// almost never retries. Two separate CAS operations handle the two cases:
    ///
    /// 1. **Empty index**: CAS on `entry_point` from `NO_ENTRY_POINT` to `node_id`.
    /// 2. **Layer promotion**: CAS on `max_layer` from `current_max` to `node_layer`.
    ///    Only the CAS winner updates `entry_point`, ensuring consistency.
    ///
    /// Between `max_layer` CAS success and `entry_point` store, a concurrent
    /// reader may see the new `max_layer` with the old `entry_point`. This is
    /// safe: `search_layer_single` returns `None` (via `with_neighbors`) for
    /// layers where the old EP has no edges, causing a no-op greedy descent.
    pub(in crate::index::hnsw::native) fn promote_entry_point(
        &self,
        node_id: NodeId,
        node_layer: usize,
    ) {
        // Case 1: First insert — race to set entry_point from NO_ENTRY_POINT.
        if self.entry_point.load(Ordering::Acquire) == NO_ENTRY_POINT {
            // CAS: only one thread wins the first-insert race.
            if self
                .entry_point
                .compare_exchange(NO_ENTRY_POINT, node_id, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                self.max_layer.store(node_layer, Ordering::Release);
                return;
            }
            // Another thread won — fall through to layer promotion check.
        }

        // Case 2: Layer promotion — CAS loop on max_layer.
        loop {
            let current_max = self.max_layer.load(Ordering::Acquire);
            if node_layer <= current_max {
                break; // No promotion needed — most common case.
            }
            // Try to atomically claim the new max_layer. Only the CAS
            // winner updates entry_point; losers retry the loop.
            if self
                .max_layer
                .compare_exchange(current_max, node_layer, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                self.entry_point.store(node_id, Ordering::Release);
                break;
            }
        }
    }

    /// Batch-allocates vectors and assigns random layers.
    ///
    /// Returns `(assignments, processed_queries)` where:
    /// - `assignments`: `(NodeId, layer)` pairs for each vector
    /// - `processed_queries`: normalized query vectors (reusable in Phase B)
    ///
    /// This is Phase A of the two-phase batch insertion: all vectors are stored
    /// and layers expanded in single lock scopes. The caller is responsible for
    /// connecting nodes (Phase B) and updating `entry_point`/`count` (Phase C).
    ///
    /// # Lock Strategy (I2 optimization)
    ///
    /// The vector write lock is acquired in two separate scopes:
    /// 1. **Capacity reservation** — initializes storage and pre-grows the buffer
    ///    if needed. May trigger an expensive realloc+copy on the cold path.
    /// 2. **Bulk push** — copies vectors into pre-reserved space. Guaranteed
    ///    fast memcpy with no reallocation.
    ///
    /// # Errors
    ///
    /// Returns an error if vector storage allocation fails.
    pub(in crate::index::hnsw::native) fn allocate_batch<'a>(
        &self,
        vectors: &[&'a [f32]],
    ) -> crate::error::Result<BatchAllocation<'a>> {
        if vectors.is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }

        let processed: Vec<Cow<'a, [f32]>> =
            vectors.iter().map(|v| self.prepare_query(v)).collect();

        // Pre-expand layers before vectors (lock ordering safe: layers only)
        let current_len = self
            .vectors
            .read()
            .as_ref()
            .map_or(0, ContiguousVectors::len);
        self.pre_expand_layers(current_len + vectors.len());

        // Scope 1: Initialize storage and reserve capacity (may resize — cold path)
        self.reserve_vector_capacity(&processed, vectors.len())?;

        // Scope 2: Bulk push into pre-reserved space (fast memcpy — no resize)
        let first_id = self.bulk_push_vectors(&processed)?;

        let assignments: Vec<(NodeId, usize)> = (0..vectors.len())
            .map(|i| (first_id + i, self.random_layer()))
            .collect();

        Ok((assignments, processed))
    }

    /// Initializes vector storage if needed and pre-reserves capacity.
    ///
    /// Cold path: the write lock may be held during a buffer resize.
    fn reserve_vector_capacity(
        &self,
        processed: &[Cow<'_, [f32]>],
        batch_size: usize,
    ) -> crate::error::Result<()> {
        let mut guard = self.vectors.write();
        if guard.is_none() {
            *guard = Some(ContiguousVectors::new(
                processed[0].len(),
                batch_size.max(16),
            )?);
        }
        let storage = guard.as_mut().ok_or_else(|| {
            crate::error::Error::Internal("Vector storage missing after init".to_string())
        })?;
        storage.reserve_additional(batch_size)?;
        Ok(())
    }

    /// Pushes all processed vectors into pre-reserved storage.
    ///
    /// Fast path: write lock held only for bulk memcpy, no reallocation.
    fn bulk_push_vectors(&self, processed: &[Cow<'_, [f32]>]) -> crate::error::Result<NodeId> {
        let mut guard = self.vectors.write();
        let storage = guard.as_mut().ok_or_else(|| {
            crate::error::Error::Internal("Vector storage missing after reserve".to_string())
        })?;
        let first = storage.len();
        let slices: Vec<&[f32]> = processed.iter().map(AsRef::as_ref).collect();
        storage.push_batch(&slices)?;
        Ok(first)
    }

    /// Greedy descent through upper HNSW layers above `node_layer` to find
    /// the best entry point for the target layers.
    pub(in crate::index::hnsw::native) fn greedy_descent_upper_layers(
        &self,
        query: &[f32],
        node_layer: usize,
        mut entry_point: NodeId,
    ) -> NodeId {
        let max_layer = self.max_layer.load(Ordering::Relaxed);
        for layer_idx in (node_layer + 1..=max_layer).rev() {
            entry_point = self.search_layer_single(query, entry_point, layer_idx);
        }
        entry_point
    }

    /// Connects a node into the HNSW graph at layers 0..=`node_layer`.
    ///
    /// Searches for neighbors at each layer, selects the best candidates,
    /// and creates bidirectional connections.
    pub(in crate::index::hnsw::native) fn connect_node(
        &self,
        node_id: NodeId,
        query: &[f32],
        node_layer: usize,
        entry_point: NodeId,
    ) {
        self.connect_node_with_ef(
            node_id,
            query,
            node_layer,
            entry_point,
            self.ef_construction,
            0,
        );
    }

    /// Connects a node into the HNSW graph using a caller-specified ef budget.
    ///
    /// Used by `connect_batch_chunked` to apply adaptive `ef_construction`
    /// reduction during bulk insert (lower search budget for large batches)
    /// without affecting single-vector insert or the stored `ef_construction`.
    pub(in crate::index::hnsw::native) fn connect_node_with_ef(
        &self,
        node_id: NodeId,
        query: &[f32],
        node_layer: usize,
        mut entry_point: NodeId,
        effective_ef: usize,
        stagnation: usize,
    ) {
        for layer_idx in (0..=node_layer).rev() {
            let max_conn = if layer_idx == 0 {
                self.max_connections_0
            } else {
                self.max_connections
            };
            let neighbors = self.search_layer(
                query,
                &[entry_point],
                effective_ef,
                layer_idx,
                stagnation,
                None,
            );
            let selected = self.select_neighbors(&neighbors, max_conn);
            self.connect_neighbors_batch(node_id, &selected, layer_idx, max_conn);
            if !neighbors.is_empty() {
                entry_point = neighbors[0].0;
            }
        }
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
        let current_ep = self.greedy_descent_upper_layers(query, node_layer, ep);
        self.connect_node(node_id, query, node_layer, current_ep);
    }
}
