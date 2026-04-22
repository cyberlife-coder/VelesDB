//! CSR (Compressed Sparse Row) graph representation for GPU traversal.
//!
//! Converts the HNSW Layer's per-node `RwLock<Vec<NodeId>>` adjacency lists
//! into a flat, GPU-friendly format suitable for coalesced memory access
//! in compute shaders.
//!
//! ## Layout
//!
//! ```text
//! offsets:   [0, 3, 5, 9, 11]           ← cumulative neighbor count (N+1 entries)
//! neighbors: [2,5,7, 0,3, 0,4,6,8, 1,5] ← all neighbors, concatenated
//! ```
//!
//! ## Cache Invalidation
//!
//! The [`CsrCache`] wraps a [`CsrGraph`] with a dirty flag that is set
//! whenever the underlying Layer is mutated (insert/delete). The CSR is
//! rebuilt lazily on the next GPU search request.

use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::RwLock;

use crate::index::hnsw::native::layer::Layer;

/// GPU-friendly CSR representation of a single HNSW layer's adjacency graph.
///
/// All data is stored in flat `Vec<u32>` arrays that can be uploaded to GPU
/// storage buffers via `wgpu::util::DeviceExt::create_buffer_init`.
#[derive(Debug, Clone)]
pub struct CsrGraph {
    /// Cumulative neighbor offsets: `offsets[node]..offsets[node+1]` gives
    /// the range of neighbors for `node` in the `neighbors` array.
    /// Length: `num_nodes + 1`.
    pub offsets: Vec<u32>,
    /// Concatenated neighbor IDs for all nodes. Length: total number of edges.
    pub neighbors: Vec<u32>,
    /// Total number of nodes in the graph.
    pub num_nodes: u32,
    /// Maximum degree (number of neighbors) across all nodes.
    pub max_degree: u32,
    /// Total number of edges (sum of all neighbor counts).
    pub total_edges: u32,
}

impl CsrGraph {
    /// Builds a CSR graph from a single HNSW [`Layer`].
    ///
    /// Acquires a read lock on each node's neighbor list sequentially.
    /// For 1M nodes with average degree 16, this takes ~50ms.
    ///
    /// # Arguments
    ///
    /// * `layer` — The HNSW layer to convert.
    /// * `num_nodes` — Number of active nodes (may be less than `layer.neighbors.len()`
    ///   if the layer was pre-allocated with extra capacity).
    #[must_use]
    pub fn from_layer(layer: &Layer, num_nodes: usize) -> Self {
        let n = num_nodes.min(layer.neighbors.len());
        let mut offsets = Vec::with_capacity(n + 1);
        // Pre-estimate: assume average degree of 16 for initial allocation
        let mut neighbors = Vec::with_capacity(n * 16);
        let mut max_degree: u32 = 0;

        offsets.push(0u32);
        for node_id in 0..n {
            let nbrs = layer.get_neighbors(node_id);
            #[allow(clippy::cast_possible_truncation)]
            let degree = nbrs.len() as u32;
            max_degree = max_degree.max(degree);
            for &nbr in &nbrs {
                // Reason: NodeId (usize) values are bounded by collection size,
                // which is validated to fit in u32 by the GPU dispatch threshold.
                #[allow(clippy::cast_possible_truncation)]
                let nbr_u32 = nbr as u32;
                neighbors.push(nbr_u32);
            }
            // Reason: neighbors.len() is bounded by n * max_degree, where both
            // n and max_degree are << u32::MAX for any practical HNSW index.
            #[allow(clippy::cast_possible_truncation)]
            let offset = neighbors.len() as u32;
            offsets.push(offset);
        }

        #[allow(clippy::cast_possible_truncation)]
        let total_edges = neighbors.len() as u32;
        #[allow(clippy::cast_possible_truncation)]
        let num_nodes_u32 = n as u32;

        CsrGraph {
            offsets,
            neighbors,
            num_nodes: num_nodes_u32,
            max_degree,
            total_edges,
        }
    }

    /// Returns true if this CSR graph has no nodes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.num_nodes == 0
    }

    /// Returns the byte size of the offsets buffer for GPU upload.
    #[must_use]
    pub fn offsets_byte_size(&self) -> usize {
        self.offsets.len() * std::mem::size_of::<u32>()
    }

    /// Returns the byte size of the neighbors buffer for GPU upload.
    #[must_use]
    pub fn neighbors_byte_size(&self) -> usize {
        self.neighbors.len() * std::mem::size_of::<u32>()
    }

    /// Returns the total GPU memory (VRAM) needed for CSR buffers alone.
    #[must_use]
    pub fn total_gpu_bytes(&self) -> usize {
        self.offsets_byte_size() + self.neighbors_byte_size()
    }

    /// Returns the graph density as a ratio of actual edges to maximum possible.
    ///
    /// For an undirected graph: `density = edges / (nodes * (nodes - 1))`.
    /// Values near 0 indicate sparse graphs (typical for HNSW), values near
    /// 1 indicate dense graphs.
    ///
    /// Used for GPU dispatch tuning: very sparse graphs have poor GPU
    /// occupancy due to uneven thread loads.
    #[must_use]
    pub fn density(&self) -> f64 {
        if self.num_nodes <= 1 {
            return 0.0;
        }
        let n = f64::from(self.num_nodes);
        let max_edges = n * (n - 1.0);
        if max_edges == 0.0 {
            return 0.0;
        }
        f64::from(self.total_edges) / max_edges
    }

    /// Returns the average degree of nodes in the graph.
    #[must_use]
    pub fn avg_degree(&self) -> f64 {
        if self.num_nodes == 0 {
            return 0.0;
        }
        f64::from(self.total_edges) / f64::from(self.num_nodes)
    }

    /// Validates CSR invariants in debug mode.
    ///
    /// Checks:
    /// 1. `offsets.len() == num_nodes + 1`
    /// 2. Offsets are monotonically non-decreasing
    /// 3. Last offset equals `total_edges`
    /// 4. All neighbor IDs are `< num_nodes`
    ///
    /// Returns `Ok(())` if all invariants hold, or `Err` with a description
    /// of the first violated invariant.
    ///
    /// # Errors
    ///
    /// Returns `Err(String)` describing the first violated invariant
    /// when `offsets.len()` is wrong, offsets are non-monotonic, the last
    /// offset disagrees with `total_edges`, or any neighbor ID is out of
    /// range. The error string is intended for diagnostics, not matching.
    pub fn validate(&self) -> Result<(), String> {
        // Check 1: offsets length
        let expected_len = self.num_nodes as usize + 1;
        if self.offsets.len() != expected_len {
            return Err(format!(
                "offsets.len()={} != num_nodes+1={}",
                self.offsets.len(),
                expected_len,
            ));
        }

        // Check 2: monotonicity
        for i in 1..self.offsets.len() {
            if self.offsets[i] < self.offsets[i - 1] {
                return Err(format!(
                    "offsets not monotonic at {}: {} < {}",
                    i,
                    self.offsets[i],
                    self.offsets[i - 1],
                ));
            }
        }

        // Check 3: last offset == total_edges
        if let Some(&last) = self.offsets.last() {
            if last != self.total_edges {
                return Err(format!(
                    "last offset {} != total_edges {}",
                    last, self.total_edges,
                ));
            }
        }

        // Check 4: neighbor bounds
        for (idx, &nbr) in self.neighbors.iter().enumerate() {
            if nbr >= self.num_nodes {
                return Err(format!(
                    "neighbor[{}]={} >= num_nodes={}",
                    idx, nbr, self.num_nodes,
                ));
            }
        }

        Ok(())
    }
}

impl std::fmt::Display for CsrGraph {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        #[allow(clippy::cast_precision_loss)]
        let vram_kb = self.total_gpu_bytes() as f64 / 1024.0;
        write!(
            f,
            "CsrGraph(nodes={}, edges={}, max_deg={}, avg_deg={:.1}, density={:.6}, vram={:.1}KB)",
            self.num_nodes,
            self.total_edges,
            self.max_degree,
            self.avg_degree(),
            self.density(),
            vram_kb,
        )
    }
}

/// Cached CSR graph with generation-based invalidation.
///
/// Thread-safe: the CSR data is behind a [`RwLock`] and invalidation uses
/// a monotonic generation counter to avoid ABA problems that a simple
/// dirty boolean would have.
///
/// ## Why not `AtomicBool`?
///
/// A boolean dirty flag has an ABA problem: if thread A reads `dirty=true`,
/// starts rebuilding, and thread B calls `invalidate()` during the rebuild,
/// thread B's `store(true)` is invisible (already true). Thread A's
/// `compare_exchange(true, false)` succeeds, clearing the flag — but the
/// cached CSR was built from a pre-mutation snapshot. The generation counter
/// detects this: A snapshots `gen=5`, B increments to `gen=6`, A's CAS on
/// `gen` fails because `5 != 6`, so the flag stays dirty.
pub struct CsrCache {
    /// The cached CSR graph. `None` if not yet built.
    csr: RwLock<Option<CsrGraph>>,
    /// Monotonically increasing generation counter.
    /// Incremented by `invalidate()` on every Layer mutation.
    generation: AtomicU64,
    /// Generation at which the cached CSR was built.
    /// If `built_generation != generation`, the cache is stale.
    built_generation: AtomicU64,
    /// Public version counter, incremented on each successful rebuild.
    version: AtomicU64,
}

impl CsrCache {
    /// Creates a new, empty CSR cache.
    #[must_use]
    pub fn new() -> Self {
        Self {
            csr: RwLock::new(None),
            // Start at gen=1, built_gen=0 → cache starts stale
            generation: AtomicU64::new(1),
            built_generation: AtomicU64::new(0),
            version: AtomicU64::new(0),
        }
    }

    /// Returns true if the cache is stale (needs rebuild).
    #[inline]
    fn is_stale(&self) -> bool {
        self.generation.load(Ordering::Acquire) != self.built_generation.load(Ordering::Acquire)
    }

    /// Marks the cache as dirty. Called after any Layer mutation (insert/delete).
    ///
    /// Each call increments the generation counter, ensuring that concurrent
    /// rebuilds can detect the mutation even if it occurs during the rebuild.
    pub fn invalidate(&self) {
        self.generation.fetch_add(1, Ordering::Release);
    }

    /// Returns the current version counter.
    #[must_use]
    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Acquire)
    }

    /// Returns a clone of the cached CSR graph, rebuilding if stale.
    ///
    /// This method is designed for the GPU dispatch hot path:
    /// - If the cache is fresh, returns the existing CSR (fast path).
    /// - If stale, rebuilds from the Layer (slow path, ~50ms for 1M nodes).
    ///
    /// The rebuild is generation-safe: if a concurrent `invalidate()` occurs
    /// during the rebuild, the stale CSR is still stored but the generation
    /// check ensures the *next* query will re-trigger a rebuild.
    pub fn get_or_rebuild(&self, layer: &Layer, num_nodes: usize) -> CsrGraph {
        // Fast path: check if cache is fresh
        if !self.is_stale() {
            let guard = self.csr.read();
            if let Some(ref csr) = *guard {
                return csr.clone();
            }
        }

        // Snapshot generation before rebuild
        let gen_before = self.generation.load(Ordering::Acquire);

        // Slow path: rebuild
        let new_csr = CsrGraph::from_layer(layer, num_nodes);

        // Update cache
        {
            let mut guard = self.csr.write();
            *guard = Some(new_csr.clone());
        }

        // Only mark as fresh if no concurrent invalidation occurred.
        // CAS on generation: if gen is still what we saw before rebuild,
        // no mutation happened and we can mark built_generation = gen.
        // If gen changed, skip — the cache stays stale and the next
        // query will trigger another rebuild.
        let gen_after = self.generation.load(Ordering::Acquire);
        if gen_after == gen_before {
            self.built_generation.store(gen_before, Ordering::Release);
            self.version.fetch_add(1, Ordering::AcqRel);
        }

        new_csr
    }

    /// Returns a clone of the cached CSR if available and fresh.
    ///
    /// Returns `None` if the cache is stale or not yet built.
    /// Used by non-critical paths that don't want to pay the rebuild cost.
    #[must_use]
    pub fn get_if_clean(&self) -> Option<CsrGraph> {
        if self.is_stale() {
            return None;
        }
        self.csr.read().clone()
    }
}

impl Default for CsrCache {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::hnsw::native::NodeId;

    #[test]
    fn test_csr_from_empty_layer() {
        let layer = Layer::new(0);
        let csr = CsrGraph::from_layer(&layer, 0);
        assert!(csr.is_empty());
        assert_eq!(csr.offsets, vec![0]);
        assert!(csr.neighbors.is_empty());
        assert_eq!(csr.max_degree, 0);
        assert_eq!(csr.total_edges, 0);
    }

    #[test]
    fn test_csr_from_simple_layer() {
        let layer = Layer::new(4);
        layer.set_neighbors(0, vec![1, 2]);
        layer.set_neighbors(1, vec![0, 3]);
        layer.set_neighbors(2, vec![0, 1, 3]);
        layer.set_neighbors(3, vec![1, 2]);

        let csr = CsrGraph::from_layer(&layer, 4);
        assert_eq!(csr.num_nodes, 4);
        assert_eq!(csr.offsets, vec![0, 2, 4, 7, 9]);
        assert_eq!(csr.neighbors, vec![1, 2, 0, 3, 0, 1, 3, 1, 2]);
        assert_eq!(csr.max_degree, 3);
        assert_eq!(csr.total_edges, 9);
    }

    #[test]
    fn test_csr_neighbor_lookup() {
        let layer = Layer::new(3);
        layer.set_neighbors(0, vec![1, 2]);
        layer.set_neighbors(1, vec![]);
        layer.set_neighbors(2, vec![0]);

        let csr = CsrGraph::from_layer(&layer, 3);

        // Node 0: neighbors at offsets[0]..offsets[1] = 0..2
        assert_eq!(
            &csr.neighbors[csr.offsets[0] as usize..csr.offsets[1] as usize],
            &[1, 2]
        );
        // Node 1: neighbors at offsets[1]..offsets[2] = 2..2 (empty)
        assert_eq!(
            &csr.neighbors[csr.offsets[1] as usize..csr.offsets[2] as usize],
            &[] as &[u32]
        );
        // Node 2: neighbors at offsets[2]..offsets[3] = 2..3
        assert_eq!(
            &csr.neighbors[csr.offsets[2] as usize..csr.offsets[3] as usize],
            &[0]
        );
    }

    #[test]
    fn test_csr_cache_dirty_flag() {
        let cache = CsrCache::new();
        assert_eq!(cache.version(), 0);

        let layer = Layer::new(2);
        layer.set_neighbors(0, vec![1]);
        layer.set_neighbors(1, vec![0]);

        // First build
        let csr = cache.get_or_rebuild(&layer, 2);
        assert_eq!(csr.num_nodes, 2);
        assert_eq!(cache.version(), 1);

        // Should return cached (not rebuild)
        let csr2 = cache.get_or_rebuild(&layer, 2);
        assert_eq!(csr2.num_nodes, 2);
        assert_eq!(cache.version(), 1); // Same version

        // Invalidate and rebuild
        cache.invalidate();
        let csr3 = cache.get_or_rebuild(&layer, 2);
        assert_eq!(csr3.num_nodes, 2);
        assert_eq!(cache.version(), 2); // Incremented
    }

    #[test]
    fn test_csr_byte_sizes() {
        let layer = Layer::new(100);
        for i in 0..100 {
            let neighbors: Vec<NodeId> = (0..16).map(|j| (i + j + 1) % 100).collect();
            layer.set_neighbors(i, neighbors);
        }

        let csr = CsrGraph::from_layer(&layer, 100);
        assert_eq!(csr.offsets_byte_size(), 101 * 4); // (N+1) * sizeof(u32)
        assert_eq!(csr.neighbors_byte_size(), 1600 * 4); // 100 * 16 * sizeof(u32)
        assert_eq!(csr.total_gpu_bytes(), 101 * 4 + 1600 * 4);
    }

    #[test]
    fn test_csr_partial_capacity() {
        // Layer pre-allocated for 100 but only 5 nodes are active
        let layer = Layer::new(100);
        layer.set_neighbors(0, vec![1, 2]);
        layer.set_neighbors(1, vec![0]);

        let csr = CsrGraph::from_layer(&layer, 5);
        assert_eq!(csr.num_nodes, 5);
        // Nodes 2..4 should have zero neighbors
        assert_eq!(csr.offsets[2], csr.offsets[3]);
        assert_eq!(csr.offsets[3], csr.offsets[4]);
        assert_eq!(csr.offsets[4], csr.offsets[5]);
    }

    #[test]
    fn test_get_if_clean_returns_none_when_dirty() {
        let cache = CsrCache::new();
        assert!(cache.get_if_clean().is_none()); // Starts dirty

        let layer = Layer::new(1);
        cache.get_or_rebuild(&layer, 1); // Build it
        assert!(cache.get_if_clean().is_some()); // Now clean

        cache.invalidate();
        assert!(cache.get_if_clean().is_none()); // Dirty again
    }
}
