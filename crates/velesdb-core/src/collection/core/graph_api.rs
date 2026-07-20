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
use crate::storage::{LogPayloadStorage, PayloadStorage, VectorStorage};

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
    /// Returns `Error::NodeNotFound` if `source` or `target` has no stored
    /// node payload, in both schema modes (issue #1470) — an edge to a node
    /// that was never created would otherwise be invisible to
    /// `all_node_ids()` and MATCH (issue #1442), since both derive their
    /// node set from the payload store, not the edge store. In strict mode,
    /// `Error::SchemaValidation` is still returned for actual schema-shape
    /// violations (undeclared node type, undeclared edge type, endpoint
    /// type mismatch) once both endpoints are confirmed to exist.
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
        // Position 1, hoisted before the payload guard below (position 3) so
        // the acquisition order is never 3 → 1 (see LOCK ORDERING in
        // collection/types.rs).
        let schema = self.non_schemaless_graph_schema();

        // Position 3, held from the referential-integrity check through the
        // end of the write below — see `validate_edge_endpoints_exist`'s
        // "Concurrency guarantee" doc for the race this closes. Reject
        // before any mutation so a violation leaves no partial write and
        // does not bump write_generation.
        let payload_guard = self.storage.payload_storage.read();
        Self::validate_edge_referential_integrity(&payload_guard, schema.as_ref(), &edge)?;

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
        // a multi-write entry. Position 3b — acquired here while STILL
        // holding `payload_guard` (position 3): see `GraphStore::edge_wal_lock`.
        let _wal_guard = self.graph.edge_wal_lock.lock();
        #[cfg(feature = "persistence")]
        crate::collection::graph::edge_wal::wal_append_add(
            &crate::collection::graph::edge_wal::wal_path_for_edges(&self.storage.path),
            &edge,
        )?;

        self.graph.edge_store.add_edge(edge)?;

        // Populate edge property indexes (EPIC-047).
        self.index_edge_properties(edge_id, &rel_type, &properties);

        // Bump write generation so any cached plan for this collection is
        // invalidated on the next query (CACHE-01).
        self.generations
            .write_generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        // `payload_guard` is dropped here (end of scope), AFTER the edge is
        // fully durable — see the concurrency guarantee this enforces.
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
    /// store is not mutated when the WAL append fails). Returns
    /// `Error::NodeNotFound` if any edge's `source` or `target` has no
    /// stored node payload (see [`Self::add_edge`]).
    pub fn add_edges_batch(&self, edges: Vec<GraphEdge>) -> Result<usize> {
        if edges.is_empty() {
            return Ok(0);
        }

        // Position 1, hoisted before the payload guard (see add_edge).
        let schema = self.non_schemaless_graph_schema();

        // Position 3, held for validating the ENTIRE batch through the end
        // of the apply below — with the guard held for the whole batch, "an
        // endpoint disappears mid-batch" is impossible by construction (see
        // `validate_edge_endpoints_exist`'s concurrency guarantee), so no
        // rollback is needed. A single violating edge still fails the whole
        // batch with no partial write and no orphaned WAL entry.
        let payload_guard = self.storage.payload_storage.read();
        for edge in &edges {
            Self::validate_edge_referential_integrity(&payload_guard, schema.as_ref(), edge)?;
        }

        // Unconditional WAL (see add_edge): writing edges implies wanting
        // them back after a restart, whatever the collection type. The WAL
        // lock spans append + apply (see add_edge). Position 3b, acquired
        // while STILL holding `payload_guard` (position 3).
        let _wal_guard = self.graph.edge_wal_lock.lock();
        #[cfg(feature = "persistence")]
        crate::collection::graph::edge_wal::wal_append_add_batch(
            &crate::collection::graph::edge_wal::wal_path_for_edges(&self.storage.path),
            &edges,
        )?;

        // Capture property metadata before the edges are moved into the store.
        let index_meta: Vec<(u64, String, _)> = edges
            .iter()
            .filter(|e| !e.properties().is_empty())
            .map(|e| (e.id(), e.label().to_string(), e.properties().clone()))
            .collect();

        let count = self.graph.edge_store.add_edges_batch(edges);

        for (edge_id, rel_type, properties) in &index_meta {
            self.index_edge_properties(*edge_id, rel_type, properties);
        }

        if count > 0 {
            self.generations
                .write_generation
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        Ok(count)
    }

    /// Replays the edge WAL on top of the loaded `edge_store` snapshot,
    /// re-running edge-property indexing for replayed ADD entries.
    ///
    /// Called from `Collection::open` AFTER `assemble`, so the snapshot is
    /// already loaded into `self.graph.edge_store`. A missing WAL file is a cheap
    /// no-op (legacy DBs predate this feature). The edge store is mutated
    /// in place via its `&self` methods; the closure indexes edge
    /// properties into `self.graph.edge_range_indexes`.
    ///
    /// # Errors
    ///
    /// Returns an error if the WAL file exists but cannot be read.
    #[cfg(feature = "persistence")]
    pub(crate) fn replay_edge_wal(&self) -> Result<u64> {
        use crate::collection::graph::edge_wal::{wal_path_for_edges, wal_replay, ReplayOp};
        let wal_path = wal_path_for_edges(&self.storage.path);
        let replayed = wal_replay(&wal_path, &self.graph.edge_store, |op| {
            if let ReplayOp::Add(edge) = op {
                if !edge.properties().is_empty() {
                    self.index_edge_properties(edge.id(), edge.label(), edge.properties());
                }
            }
        })?;
        if replayed > 0 {
            // Refresh the CSR read snapshot so traversals see replayed edges.
            self.graph.edge_store.build_read_snapshot();
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
        let schema = match self.storage.config.read().graph_schema.clone() {
            Some(s) if !s.is_schemaless() => s,
            _ => return Ok(()),
        };

        for label in extract_labels(payload) {
            schema.validate_node_type(&label)?;
        }
        Ok(())
    }

    /// Returns the collection's graph schema, but only when it declares a
    /// non-schemaless mode (`None` otherwise, including "no schema at all").
    ///
    /// Reads `config` — lock order position **1**. Callers MUST call this
    /// BEFORE acquiring `payload_storage` (position 3): resolving the
    /// schema after the payload guard would invert the documented
    /// acquisition order to 3 → 1 (see LOCK ORDERING in
    /// `collection/types.rs`).
    fn non_schemaless_graph_schema(&self) -> Option<GraphSchema> {
        match self.storage.config.read().graph_schema.clone() {
            Some(s) if !s.is_schemaless() => Some(s),
            _ => None,
        }
    }

    /// Enforces that both edge endpoints have a stored node payload
    /// (schemaless-mode existence check — see
    /// [`Self::validate_edge_referential_integrity`]). Unlike
    /// [`Self::endpoint_node_type`], this does not require a `_labels`
    /// field — plain (untyped) node payloads are enough.
    ///
    /// Takes the already-acquired payload-store guard by reference instead
    /// of re-locking — see the "Concurrency guarantee" section below for
    /// why a fresh lock acquisition here would be both redundant and unsafe.
    ///
    /// # Concurrency guarantee
    ///
    /// `add_edge`/`add_edges_batch` hold `payload_storage`'s READ guard from
    /// this check through the end of the edge write (WAL append + edge-store
    /// apply). `delete()`'s node-removal path acquires `payload_storage`'s
    /// WRITE guard before it can remove a payload, so it blocks until that
    /// read guard is released — by which point the edge is already durable,
    /// and `delete()`'s own edge cascade (`cascade_delete_node_edges`) will
    /// see and remove it. This closes the race the previous version of this
    /// check left open (drop the read guard, THEN write the edge — a
    /// concurrent delete could interleave in between and land a dangling
    /// edge). Passing the guard down also avoids a second, nested
    /// `payload_storage.read()` call from inside the outer guard's scope:
    /// parking_lot's `RwLock` is not safely re-entrant once a writer is
    /// queued, so a nested read acquired on the same thread while a writer
    /// waits would deadlock.
    ///
    /// # Errors
    ///
    /// Returns `Error::NodeNotFound` if `source` or `target` has no stored
    /// payload.
    fn validate_edge_endpoints_exist(payload: &LogPayloadStorage, edge: &GraphEdge) -> Result<()> {
        for node_id in [edge.source(), edge.target()] {
            if payload.retrieve(node_id)?.is_none() {
                return Err(Error::NodeNotFound(node_id));
            }
        }
        Ok(())
    }

    /// Enforces referential integrity for an edge write.
    ///
    /// In schemaless mode (the default), only endpoint existence is checked
    /// (`Error::NodeNotFound`) — an edge to a node that was never created
    /// would otherwise be accepted into the edge store while staying
    /// invisible to `all_node_ids()` and MATCH, which both resolve their
    /// node set from the payload store rather than the edge store (#1442).
    /// In strict mode it additionally verifies that the edge type /
    /// endpoint types satisfy the declared schema.
    ///
    /// `payload` is the caller's already-acquired `payload_storage` read
    /// guard (see [`Self::validate_edge_endpoints_exist`]'s concurrency
    /// guarantee) and `schema` is the caller's [`Self::non_schemaless_graph_schema`]
    /// result — both resolved by the caller so this function never acquires
    /// a lock itself.
    ///
    /// # Errors
    ///
    /// Returns `Error::NodeNotFound` when `source` or `target` has no stored
    /// payload, in both schema modes (issue #1470 — unified with the
    /// schemaless-only behavior described above; strict mode used to
    /// overload `Error::SchemaValidation` for this case, see the CHANGELOG
    /// entry for the next major version and issue #1442 for why it
    /// originally diverged). Returns `Error::SchemaValidation` in strict
    /// mode for an actual schema-shape violation once both endpoints are
    /// confirmed to exist: no `_labels` on an endpoint, or an edge-type /
    /// endpoint-type constraint violation.
    fn validate_edge_referential_integrity(
        payload: &LogPayloadStorage,
        schema: Option<&GraphSchema>,
        edge: &GraphEdge,
    ) -> Result<()> {
        let Some(schema) = schema else {
            return Self::validate_edge_endpoints_exist(payload, edge);
        };

        let from_type = Self::endpoint_node_type(payload, edge.source())?;
        let to_type = Self::endpoint_node_type(payload, edge.target())?;
        schema.validate_edge_type(edge.label(), &from_type, &to_type)
    }

    /// Resolves the node type (first `_labels` entry) for a graph node,
    /// requiring the node to exist and carry a label (strict-mode helper).
    ///
    /// Takes the already-acquired payload-store guard by reference — see
    /// [`Self::validate_edge_endpoints_exist`]'s concurrency guarantee.
    ///
    /// # Errors
    ///
    /// Returns `Error::NodeNotFound` if the node has no stored payload — a
    /// genuinely missing endpoint, unified with the schemaless-mode contract
    /// (issue #1470; previously `Error::SchemaValidation`, see the CHANGELOG
    /// entry for the next major version). Returns `Error::SchemaValidation`
    /// if the node exists but its payload declares no `_labels` — an actual
    /// schema-shape violation, not a missing endpoint.
    fn endpoint_node_type(payload: &LogPayloadStorage, node_id: u64) -> Result<String> {
        let stored = payload.retrieve(node_id)?;
        let Some(stored) = stored else {
            return Err(Error::NodeNotFound(node_id));
        };
        extract_labels(&stored)
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
        self.graph.edge_store.all_edges()
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
        self.graph.edge_store.get_edges_by_label(label)
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
        self.graph.edge_store.get_outgoing(node_id)
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
        self.graph.edge_store.get_incoming(node_id)
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
        let params = self.traversal_params(source, max_depth, filter, limit);
        let mut frontier = std::collections::VecDeque::new();
        frontier.push_back((source, 0u32));

        Ok(traverse_with_frontier(
            &params,
            bfs_pop,
            bfs_push,
            &mut frontier,
        ))
    }

    /// Builds the [`TraversalParams`] bundle shared by `traverse_bfs` and
    /// `traverse_dfs`; the two differ only in their frontier type and
    /// pop/push functions, which stay with each method.
    fn traversal_params<'a>(
        &'a self,
        source: u64,
        max_depth: u32,
        filter: &'a [&'a str],
        limit: usize,
    ) -> TraversalParams<'a> {
        TraversalParams {
            store: &self.graph.edge_store,
            filter,
            limit,
            max_depth,
            source,
        }
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
        let params = self.traversal_params(source, max_depth, filter, limit);
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
        let in_degree = self.graph.edge_store.incoming_degree(node_id);
        let out_degree = self.graph.edge_store.outgoing_degree(node_id);
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
        if !self.graph.edge_store.contains_edge(edge_id) {
            return false;
        }
        // WAL-before-apply (crash durability): log the remove intent before
        // mutating the store. Fail-closed: if the WAL append fails we do NOT
        // mutate the store and report `false` (no panic — matches the
        // no-unwrap policy and the bool return contract). Unconditional for
        // the same reason as add_edge; the WAL lock spans append + apply.
        let _wal_guard = self.graph.edge_wal_lock.lock();
        #[cfg(feature = "persistence")]
        if let Err(e) = crate::collection::graph::edge_wal::wal_append_remove(
            &crate::collection::graph::edge_wal::wal_path_for_edges(&self.storage.path),
            edge_id,
        ) {
            tracing::error!("Edge WAL append remove failed for edge {edge_id}: {e}");
            return false;
        }

        // Atomic check-and-remove — no TOCTOU race.
        let removed = self.graph.edge_store.remove_edge(edge_id);
        if removed {
            self.generations
                .write_generation
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        removed
    }

    /// Returns the total number of edges in the graph.
    #[must_use]
    pub fn edge_count(&self) -> usize {
        self.graph.edge_store.len()
    }

    /// Returns the highest edge id in the graph, if any (no edge cloning).
    pub(crate) fn max_edge_id(&self) -> Option<u64> {
        self.graph.edge_store.max_edge_id()
    }

    /// Returns `true` when an edge with `edge_id` exists.
    pub(crate) fn edge_exists(&self, edge_id: u64) -> bool {
        self.graph.edge_store.contains_edge(edge_id)
    }

    /// Rebuilds edge property indexes from edges already in the store.
    ///
    /// Called on open AFTER the edge snapshot is loaded and BEFORE the edge
    /// WAL replays (replay indexes its own ADDs), so snapshot-loaded edge
    /// properties become queryable again — previously they were only ever
    /// indexed at write time and silently lost once the WAL was truncated
    /// into the snapshot.
    pub(crate) fn reindex_edge_properties_from_store(&self) {
        for edge in self.graph.edge_store.all_edges() {
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
        self.storage.config.read().graph_schema.clone()
    }

    /// Returns `true` if this collection was created as a graph collection.
    #[must_use]
    #[allow(dead_code)] // Reason: Called via GraphCollection inner delegation + tests
    pub fn is_graph(&self) -> bool {
        self.storage.config.read().graph_schema.is_some()
    }

    /// Returns `true` if this graph collection stores node embeddings.
    #[must_use]
    pub fn has_embeddings(&self) -> bool {
        self.storage.config.read().embedding_dimension.is_some()
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

        let mut storage = self.storage.payload_storage.write();

        // Remove old labels and property indexes if this is an update.
        let mut label_idx = self.graph.label_index.write();
        if let Ok(Some(old_payload)) = storage.retrieve(node_id) {
            label_idx.remove_from_payload(node_id, &old_payload);
            // Release label_index before touching graph property indexes
            // (both are at lock order 7, no ordering between them).
            drop(label_idx);
            self.deindex_node_properties(node_id, &old_payload);
            label_idx = self.graph.label_index.write();
        }

        storage.store(node_id, payload)?;
        label_idx.index_from_payload(node_id, payload);
        drop(label_idx);
        drop(storage);

        // Populate graph property indexes (EPIC-047).
        self.index_node_properties(node_id, payload);

        // Node payload writes bypass the upsert mirror hooks — drop the
        // payload mirror so it can never serve stale columnar data.
        self.storage.payload_mirror.invalidate();

        // Bump write generation so any cached plan for this collection is
        // invalidated on the next query (CACHE-01).
        self.generations
            .write_generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    /// Retrieves the JSON payload for a graph node.
    ///
    /// # Errors
    ///
    /// Returns an error if retrieval fails.
    pub fn get_node_payload(&self, node_id: u64) -> Result<Option<serde_json::Value>> {
        Ok(self.storage.payload_storage.read().retrieve(node_id)?)
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
        self.graph
            .edge_store
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
        if self.graph.edge_store.csr_is_authoritative() || self.graph.edge_store.csr_rebuild_due() {
            // Prefer the lock-free CSR snapshot path when available (Issue #491).
            let snapshot = self.graph.edge_store.get_csr_snapshot();
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
        concurrent_bfs_stream(&self.graph.edge_store, source_id, streaming)
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
        self.graph
            .edge_store
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
                    &self.graph.edge_store,
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
        let edge_store = &self.graph.edge_store;

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
        let config = self.storage.config.read();
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
        let metric = self.storage.config.read().metric;
        let ids = self.storage.index.search(query, k);
        let ids = self.merge_delta(ids, query, k, metric);

        // Acquire each lock once: collect vector data, then collect payload data.
        // This avoids holding vector_storage while locking payload_storage per item.
        let vectors: Vec<(u64, f32, Option<Vec<f32>>)> = {
            let vector_storage = self.storage.vector_storage.read();
            ids.into_iter()
                .map(|sr| {
                    let vec = vector_storage.retrieve(sr.id).ok().flatten();
                    (sr.id, sr.score, vec)
                })
                .collect()
        };
        let results = {
            let payload_storage = self.storage.payload_storage.read();
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
