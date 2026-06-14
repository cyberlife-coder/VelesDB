//! Graph API methods for Collection (EPIC-015 US-001).
//!
//! Exposes Knowledge Graph operations on Collection for use by
//! Tauri plugin, REST API, and other consumers.

use rustc_hash::{FxHashMap, FxHashSet};

use crate::collection::graph::{GraphEdge, GraphSchema, TraversalConfig, TraversalResult};
use crate::collection::types::Collection;
use crate::error::{Error, Result};
use crate::index::VectorIndex;
use crate::point::{Point, SearchResult};
use crate::storage::{PayloadStorage, VectorStorage};

use super::graph_property_index_wiring::extract_labels;
use super::graph_traversal_helpers::{
    bfs_pop, bfs_push, dfs_pop, dfs_push, expand_dfs_neighbors, reconstruct_path,
    traverse_with_frontier, DfsFrontier, TraversalEntry, TraversalParams,
};

// Traversal helper functions are in graph_traversal_helpers.rs

impl Collection {
    /// Adds an edge to the collection's knowledge graph.
    ///
    /// # Arguments
    ///
    /// * `edge` - The edge to add (id, source, target, label, properties)
    ///
    /// # Errors
    ///
    /// Returns `Error::EdgeExists` if an edge with the same ID already exists.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use velesdb_core::collection::graph::GraphEdge;
    ///
    /// let edge = GraphEdge::new(1, 100, 200, "KNOWS")?;
    /// collection.add_edge(edge)?;
    /// ```
    pub fn add_edge(&self, edge: GraphEdge) -> Result<()> {
        // Strict-schema referential integrity: reject before any mutation so a
        // violation leaves no partial write and does not bump write_generation.
        // Schemaless collections (the default) short-circuit at zero added cost.
        self.validate_edge_referential_integrity(&edge)?;

        let edge_id = edge.id();
        let rel_type = edge.label().to_string();
        let properties = edge.properties().clone();

        // WAL-before-apply (crash durability): log the edge to the edge WAL
        // before mutating the in-memory store, so a crash between the two
        // replays the edge on the next open. Unconditional: the append only
        // happens when an edge IS written, so collections that never use the
        // graph dimension pay nothing — and edges on vector collections
        // (e.g. agent-memory relations) are as durable as on graph ones.
        //
        // The WAL lock spans append + apply so the WAL order always equals
        // the store-apply order (replay must resolve id collisions exactly
        // like live execution) and concurrent appends can never interleave
        // a multi-write entry.
        let _wal_guard = self.edge_wal_lock.lock();
        #[cfg(feature = "persistence")]
        crate::collection::graph::edge_wal::wal_append_add(
            &crate::collection::graph::edge_wal::wal_path_for_edges(&self.path),
            &edge,
        )?;

        self.edge_store.add_edge(edge)?;

        // Populate edge property indexes (EPIC-047).
        self.index_edge_properties(edge_id, &rel_type, &properties);

        // Bump write generation so any cached plan for this collection is
        // invalidated on the next query (CACHE-01).
        self.write_generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    /// Adds multiple edges in batch with crash-durable WAL logging.
    ///
    /// Appends one ADD record per edge to the edge WAL (single open +
    /// fsync) BEFORE applying the batch to the in-memory store, then
    /// populates edge property indexes for the successfully added edges.
    ///
    /// # Returns
    ///
    /// Number of edges successfully added (duplicates are skipped).
    ///
    /// # Errors
    ///
    /// Returns an error if WAL logging fails (fail-closed: the in-memory
    /// store is not mutated when the WAL append fails).
    pub fn add_edges_batch(&self, edges: Vec<GraphEdge>) -> Result<usize> {
        if edges.is_empty() {
            return Ok(0);
        }

        // Strict-schema referential integrity: validate the ENTIRE batch before
        // any mutation (WAL append or store write), so a single violating edge
        // fails the whole batch with no partial write and no orphaned WAL entry.
        // Schemaless collections (the default) short-circuit at zero added cost.
        for edge in &edges {
            self.validate_edge_referential_integrity(edge)?;
        }

        // Unconditional WAL (see add_edge): writing edges implies wanting
        // them back after a restart, whatever the collection type. The WAL
        // lock spans append + apply (see add_edge).
        let _wal_guard = self.edge_wal_lock.lock();
        #[cfg(feature = "persistence")]
        crate::collection::graph::edge_wal::wal_append_add_batch(
            &crate::collection::graph::edge_wal::wal_path_for_edges(&self.path),
            &edges,
        )?;

        // Capture property metadata before the edges are moved into the store.
        let index_meta: Vec<(u64, String, _)> = edges
            .iter()
            .filter(|e| !e.properties().is_empty())
            .map(|e| (e.id(), e.label().to_string(), e.properties().clone()))
            .collect();

        let count = self.edge_store.add_edges_batch(edges);

        for (edge_id, rel_type, properties) in &index_meta {
            self.index_edge_properties(*edge_id, rel_type, properties);
        }

        if count > 0 {
            self.write_generation
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        Ok(count)
    }

    /// Replays the edge WAL on top of the loaded `edge_store` snapshot,
    /// re-running edge-property indexing for replayed ADD entries.
    ///
    /// Called from `Collection::open` AFTER `assemble`, so the snapshot is
    /// already loaded into `self.edge_store`. A missing WAL file is a cheap
    /// no-op (legacy DBs predate this feature). The edge store is mutated
    /// in place via its `&self` methods; the closure indexes edge
    /// properties into `self.edge_range_indexes`.
    ///
    /// # Errors
    ///
    /// Returns an error if the WAL file exists but cannot be read.
    #[cfg(feature = "persistence")]
    pub(crate) fn replay_edge_wal(&self) -> Result<u64> {
        use crate::collection::graph::edge_wal::{wal_path_for_edges, wal_replay, ReplayOp};
        let wal_path = wal_path_for_edges(&self.path);
        let replayed = wal_replay(&wal_path, &self.edge_store, |op| {
            if let ReplayOp::Add(edge) = op {
                if !edge.properties().is_empty() {
                    self.index_edge_properties(edge.id(), edge.label(), edge.properties());
                }
            }
        })?;
        if replayed > 0 {
            // Refresh the CSR read snapshot so traversals see replayed edges.
            self.edge_store.build_read_snapshot();
        }
        Ok(replayed)
    }

    /// Enforces strict-schema node-type validation for a node payload write.
    ///
    /// In schemaless mode (the default) or when the payload carries no
    /// `_labels`, this is a no-op. In strict mode every declared label is
    /// checked against the schema before any mutation takes place, so an
    /// undeclared node type is rejected atomically with no partial write.
    ///
    /// # Errors
    ///
    /// Returns `Error::SchemaValidation` if any label in `_labels` is not
    /// declared in the strict schema.
    fn validate_node_labels_against_schema(&self, payload: &serde_json::Value) -> Result<()> {
        let schema = match self.config.read().graph_schema.clone() {
            Some(s) if !s.is_schemaless() => s,
            _ => return Ok(()),
        };

        for label in extract_labels(payload) {
            schema.validate_node_type(&label)?;
        }
        Ok(())
    }

    /// Enforces strict-schema referential integrity for an edge write.
    ///
    /// In schemaless mode (the default), this is a no-op and returns
    /// immediately. In strict mode it verifies that both endpoint nodes exist
    /// and that the edge type / endpoint types satisfy the declared schema.
    ///
    /// # Errors
    ///
    /// Returns `Error::SchemaValidation` if an endpoint node is missing, has no
    /// `_labels`, or the edge violates the schema's edge-type constraints.
    fn validate_edge_referential_integrity(&self, edge: &GraphEdge) -> Result<()> {
        let schema = match self.config.read().graph_schema.clone() {
            Some(s) if !s.is_schemaless() => s,
            _ => return Ok(()),
        };

        let from_type = self.endpoint_node_type(edge.source())?;
        let to_type = self.endpoint_node_type(edge.target())?;
        schema.validate_edge_type(edge.label(), &from_type, &to_type)
    }

    /// Resolves the node type (first `_labels` entry) for a graph node,
    /// requiring the node to exist and carry a label (strict-mode helper).
    ///
    /// # Errors
    ///
    /// Returns `Error::SchemaValidation` if the node has no stored payload
    /// (referential integrity violation) or the payload declares no `_labels`.
    fn endpoint_node_type(&self, node_id: u64) -> Result<String> {
        let payload = self.payload_storage.read().retrieve(node_id)?;
        let Some(payload) = payload else {
            return Err(Error::SchemaValidation(format!(
                "edge references non-existent node {node_id}"
            )));
        };
        extract_labels(&payload)
            .into_iter()
            .next()
            .ok_or_else(|| Error::SchemaValidation(format!("node {node_id} has no '_labels' type")))
    }

    /// Gets all edges from the collection's knowledge graph.
    ///
    /// Note: This iterates through all stored edges. For large graphs,
    /// consider using `get_edges_by_label` or `get_outgoing_edges` for
    /// more targeted queries.
    ///
    /// # Returns
    ///
    /// Vector of all edges in the graph (cloned).
    #[must_use]
    pub fn get_all_edges(&self) -> Vec<GraphEdge> {
        self.edge_store.all_edges()
    }

    /// Gets edges filtered by label.
    ///
    /// # Arguments
    ///
    /// * `label` - The edge label (relationship type) to filter by
    ///
    /// # Returns
    ///
    /// Vector of edges with the specified label (cloned).
    #[must_use]
    pub fn get_edges_by_label(&self, label: &str) -> Vec<GraphEdge> {
        self.edge_store.get_edges_by_label(label)
    }

    /// Gets outgoing edges from a specific node.
    ///
    /// # Arguments
    ///
    /// * `node_id` - The source node ID
    ///
    /// # Returns
    ///
    /// Vector of edges originating from the specified node (cloned).
    #[must_use]
    pub fn get_outgoing_edges(&self, node_id: u64) -> Vec<GraphEdge> {
        self.edge_store.get_outgoing(node_id)
    }

    /// Gets incoming edges to a specific node.
    ///
    /// # Arguments
    ///
    /// * `node_id` - The target node ID
    ///
    /// # Returns
    ///
    /// Vector of edges pointing to the specified node (cloned).
    #[must_use]
    pub fn get_incoming_edges(&self, node_id: u64) -> Vec<GraphEdge> {
        self.edge_store.get_incoming(node_id)
    }

    /// Traverses the graph using BFS from a source node.
    ///
    /// # Arguments
    ///
    /// * `source` - Starting node ID
    /// * `max_depth` - Maximum traversal depth
    /// * `rel_types` - Optional filter by relationship types
    /// * `limit` - Maximum number of results
    ///
    /// # Returns
    ///
    /// Vector of traversal results with target nodes and paths.
    ///
    /// # Errors
    ///
    /// Returns an error if traversal fails.
    #[allow(dead_code)] // Reason: Called via GraphCollection inner delegation + tests
    #[allow(clippy::unnecessary_wraps)] // Reason: Public API contract — callers expect Result
    pub fn traverse_bfs(
        &self,
        source: u64,
        max_depth: u32,
        rel_types: Option<&[&str]>,
        limit: usize,
    ) -> Result<Vec<TraversalResult>> {
        let filter: &[&str] = rel_types.unwrap_or(&[]);
        let params = TraversalParams {
            store: &self.edge_store,
            filter,
            limit,
            max_depth,
            source,
        };
        let mut frontier = std::collections::VecDeque::new();
        frontier.push_back((source, 0u32));

        Ok(traverse_with_frontier(
            &params,
            bfs_pop,
            bfs_push,
            &mut frontier,
        ))
    }

    /// Traverses the graph using DFS from a source node.
    ///
    /// # Arguments
    ///
    /// * `source` - Starting node ID
    /// * `max_depth` - Maximum traversal depth
    /// * `rel_types` - Optional filter by relationship types
    /// * `limit` - Maximum number of results
    ///
    /// # Returns
    ///
    /// Vector of traversal results with target nodes and paths.
    ///
    /// # Errors
    ///
    /// Returns an error if traversal fails.
    #[allow(dead_code)] // Reason: Called via GraphCollection inner delegation + tests
    #[allow(clippy::unnecessary_wraps)] // Reason: Public API contract — callers expect Result
    pub fn traverse_dfs(
        &self,
        source: u64,
        max_depth: u32,
        rel_types: Option<&[&str]>,
        limit: usize,
    ) -> Result<Vec<TraversalResult>> {
        let filter: &[&str] = rel_types.unwrap_or(&[]);
        let params = TraversalParams {
            store: &self.edge_store,
            filter,
            limit,
            max_depth,
            source,
        };
        let mut frontier = vec![(source, 0u32)];

        Ok(traverse_with_frontier(
            &params,
            dfs_pop,
            dfs_push,
            &mut frontier,
        ))
    }

    /// Gets the in-degree and out-degree of a node.
    ///
    /// Uses degree counters instead of materializing edge vectors for O(1) lookup.
    ///
    /// # Arguments
    ///
    /// * `node_id` - The node ID
    ///
    /// # Returns
    ///
    /// Tuple of (`in_degree`, `out_degree`).
    #[must_use]
    pub fn get_node_degree(&self, node_id: u64) -> (usize, usize) {
        let in_degree = self.edge_store.incoming_degree(node_id);
        let out_degree = self.edge_store.outgoing_degree(node_id);
        (in_degree, out_degree)
    }

    /// Removes an edge from the graph by ID.
    ///
    /// # Arguments
    ///
    /// * `edge_id` - The edge ID to remove
    ///
    /// # Returns
    ///
    /// `true` if the edge existed and was removed, `false` if it didn't exist.
    #[must_use]
    pub fn remove_edge(&self, edge_id: u64) -> bool {
        // Cheap pre-check: a remove of a non-existent id must not create or
        // grow the WAL with junk tombstones (a racing remove between this
        // check and the append still replays as a harmless no-op).
        if !self.edge_store.contains_edge(edge_id) {
            return false;
        }
        // WAL-before-apply (crash durability): log the remove intent before
        // mutating the store. Fail-closed: if the WAL append fails we do NOT
        // mutate the store and report `false` (no panic — matches the
        // no-unwrap policy and the bool return contract). Unconditional for
        // the same reason as add_edge; the WAL lock spans append + apply.
        let _wal_guard = self.edge_wal_lock.lock();
        #[cfg(feature = "persistence")]
        if let Err(e) = crate::collection::graph::edge_wal::wal_append_remove(
            &crate::collection::graph::edge_wal::wal_path_for_edges(&self.path),
            edge_id,
        ) {
            tracing::error!("Edge WAL append remove failed for edge {edge_id}: {e}");
            return false;
        }

        // Atomic check-and-remove — no TOCTOU race.
        let removed = self.edge_store.remove_edge(edge_id);
        if removed {
            self.write_generation
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        removed
    }

    /// Returns the total number of edges in the graph.
    #[must_use]
    pub fn edge_count(&self) -> usize {
        self.edge_store.len()
    }

    /// Returns the highest edge id in the graph, if any (no edge cloning).
    pub(crate) fn max_edge_id(&self) -> Option<u64> {
        self.edge_store.max_edge_id()
    }

    /// Returns `true` when an edge with `edge_id` exists.
    pub(crate) fn edge_exists(&self, edge_id: u64) -> bool {
        self.edge_store.contains_edge(edge_id)
    }

    /// Rebuilds edge property indexes from edges already in the store.
    ///
    /// Called on open AFTER the edge snapshot is loaded and BEFORE the edge
    /// WAL replays (replay indexes its own ADDs), so snapshot-loaded edge
    /// properties become queryable again — previously they were only ever
    /// indexed at write time and silently lost once the WAL was truncated
    /// into the snapshot.
    pub(crate) fn reindex_edge_properties_from_store(&self) {
        for edge in self.edge_store.all_edges() {
            if !edge.properties().is_empty() {
                self.index_edge_properties(edge.id(), edge.label(), edge.properties());
            }
        }
    }

    // -------------------------------------------------------------------------
    // Graph schema
    // -------------------------------------------------------------------------

    /// Returns the graph schema stored in the collection config, if any.
    #[must_use]
    pub fn graph_schema(&self) -> Option<GraphSchema> {
        self.config.read().graph_schema.clone()
    }

    /// Returns `true` if this collection was created as a graph collection.
    #[must_use]
    #[allow(dead_code)] // Reason: Called via GraphCollection inner delegation + tests
    pub fn is_graph(&self) -> bool {
        self.config.read().graph_schema.is_some()
    }

    /// Returns `true` if this graph collection stores node embeddings.
    #[must_use]
    pub fn has_embeddings(&self) -> bool {
        self.config.read().embedding_dimension.is_some()
    }

    // -------------------------------------------------------------------------
    // Node payload (graph node properties)
    // -------------------------------------------------------------------------

    /// Stores a JSON payload for a graph node.
    ///
    /// Also maintains the label index: if the payload contains a `_labels`
    /// array, each label is indexed for O(1) lookup in `find_start_nodes()`.
    /// On update (existing node), old labels are removed before new ones
    /// are inserted.
    ///
    /// # Errors
    ///
    /// Returns an error if storage fails.
    pub fn store_node_payload(&self, node_id: u64, payload: &serde_json::Value) -> Result<()> {
        // Parity item E: gate the node payload size at the cold ingest boundary
        // before any mutation. Graph node writes bypass `enforce_upsert_limits`
        // (they take a raw `&Value`, not a `Point`), so apply the shared
        // payload-size gate here. `max_vectors_per_collection` is intentionally
        // not checked: vector-less node writes never touch `config.point_count`,
        // so a projected count would be meaningless on this path.
        Self::enforce_payload_value_size(node_id, payload, self.runtime_limits().max_payload_size)?;

        // Reject undeclared node types before any mutation. Schemaless and
        // payloads without `_labels` short-circuit at zero cost.
        // LOCK ORDER: payload_storage(3) → label_index(7) → graph_range_indexes(7).
        self.validate_node_labels_against_schema(payload)?;

        let mut storage = self.payload_storage.write();

        // Remove old labels and property indexes if this is an update.
        let mut label_idx = self.label_index.write();
        if let Ok(Some(old_payload)) = storage.retrieve(node_id) {
            label_idx.remove_from_payload(node_id, &old_payload);
            // Release label_index before touching graph property indexes
            // (both are at lock order 7, no ordering between them).
            drop(label_idx);
            self.deindex_node_properties(node_id, &old_payload);
            label_idx = self.label_index.write();
        }

        storage.store(node_id, payload)?;
        label_idx.index_from_payload(node_id, payload);
        drop(label_idx);
        drop(storage);

        // Populate graph property indexes (EPIC-047).
        self.index_node_properties(node_id, payload);

        // Node payload writes bypass the upsert mirror hooks — drop the
        // payload mirror so it can never serve stale columnar data.
        self.payload_mirror.invalidate();

        // Bump write generation so any cached plan for this collection is
        // invalidated on the next query (CACHE-01).
        self.write_generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    /// Retrieves the JSON payload for a graph node.
    ///
    /// # Errors
    ///
    /// Returns an error if retrieval fails.
    pub fn get_node_payload(&self, node_id: u64) -> Result<Option<serde_json::Value>> {
        Ok(self.payload_storage.read().retrieve(node_id)?)
    }

    // -------------------------------------------------------------------------
    // Graph traversal with TraversalConfig
    // -------------------------------------------------------------------------

    /// BFS traversal using the core `concurrent_bfs_stream` iterator.
    ///
    /// Wraps [`Self::traverse_bfs_config_inner`] with traversal metrics timing.
    #[must_use]
    pub fn traverse_bfs_config(
        &self,
        source_id: u64,
        config: &TraversalConfig,
    ) -> Vec<TraversalResult> {
        let start = std::time::Instant::now();
        let results = self.traverse_bfs_config_inner(source_id, config);
        self.edge_store
            .metrics()
            .record_traversal(start.elapsed(), results.len() as u64);
        results
    }

    /// Inner BFS traversal without metrics (see [`Self::traverse_bfs_config`]).
    #[must_use]
    fn traverse_bfs_config_inner(
        &self,
        source_id: u64,
        config: &TraversalConfig,
    ) -> Vec<TraversalResult> {
        use crate::collection::graph::{concurrent_bfs_stream, StreamingConfig, MAX_VISITED_SIZE};

        // Issue #905 debounce: only pay for the O(N+E) CSR rebuild when the
        // snapshot is already authoritative (no pending writes) or enough
        // writes have accumulated to make the rebuild worthwhile. While the
        // snapshot is stale-but-below-threshold we serve from the
        // authoritative per-shard streaming path instead of rebuilding on
        // every read — correct results, no stale data, no rebuild.
        if self.edge_store.csr_is_authoritative() || self.edge_store.csr_rebuild_due() {
            // Prefer the lock-free CSR snapshot path when available (Issue #491).
            let snapshot = self.edge_store.get_csr_snapshot();
            // Guard: only use the CSR path when the snapshot has been populated
            // (node_count > 0). An empty snapshot means no edges have been
            // ingested yet, so we fall through to the per-shard streaming BFS.
            if snapshot.node_count() > 0 {
                return crate::collection::graph::bfs_traverse_csr(&snapshot, source_id, config);
            }
        }

        // Fallback: streaming BFS via per-shard locks (also the correctness
        // path while the CSR rebuild is debounced).
        let streaming = StreamingConfig {
            max_depth: config.max_depth,
            rel_types: config.rel_types.clone(),
            limit: Some(config.limit),
            max_visited_size: MAX_VISITED_SIZE,
            deadline: config.deadline,
        };
        concurrent_bfs_stream(&self.edge_store, source_id, streaming)
            .filter(|result| result.depth >= config.min_depth)
            .take(config.limit)
            .collect()
    }

    /// DFS traversal (iterative) using `TraversalConfig`.
    ///
    /// Wraps [`Self::traverse_dfs_config_inner`] with traversal metrics timing.
    #[must_use]
    pub fn traverse_dfs_config(
        &self,
        source_id: u64,
        config: &TraversalConfig,
    ) -> Vec<TraversalResult> {
        let start = std::time::Instant::now();
        let results = self.traverse_dfs_config_inner(source_id, config);
        self.edge_store
            .metrics()
            .record_traversal(start.elapsed(), results.len() as u64);
        results
    }

    /// Inner DFS traversal without metrics (see [`Self::traverse_dfs_config`]).
    ///
    /// Uses parent-pointer map for zero-clone path reconstruction (G4).
    #[must_use]
    fn traverse_dfs_config_inner(
        &self,
        source_id: u64,
        config: &TraversalConfig,
    ) -> Vec<TraversalResult> {
        let rel_filter: FxHashSet<&str> = config.rel_types.iter().map(String::as_str).collect();

        let mut results = Vec::new();
        let mut visited: FxHashSet<u64> = FxHashSet::default();
        let mut parent_map: FxHashMap<u64, (u64, u64)> = FxHashMap::default();
        let mut stack: Vec<TraversalEntry> = vec![(source_id, 0)];

        // Start at the threshold so an already-expired deadline aborts on the
        // first pop; otherwise the clock is only read every N pops.
        let mut nodes_since_check = crate::collection::graph::DEADLINE_CHECK_INTERVAL;
        while let Some((node_id, depth)) = stack.pop() {
            if results.len() >= config.limit {
                break;
            }
            if crate::collection::graph::deadline_reached(config.deadline, &mut nodes_since_check) {
                break;
            }
            // Issue #906: bound the visited set / parent map. A highly-connected
            // graph with a high limit/max_depth would otherwise grow these
            // unboundedly (OOM). Consistent with the streaming iterators and
            // `traverse_with_frontier`: stop and return the bounded result.
            //
            // NOTE: this pop-time guard alone is insufficient for DFS — a single
            // high-out-degree hub fills `stack`/`parent_map` at PUSH time while
            // `visited` stays tiny. `expand_dfs_neighbors` therefore also caps
            // push-time growth at `MAX_VISITED_SIZE`.
            if visited.len() >= crate::collection::graph::MAX_VISITED_SIZE {
                break;
            }
            if !visited.insert(node_id) {
                continue;
            }
            if depth >= config.min_depth && depth > 0 {
                let path = reconstruct_path(&parent_map, source_id, node_id);
                results.push(TraversalResult::new(node_id, path, depth));
                if results.len() >= config.limit {
                    break;
                }
            }
            if depth < config.max_depth {
                let mut frontier = DfsFrontier {
                    stack: &mut stack,
                    parent_map: &mut parent_map,
                    max_pending: crate::collection::graph::MAX_VISITED_SIZE,
                };
                expand_dfs_neighbors(
                    &self.edge_store,
                    node_id,
                    depth,
                    &rel_filter,
                    &visited,
                    &mut frontier,
                );
            }
        }
        results
    }

    /// Parallel BFS traversal from multiple start nodes using rayon.
    ///
    /// When `start_nodes` exceeds the parallel threshold (100), rayon distributes
    /// independent per-start-node BFS traversals across CPU cores. Below the
    /// threshold, falls back to sequential execution.
    ///
    /// Results are deduplicated by path signature and truncated to `config.limit`.
    #[must_use]
    pub fn traverse_bfs_parallel(
        &self,
        start_nodes: &[u64],
        config: &TraversalConfig,
    ) -> Vec<TraversalResult> {
        use crate::collection::search::query::parallel_traversal::{
            ParallelConfig, ParallelTraverser,
        };

        let par_config = ParallelConfig::new()
            .with_max_depth(config.max_depth)
            .with_limit(config.limit);

        let traverser = ParallelTraverser::with_config(par_config);
        let edge_store = &self.edge_store;

        let rel_types = &config.rel_types;
        let adjacency = |node: u64| -> Vec<(u64, u64)> {
            edge_store
                .get_outgoing(node)
                .into_iter()
                .filter(|e| rel_types.is_empty() || rel_types.contains(&e.label().to_string()))
                .map(|e| (e.target(), e.id()))
                .collect()
        };

        let (results, _stats) = traverser.bfs_parallel(start_nodes, adjacency);

        results
            .into_iter()
            .filter(|r| r.depth >= config.min_depth)
            .map(|r| TraversalResult::new(r.end_node, r.path, r.depth))
            .collect()
    }

    // -------------------------------------------------------------------------
    // Embedding search on graph nodes
    // -------------------------------------------------------------------------

    /// Searches for similar graph nodes by embedding vector.
    ///
    /// Only available if `has_embeddings()` returns `true`.
    ///
    /// # Errors
    ///
    /// Returns `Error::VectorNotAllowed` if no embeddings are configured,
    /// or `Error::DimensionMismatch` if the query dimension is wrong.
    pub fn search_by_embedding(&self, query: &[f32], k: usize) -> Result<Vec<SearchResult>> {
        let config = self.config.read();
        let emb_dim = config
            .embedding_dimension
            .ok_or_else(|| Error::VectorNotAllowed(config.name.clone()))?;
        drop(config);

        if query.len() != emb_dim {
            return Err(Error::DimensionMismatch {
                expected: emb_dim,
                actual: query.len(),
            });
        }

        // Reason: we reuse the existing HNSW index (dimension == emb_dim when created
        // via create_graph_collection_with_embeddings). For graph-without-embeddings
        // the HNSW has dimension 0 and the guard above already rejected the call.
        let metric = self.config.read().metric;
        let ids = self.index.search(query, k);
        let ids = self.merge_delta(ids, query, k, metric);

        // Acquire each lock once: collect vector data, then collect payload data.
        // This avoids holding vector_storage while locking payload_storage per item.
        let vectors: Vec<(u64, f32, Option<Vec<f32>>)> = {
            let vector_storage = self.vector_storage.read();
            ids.into_iter()
                .map(|sr| {
                    let vec = vector_storage.retrieve(sr.id).ok().flatten();
                    (sr.id, sr.score, vec)
                })
                .collect()
        };
        let results = {
            let payload_storage = self.payload_storage.read();
            vectors
                .into_iter()
                .filter_map(|(id, score, vector)| {
                    let vector = vector?;
                    let payload = payload_storage.retrieve(id).ok().flatten();
                    Some(SearchResult::new(
                        Point {
                            id,
                            vector,
                            payload,
                            sparse_vectors: None,
                        },
                        score,
                    ))
                })
                .collect()
        };
        Ok(results)
    }
}
